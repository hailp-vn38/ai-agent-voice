use super::super::*;
use crate::services::provider_runtime::{RuntimeMetrics, RuntimePhase};
use std::time::Instant;
use tracing::Instrument;

mod admission;
mod writer;

use admission::Admission;

pub(crate) async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WebsocketQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let connect_started = Instant::now();
    let metrics = state
        .provider_runtime_manager
        .as_ref()
        .map(|manager| manager.metrics().clone());
    let _pre_upgrade_timer = metrics
        .as_ref()
        .map(|metrics| metrics.timer(RuntimePhase::WsPreUpgrade));
    let config = state.config.clone();
    let bearer_auth_enabled = !config.auth.token.is_empty();
    // Registered before anything else happens, so the drain can observe and close a connection
    // even if it never reaches the handshake.  The handle is moved into the socket task, which is
    // what keeps this connection counted for exactly as long as it exists.
    let drain = state.register_session();
    let admission = match admission::admit(&state, &headers, &query, connect_started, metrics).await
    {
        Ok(admission) => admission,
        Err(rejection) => {
            rejection.log(bearer_auth_enabled);
            return rejection.into_response();
        }
    };
    // The transport cap bounds the whole message, not only each frame: continuation frames that
    // each fit `max_frame_size` would otherwise be reassembled up to the library default before
    // the application size check ever runs.
    let cap = config.websocket.max_frame_bytes.saturating_add(1024);
    let upgrade = upgrade.max_frame_size(cap).max_message_size(cap);
    let runtimes = match admission {
        Admission::Enrollment(connection) => {
            log_ws_auth(
                bearer_auth_enabled,
                bearer_auth_result(bearer_auth_enabled),
                "pending_enrollment",
                "accepted",
                "pending_enrollment",
            );
            return upgrade
                .on_upgrade(move |socket| {
                    super::super::enrollment::run(socket, state, connection, drain)
                })
                .into_response();
        }
        Admission::Voice(runtimes) => *runtimes,
    };
    let stopping = state.lifecycle.stopping().clone();
    let session_span = tracing::info_span!(
        "voice_session",
        session_id = %runtimes.session_id,
        agent_key = %runtimes.profile.agent_key,
    );
    session_span.in_scope(|| {
        info!(
            event = "ws_upgrade_accepted",
            agent_key = %runtimes.profile.agent_key,
            template = ?runtimes.profile.source,
            "WebSocket upgrade accepted"
        );
        log_ws_auth(
            bearer_auth_enabled,
            bearer_auth_result(bearer_auth_enabled),
            "admitted",
            "accepted",
            "ok",
        );
    });
    upgrade
        .on_upgrade(move |socket| {
            handle_socket(
                socket,
                config,
                runtimes,
                SessionControl {
                    stopping,
                    drain_close: drain.close_signal(),
                    registration: drain,
                },
            )
            .instrument(session_span)
        })
        .into_response()
}

fn bearer_auth_result(enabled: bool) -> &'static str {
    if enabled { "accepted" } else { "not_required" }
}

/// Connection telemetry deliberately has no raw headers, token, Device ID or Client ID.
fn log_ws_auth(
    bearer_auth_enabled: bool,
    bearer_auth_result: &'static str,
    device_admission_result: &'static str,
    result: &'static str,
    reason_code: &'static str,
) {
    if result == "accepted" {
        info!(
            event = "ws_auth",
            bearer_auth_enabled,
            bearer_auth_result,
            device_admission_result,
            result,
            reason_code,
            "WebSocket authentication decision"
        );
    } else {
        warn!(
            event = "ws_auth",
            bearer_auth_enabled,
            bearer_auth_result,
            device_admission_result,
            result,
            reason_code,
            "WebSocket authentication decision"
        );
    }
}

#[derive(Deserialize)]
pub(crate) struct WebsocketQuery {
    #[serde(rename = "protocol-version")]
    protocol_version: Option<String>,
    #[serde(rename = "device-id")]
    device_id: Option<String>,
    #[serde(rename = "client-id")]
    client_id: Option<String>,
    authorization: Option<String>,
    ticket: Option<String>,
}

struct SocketRuntimes {
    job_submit: Option<crate::services::task_flow::VoiceTaskSubmit>,
    active_device_sessions: Arc<crate::services::active_device_sessions::ActiveDeviceSessions>,
    database: Option<Arc<crate::database::Database>>,
    system_mcp: Option<crate::tools::system_mcp::SessionSystemMcp>,
    reminder_tools: Option<crate::services::notifications::SessionReminderTools>,
    assistant_connection: Option<super::super::assistant::Connection>,
    connect_started: Instant,
    metrics: Option<Arc<RuntimeMetrics>>,
    asr: Arc<AsrWorkerRuntime>,
    vad: Arc<VadWorkerRuntime>,
    llm: Arc<LlmRuntime>,
    tts: Arc<TtsWorkerRuntime>,
    vad_segmenter: VadSegmenterConfig,
    vad_pre_roll_samples: u64,
    active_turn_limiter: Arc<ActiveTurnLimiter>,
    pilot_admission: crate::session::pilot::PilotAdmission,
    /// Test-only; see [`WriterOutcomeProbe`](crate::session::WriterOutcomeProbe).
    writer_probe: Option<Arc<dyn crate::session::WriterOutcomeProbe>>,
    profile: crate::session::EffectiveSessionProfile,
    /// This connection's own session identity, resolved before the upgrade so the archive and the
    /// Voice Session cannot disagree about which connection a transcript belongs to.
    session_id: String,
    /// The optional Persistent Transcript binding, resolved with the profile above.  `None` means
    /// capture is off or this session has no database identity, and the actor then archives
    /// nothing at all.
    transcript: Option<crate::database::history::TranscriptCapture>,
    /// Ticket 10 Observe, resolved at admission. `None` keeps the actor free of speaker inference
    /// and utterance PCM retention.
    speaker_observe: Option<Arc<SpeakerObserve>>,
    /// The application admission gate. The actor only asks; the application closes it.
    admission_gate: Arc<crate::lifecycle::AdmissionGate>,
}

/// The application-owned handles that keep one connection countable and closable.
///
/// Holding `registration` is what keeps this connection in the drain registry: it is registered
/// before the upgrade and released when the socket task ends, so the registry's count is a fact
/// about live connections rather than a guess.  The two signals are separate decisions — `stopping`
/// is the process finishing, `drain_close` is the deadline reaching *this* connection — and a
/// controlled close is a request the session performs itself, not a force.
struct SessionControl {
    stopping: CancellationToken,
    drain_close: CancellationToken,
    registration: crate::lifecycle::DrainRegistration,
}

/// Why the application ended a Voice Session from outside its own protocol flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseReason {
    /// The process is finishing outright.
    Stopping,
    /// The shutdown drain deadline reached this session: Controlled Close.
    DrainDeadline,
    /// A newer connection of the same Device, or a revoked Web Assistant credential, replaces it.
    Replaced,
    /// A security mutation revoked a snapshot the session pinned (System MCP grants, External MCP
    /// tools, speaker policy).
    Revoked,
}

/// Every external signal that can end one Voice Session, observed in one place. Tokens a session
/// does not use (no Web Assistant, no System MCP, ...) are fresh ones that never fire.
struct SessionCloseSet {
    stopping: CancellationToken,
    drain_deadline: CancellationToken,
    replaced: CancellationToken,
    assistant_revoked: CancellationToken,
    system_mcp_revoked: CancellationToken,
    external_tools_revoked: CancellationToken,
    speaker_revoked: CancellationToken,
}

impl SessionCloseSet {
    /// Completes with the first signal that fired; shutdown signals take precedence.
    async fn cancelled(&self) -> CloseReason {
        tokio::select! {
            biased;
            _ = self.stopping.cancelled() => CloseReason::Stopping,
            _ = self.drain_deadline.cancelled() => CloseReason::DrainDeadline,
            _ = self.replaced.cancelled() => CloseReason::Replaced,
            _ = self.assistant_revoked.cancelled() => CloseReason::Replaced,
            _ = self.system_mcp_revoked.cancelled() => CloseReason::Revoked,
            _ = self.external_tools_revoked.cancelled() => CloseReason::Revoked,
            _ = self.speaker_revoked.cancelled() => CloseReason::Revoked,
        }
    }

    /// The same signals for SessionActor, which stops new work as soon as any fires.
    fn tokens(&self) -> [CancellationToken; 7] {
        [
            self.replaced.clone(),
            self.external_tools_revoked.clone(),
            self.system_mcp_revoked.clone(),
            self.speaker_revoked.clone(),
            self.assistant_revoked.clone(),
            self.stopping.clone(),
            self.drain_deadline.clone(),
        ]
    }
}

async fn handle_socket(
    socket: WebSocket,
    config: Arc<AppConfig>,
    runtimes: SocketRuntimes,
    control: SessionControl,
) {
    let active_device_sessions = runtimes.active_device_sessions;
    let database = runtimes.database;
    let device_db_id = runtimes.profile.device_db_id;
    let physical_connection = runtimes.assistant_connection.is_none();
    let assistant_connection = runtimes.assistant_connection;
    let socket_started = Instant::now();
    let connect_started = runtimes.connect_started;
    let metrics = runtimes.metrics;
    // The profile stays owned by the connection lifetime. SessionActor receives only the
    // materialized snapshot, its admission-time switch catalog and the External MCP tools it may
    // call, never a Database/pool or a live Device/Agent row.
    let admitted = runtimes.profile.into_admitted_profile();
    let closes = SessionCloseSet {
        stopping: control.stopping.clone(),
        drain_deadline: control.drain_close.clone(),
        replaced: CancellationToken::new(),
        assistant_revoked: assistant_connection
            .as_ref()
            .map(|c| (*c.close).clone())
            .unwrap_or_default(),
        system_mcp_revoked: runtimes
            .system_mcp
            .as_ref()
            .map(|system| (*system.context.revoked).clone())
            .unwrap_or_default(),
        external_tools_revoked: admitted
            .external_mcp
            .guard
            .as_ref()
            .map(|g| (*g.close).clone())
            .unwrap_or_default(),
        // The Speaker/grant/policy/template snapshot this session pinned. Cancelled by the
        // registry on any mutation that revokes it, exactly like the external tool guard.
        speaker_revoked: runtimes
            .speaker_observe
            .as_ref()
            .map(|observe| (*observe.security_token()).clone())
            .unwrap_or_default(),
    };
    let writer_probe = runtimes.writer_probe;
    let session_id = runtimes.session_id;
    // `control` itself is held to the end of this function: its registration is the connection's
    // place in the drain registry, and dropping it early would un-count a live session.
    let (mut sender, mut receiver) = socket.split();
    let write_timeout = Duration::from_millis(config.websocket.write_timeout_ms);
    let hello_wait_timer = metrics
        .as_ref()
        .map(|metrics| metrics.timer(RuntimePhase::WsHelloWait));
    let first = tokio::select! {
        reason = closes.cancelled() => {
            // No Voice Session exists yet, so there is nothing to drain: close directly.
            match reason {
                CloseReason::Stopping => {}
                CloseReason::DrainDeadline => close_direct(&mut sender, write_timeout, 1001).await,
                CloseReason::Replaced | CloseReason::Revoked => {
                    close_direct(&mut sender, write_timeout, 1008).await;
                }
            }
            return;
        },
        first = timeout(Duration::from_millis(config.server.hello_timeout_ms), next_application_frame(&mut receiver)) => first,
    };
    drop(hello_wait_timer);
    let hello_init_timer = metrics
        .as_ref()
        .map(|metrics| metrics.timer(RuntimePhase::WsHelloInit));
    let hello = match first {
        Ok(Some(Ok(message))) => message,
        _ => {
            close_direct(&mut sender, write_timeout, 1002).await;
            return;
        }
    };
    let text = match checked_text(hello, config.websocket.max_frame_bytes) {
        Ok(text) => text,
        Err(code) => {
            close_direct(&mut sender, write_timeout, code).await;
            return;
        }
    };
    let hello = match parse_client_message(&text) {
        Ok(ClientMessage::Hello(hello)) if hello.validate_v1().is_ok() => hello,
        _ => {
            close_direct(&mut sender, write_timeout, 1002).await;
            return;
        }
    };
    info!(
        event = "client_hello_accepted",
        transport = %hello.transport,
        "ClientHello accepted"
    );

    let (control_tx, control_rx) = mpsc::channel(config.limits.outbound_control_queue);
    let (urgent_tx, urgent_rx) = mpsc::channel(config.limits.urgent_control_queue);
    let (audio_tx, audio_rx) = mpsc::channel(config.limits.outbound_audio_queue);
    let (writer_event_tx, writer_event_rx) = mpsc::channel(config.limits.session_event_queue);
    let generation_gate = Arc::new(crate::session::GenerationGate::new());
    let (ingress_tx, ingress_rx) = mpsc::channel(config.limits.session_event_queue);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    // The writer reads this to learn the application stopped it. It must outlive SessionActor's own
    // sender — an actor that ends for an ordinary reason would otherwise stop the writer mid-drain —
    // and is released explicitly once the actor has finished and every lane has been closed.
    let writer_shutdown_tx = shutdown_tx.clone();
    let actor = match SessionActor::new_with_runtimes_and_limiter_and_outbound(
        session_id.clone(),
        control_tx.clone(),
        urgent_tx.clone(),
        audio_tx,
        Arc::clone(&generation_gate),
        shutdown_tx,
        config.max_capture_frames(),
        config.llm.max_history_messages,
        SessionRuntimes {
            job_submit: runtimes.job_submit,
            asr: runtimes.asr,
            vad: runtimes.vad,
            llm: runtimes.llm,
            tts: runtimes.tts,
            active_turn_limiter: runtimes.active_turn_limiter,
            vad_segmenter_config: runtimes.vad_segmenter,
            pre_roll_samples: runtimes.vad_pre_roll_samples,
        },
    ) {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "failed to initialize session audio runtime");
            close_direct(&mut sender, write_timeout, 1011).await;
            return;
        }
    };
    if actor.job_submit_available_without_tool_support() {
        tracing::warn!(
            event = "voice_job_prerequisite_unavailable",
            code = "llm_tool_calls_unavailable",
            "Admitted Agent has no tool-capable Voice LLM"
        );
    }
    let (pipeline_writer_done, pipeline_writer_terminal) = tokio::sync::watch::channel(false);
    let actor = actor
        .with_direct_session_closes(closes.tokens())
        .with_direct_metadata(hello.features.direct_tts || hello.features.reminders)
        .with_turn_status(assistant_connection.is_some() && hello.features.turn_status)
        .with_tool_status(tool_status_enabled(
            assistant_connection.is_some(),
            &hello.features,
        ));
    let actor = actor.with_pipeline_writer_terminal(pipeline_writer_terminal);
    let actor =
        actor.with_pilot_admission(runtimes.pilot_admission, hello.features.pipeline_status);
    let pipeline_request = actor.pipeline_request_epoch();
    let actor = actor.with_switch_ingress_limits(
        config.limits.session_event_queue,
        config.websocket.max_frame_bytes,
    );
    let actor = match actor.with_delivery_runtime_config(config.speech_output.clone()) {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "failed to initialize session speech output");
            close_direct(&mut sender, write_timeout, 1011).await;
            return;
        }
    };
    let actor = match actor.with_effective_profile(
        admitted.active,
        admitted.switch_catalog,
        admitted.external_mcp,
        config.llm.max_tool_result_chars,
    ) {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "session profile prompt exceeds the system prompt bound");
            close_direct(&mut sender, write_timeout, 1011).await;
            return;
        }
    };
    // The Device's reminder protocol is the notification service's business; this task only
    // carries its events in and its replies back through the actor's mailbox.
    let (reminder_event_tx, reminder_worker) = match &database {
        Some(database) if physical_connection && hello.features.reminders => {
            let (tx, mut rx) = mpsc::channel::<crate::protocol::ReminderEvent>(8);
            let coordinator = crate::services::notifications::ReminderCoordinator::from_database(
                (**database).clone(),
                Arc::clone(&active_device_sessions),
            );
            let sid = session_id.clone();
            let ingress = ingress_tx.clone();
            let worker = tokio::spawn(
                async move {
                    while let Some(event) = rx.recv().await {
                        if let Some(text) = coordinator
                            .on_device_reminder_event(device_db_id, &sid, event)
                            .await
                            && ingress
                                .try_send(SessionEvent::ReminderControl { text })
                                .is_err()
                        {
                            warn!(
                                subsystem = "notification",
                                event = "reminder_control_dropped",
                                "reminder reply dropped because the session mailbox is full or closed"
                            );
                        }
                    }
                }
                .instrument(tracing::Span::current()),
            );
            (Some(tx), Some(worker))
        }
        _ => (None, None),
    };
    // Ticket 10: install Observe only when the Agent policy enables it. Best-effort and
    // admission-time: a resolution/DB failure degrades to `off`, never fails the session.
    let mut actor = actor
        .with_system_mcp(runtimes.system_mcp)
        .with_reminder_tools(runtimes.reminder_tools)
        .with_transcript(runtimes.transcript)
        .with_speaker_observe(
            runtimes.speaker_observe.clone(),
            hello.features.speaker_status,
        )
        .with_admission_gate(runtimes.admission_gate)
        .with_reminder_turn_wakeup(
            (physical_connection && hello.features.reminders)
                .then(|| Arc::clone(active_device_sessions.scheduler_wakeup())),
        )
        .with_reminder_events(reminder_event_tx.clone())
        .with_writer_outcome_probe_opt(writer_probe.clone())
        .with_client_capabilities(
            hello.features.aec,
            crate::session::BargeInPolicy {
                enabled: config.barge_in.enabled,
                trust_client_aec_feature: config.barge_in.trust_client_aec_feature,
            },
        )
        .with_device_mcp(
            hello.features.mcp && assistant_connection.is_none(),
            &config.mcp,
            vision_capability(&config),
        )
        .with_tool_round_limits(crate::tools::round::ToolRoundLimits::from_config(
            &config.llm.tools,
        ))
        .with_writer_events(writer_event_rx)
        .with_job_mailbox(ingress_tx.clone());
    let server_hello = serde_json::to_string(&ServerHello::v1(actor.session_id()))
        .expect("ServerHello is serializable");
    if actor.send_control(server_hello).is_err() {
        close_direct(&mut sender, write_timeout, 1011).await;
        return;
    }
    drop(hello_init_timer);
    let hello_queued = Instant::now();
    actor.start_mcp_discovery();
    let writer = tokio::spawn(
        writer::WebSocketWriter {
            sender,
            write_timeout,
            ping_period: Duration::from_millis(config.websocket.ping_interval_ms),
            generation_gate,
            pipeline_request,
            writer_events: writer_event_tx,
            probe: writer_probe,
            hello: writer::HelloTelemetry {
                metrics,
                connect_started,
                socket_started,
                hello_queued,
            },
            done: pipeline_writer_done,
        }
        .run(writer::WriterLanes {
            urgent: urgent_rx,
            control: control_rx,
            audio: audio_rx,
            shutdown: shutdown_rx,
        })
        .instrument(tracing::Span::current()),
    );

    if physical_connection {
        active_device_sessions.register(
            device_db_id,
            session_id.clone(),
            hello.features.clone(),
            ingress_tx.clone(),
            closes.replaced.clone(),
        );
    }
    let session = tokio::spawn(actor.run(ingress_rx).instrument(tracing::Span::current()));

    let idle_timeout = Duration::from_millis(config.websocket.idle_timeout_ms);
    let mut last_inbound = tokio::time::Instant::now();
    loop {
        let idle_deadline = last_inbound + idle_timeout;
        tokio::select! {
            reason = closes.cancelled() => {
                match reason {
                    // A newer connection of this Device, or the Web Assistant credential, took
                    // this session's place: the actor ends it as a security invalidation.
                    CloseReason::Replaced => {
                        let _ = ingress_tx.send(SessionEvent::SecurityInvalidated).await;
                    }
                    // A security mutation revoked a snapshot this session pinned: no new
                    // dispatch, close 1008, terminal event. If the mailbox is gone the send fails
                    // and this loop simply ends, which is the shutdown escape path.
                    CloseReason::Revoked => {
                        let _ = urgent_tx.send(OutboundMessage::Close(1008)).await;
                        let _ = ingress_tx.try_send(SessionEvent::SecurityInvalidated);
                    }
                    // The drain deadline reached this session. It closes cleanly: the actor is
                    // told to stop, sends its terminal close frame and lets the writer drain, and
                    // this loop only exits once the connection itself is done. No task is aborted.
                    CloseReason::DrainDeadline => {
                        info!(
                            event = "ws_controlled_close_issued",
                            "controlled close issued at the shutdown grace deadline"
                        );
                        let _ = ingress_tx.send(SessionEvent::Shutdown).await;
                    }
                    // The process is finishing outright. There is no deadline left to wait for, so
                    // this session ends now rather than holding the process open.
                    CloseReason::Stopping => {
                        let _ = ingress_tx.send(SessionEvent::Shutdown).await;
                    }
                }
                break;
            }
            // Nothing at all — not even a Pong to the writer's Pings — has arrived for the idle
            // window: the peer is gone without a close (power loss, NAT drop). The session ends
            // now instead of when the operating system eventually notices.
            _ = tokio::time::sleep_until(idle_deadline), if !idle_timeout.is_zero() => {
                info!(event = "ws_idle_timeout", idle_timeout_ms = config.websocket.idle_timeout_ms, "closing a voice session with no inbound frames");
                let _ = urgent_tx.try_send(OutboundMessage::Close(1001));
                break;
            }
            next = receiver.next() => {
                last_inbound = tokio::time::Instant::now();
                let Some(next) = next else { break; };
                match next {
            Ok(Message::Text(text)) => {
                if text.len() > config.websocket.max_frame_bytes {
                    let _ = urgent_tx.send(OutboundMessage::Close(1009)).await;
                    break;
                }
                match parse_client_message(&text) {
                    Ok(message) => {
                        if let ClientMessage::Reminder { session_id: inbound, event } = message {
                            if hello.features.reminders
                                && inbound == session_id
                                && let Some(tx) = &reminder_event_tx
                                && tx.try_send(event).is_err()
                            {
                                warn!(
                                    subsystem = "notification",
                                    event = "reminder_event_dropped",
                                    "Device reminder event dropped because the reminder queue is full"
                                );
                            }
                            continue;
                        }
                        if ingress_tx
                            .try_send(SessionEvent::ClientMessage(message))
                            .is_err()
                        {
                            let _ = urgent_tx.send(OutboundMessage::Close(1013)).await;
                            break;
                        }
                    }
                    Err(error) => {
                        debug!(%error, "ignored invalid application message after handshake")
                    }
                }
            }
            Ok(Message::Binary(payload)) => {
                if payload.len() > config.websocket.max_frame_bytes {
                    let _ = urgent_tx.send(OutboundMessage::Close(1009)).await;
                    break;
                }
                if ingress_tx
                    .try_send(SessionEvent::ClientAudio(payload.to_vec()))
                    .is_err()
                {
                    debug!("dropped binary because bounded ingress is full or session is closed");
                }
            }
            Ok(Message::Close(frame)) => {
                info!(
                    event = "ws_peer_closed",
                    ?frame,
                    "websocket peer closed connection"
                );
                break;
            }
            Err(error) => {
                warn!(
                    event = "ws_receive_failed",
                    outcome = "failure",
                    %error,
                    "websocket receive failed"
                );
                break;
            }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                }
            }
        }
    }
    if physical_connection {
        active_device_sessions.unregister(device_db_id, &session_id);
    }
    drop(ingress_tx);
    let _ = session.await;
    drop(reminder_event_tx);
    if let Some(worker) = reminder_worker {
        let _ = worker.await;
    }
    if physical_connection
        && hello.features.reminders
        && let Some(database) = &database
    {
        let coordinator = crate::services::notifications::ReminderCoordinator::from_database(
            (**database).clone(),
            Arc::clone(&active_device_sessions),
        );
        let _ = coordinator
            .on_session_closed(device_db_id, &session_id)
            .await;
    }
    drop(control_tx);
    drop(urgent_tx);
    // The actor is gone and no lane can produce another message, so the watch channel's last sender
    // goes too. That is what lets the writer's stop branch complete; awaiting it before this point
    // is the deadlock this ordering exists to avoid.
    drop(writer_shutdown_tx);
    let _ = writer.await;
    // The session is finished: releasing its drain registration here is what lets a shutdown that
    // is waiting observe that this connection is gone.
    drop(control.registration);
    info!(
        event = "ws_session_disconnected",
        duration_ms = socket_started.elapsed().as_millis() as u64,
        "voice session disconnected"
    );
}

fn vision_capability(config: &AppConfig) -> Option<crate::tools::device_mcp::VisionCapability> {
    if config.vision.enabled && config.vision.advertise_via_mcp {
        Some(crate::tools::device_mcp::VisionCapability {
            url: config.vision.public_url.as_ref()?.to_string(),
            token: config.auth.token.clone(),
        })
    } else {
        None
    }
}

fn checked_text(message: Message, max: usize) -> Result<String, u16> {
    match message {
        Message::Text(text) if text.len() <= max => Ok(text.to_string()),
        Message::Text(_) => Err(1009),
        Message::Binary(_) => Err(1002),
        _ => Err(1002),
    }
}

/// Web Assistant must observe server-owned tool execution even if its ClientHello
/// predates the optional tool_status feature. This is independent of Device MCP.
fn tool_status_enabled(is_web_assistant: bool, features: &crate::protocol::ClientFeatures) -> bool {
    is_web_assistant || features.tool_status
}

async fn send_outbound(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    write_timeout: Duration,
    message: OutboundMessage,
    generation_gate: &crate::session::GenerationGate,
    pipeline_request: &std::sync::atomic::AtomicU64,
) -> bool {
    if let OutboundMessage::PipelineStatus { request, .. } = &message
        && *request != pipeline_request.load(std::sync::atomic::Ordering::Acquire)
    {
        return false;
    }
    let generation = match &message {
        OutboundMessage::TurnText { generation, .. }
        | OutboundMessage::Binary { generation, .. } => Some(*generation),
        OutboundMessage::Text(_)
        | OutboundMessage::PipelineStatus { .. }
        | OutboundMessage::SpeakerStatus { .. }
        | OutboundMessage::BeginTurn { .. }
        | OutboundMessage::FinishTurn { .. }
        | OutboundMessage::AbortTurn { .. }
        | OutboundMessage::Close(_)
        | OutboundMessage::CloseWithReason { .. } => None,
    };
    if let Some(generation) = generation.filter(|generation| !generation_gate.admits(*generation)) {
        tracing::debug!(
            event = "outbound_generation_rejected",
            generation,
            "Dropped stale turn-scoped outbound message"
        );
        return false;
    }
    let closes = matches!(message, OutboundMessage::Close(_));
    let result = match message {
        OutboundMessage::Text(text)
        | OutboundMessage::TurnText { text, .. }
        | OutboundMessage::SpeakerStatus { text }
        | OutboundMessage::PipelineStatus { text, .. } => {
            // Only `llm` messages are logged; skip parsing the (frequent) rest of the text stream.
            let llm_message = text
                .contains("\"llm\"")
                .then(|| serde_json::from_str::<serde_json::Value>(&text).ok())
                .flatten()
                .filter(|value| value["type"] == "llm");
            let result = send_frame(sender, write_timeout, Message::Text(text.into())).await;
            if result.is_ok()
                && let Some(message) = llm_message
                && let Some(llm_text) = message["text"].as_str()
            {
                info!(
                    event = "ws_llm_text_sent",
                    message_type = "llm",
                    chars = llm_text.chars().count(),
                    "WebSocket text sent to client"
                );
            }
            result
        }
        OutboundMessage::Binary { packet, .. } => {
            send_frame(sender, write_timeout, Message::Binary(packet.into())).await
        }
        // Semantic playback commands must be consumed by the writer state machine above.
        OutboundMessage::BeginTurn { .. }
        | OutboundMessage::FinishTurn { .. }
        | OutboundMessage::AbortTurn { .. } => return false,
        OutboundMessage::Close(code) => {
            send_frame(
                sender,
                write_timeout,
                Message::Close(Some(CloseFrame {
                    code,
                    reason: "".into(),
                })),
            )
            .await
        }
        OutboundMessage::CloseWithReason { code, reason } => {
            send_frame(
                sender,
                write_timeout,
                Message::Close(Some(CloseFrame {
                    code,
                    reason: reason.into(),
                })),
            )
            .await
        }
    };
    if let Err(ref error) = result {
        warn!(
            event = "ws_writer_failed",
            outcome = "failure",
            %error,
            "websocket writer failed"
        );
    }
    closes || result.is_err()
}

async fn close_direct(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    write_timeout: Duration,
    code: u16,
) {
    let _ = send_frame(
        sender,
        write_timeout,
        Message::Close(Some(CloseFrame {
            code,
            reason: "".into(),
        })),
    )
    .await;
}

/// Why one WebSocket send failed.
enum WriteFailure {
    Socket(axum::Error),
    /// The peer stopped reading and the send did not finish inside the write timeout.
    TimedOut,
}

impl std::fmt::Display for WriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Socket(error) => error.fmt(f),
            Self::TimedOut => f.write_str("write timed out"),
        }
    }
}

/// Every WebSocket send goes through here so no send can wait on a peer that stopped reading for
/// longer than the write timeout; a writer that cannot finish is a session the drain never sees
/// end.
async fn send_frame(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    write_timeout: Duration,
    message: Message,
) -> Result<(), WriteFailure> {
    match timeout(write_timeout, sender.send(message)).await {
        Ok(result) => result.map_err(WriteFailure::Socket),
        Err(_) => Err(WriteFailure::TimedOut),
    }
}

/// The next frame of the handshake phase, skipping transport control: a keepalive Ping or Pong
/// before ClientHello is not the client's first application message.
async fn next_application_frame(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<Result<Message, axum::Error>> {
    loop {
        match receiver.next().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => return other,
        }
    }
}

#[cfg(test)]
#[path = "websocket/tool_status_tests.rs"]
mod tool_status_negotiation_tests;
