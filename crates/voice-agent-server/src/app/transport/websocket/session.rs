use super::*;

pub(super) async fn handle_socket(
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
    let closes = SessionCloseSet::new(
        control.stopping.clone(),
        control.drain_close.clone(),
        assistant_connection.as_ref().map(|c| (*c.close).clone()),
        [
            runtimes
                .system_mcp
                .as_ref()
                .map(|system| (*system.context.revoked).clone()),
            admitted
                .external_mcp
                .guard
                .as_ref()
                .map(|g| (*g.close).clone()),
            // The Speaker/grant/policy/template snapshot this session pinned. Cancelled by the
            // registry on any mutation that revokes it, exactly like the external tool guard.
            runtimes
                .speaker_observe
                .as_ref()
                .map(|observe| (*observe.security_token()).clone()),
        ],
    );
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
    // The reader also watches the writer: once nothing can reach the peer, there is no session left
    // to read for.
    let mut writer_stopped = pipeline_writer_terminal.clone();
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
    let activity = Arc::new(TransportActivity::new());
    let writer = tokio::spawn(
        writer::WebSocketWriter {
            sender,
            activity: Arc::clone(&activity),
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
            closes.replacement(),
        );
    }
    let session = tokio::spawn(actor.run(ingress_rx).instrument(tracing::Span::current()));

    let idle_timeout = Duration::from_millis(config.websocket.idle_timeout_ms);
    loop {
        let idle_deadline = activity.last() + idle_timeout;
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
            // Transport idle (ADR 0020): no valid WebSocket frame in either direction for the
            // idle window. The writer may have sent while this branch slept, so the clock is read
            // again before closing.
            _ = tokio::time::sleep_until(idle_deadline), if !idle_timeout.is_zero() => {
                if activity.last() + idle_timeout > tokio::time::Instant::now() {
                    continue;
                }
                info!(event = "ws_idle_close", idle_timeout_ms = config.websocket.idle_timeout_ms, "closing a voice session with no transport activity");
                // Bounded like any send: a full urgent lane means the writer is stuck on the peer,
                // and the write timeout ends that anyway.
                let _ = timeout(write_timeout, urgent_tx.send(OutboundMessage::Close(1001))).await;
                break;
            }
            // The guard `wait_for` returns is not `Send`; drop it inside the branch future.
            _ = async { drop(writer_stopped.wait_for(|stopped| *stopped).await) } => {
                break;
            }
            next = receiver.next() => {
                activity.touch();
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

/// Web Assistant must observe server-owned tool execution even if its ClientHello
/// predates the optional tool_status feature. This is independent of Device MCP.
pub(super) fn tool_status_enabled(
    is_web_assistant: bool,
    features: &crate::protocol::ClientFeatures,
) -> bool {
    is_web_assistant || features.tool_status
}
