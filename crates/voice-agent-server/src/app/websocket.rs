use super::*;

pub(super) async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WebsocketQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let config = state.config;
    let providers = state.providers;
    let asr_runtime = state.asr_runtime;
    let vad_runtime = state.vad_runtime;
    let llm_runtime = state.llm_runtime;
    let tts_runtime = state.tts_runtime;
    let active_turn_limiter = state.active_turn_limiter;
    if !header_or_query_is_or_absent(
        &headers,
        "protocol-version",
        query.protocol_version.as_deref(),
        "1",
    ) || !header_or_query_present(&headers, "device-id", query.device_id.as_deref())
        || !header_or_query_present(&headers, "client-id", query.client_id.as_deref())
    {
        return (
            StatusCode::BAD_REQUEST,
            "Protocol-Version, Device-Id and Client-Id are required",
        )
            .into_response();
    }
    if !config.auth.token.is_empty()
        && !header_or_query_is(
            &headers,
            header::AUTHORIZATION.as_str(),
            query.authorization.as_deref(),
            &format!("Bearer {}", config.auth.token),
        )
    {
        return (StatusCode::UNAUTHORIZED, "invalid bearer token").into_response();
    }
    info!("WebSocket upgrade accepted");
    let max = config.websocket.max_frame_bytes;
    upgrade
        .max_frame_size(max.saturating_add(1024))
        .on_upgrade(move |socket| {
            handle_socket(
                socket,
                config,
                providers,
                SocketRuntimes {
                    asr: asr_runtime,
                    vad: vad_runtime,
                    llm: llm_runtime,
                    tts: tts_runtime,
                    active_turn_limiter,
                },
            )
        })
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct WebsocketQuery {
    #[serde(rename = "protocol-version")]
    protocol_version: Option<String>,
    #[serde(rename = "device-id")]
    device_id: Option<String>,
    #[serde(rename = "client-id")]
    client_id: Option<String>,
    authorization: Option<String>,
}

struct PlaybackTurn {
    id: crate::session::TurnId,
    start_was_sent: bool,
}

fn header_is(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers.get(name).and_then(|value| value.to_str().ok()) == Some(expected)
}

/// Browser clients cannot set custom headers; legacy clients may omit the version entirely.
fn header_or_query_is_or_absent(
    headers: &HeaderMap,
    header_name: &str,
    query_value: Option<&str>,
    expected: &str,
) -> bool {
    match headers.get(header_name) {
        Some(_) => header_is(headers, header_name, expected),
        None => query_value.is_none_or(|value| value == expected),
    }
}

fn header_or_query_is(
    headers: &HeaderMap,
    header_name: &str,
    query_value: Option<&str>,
    expected: &str,
) -> bool {
    match headers.get(header_name) {
        Some(_) => header_is(headers, header_name, expected),
        None => query_value == Some(expected),
    }
}

/// Browser WebSocket APIs cannot set custom headers, so accept identity query fallbacks.
fn header_or_query_present(
    headers: &HeaderMap,
    header_name: &str,
    query_value: Option<&str>,
) -> bool {
    match headers.get(header_name) {
        Some(value) => value.to_str().is_ok_and(|value| !value.is_empty()),
        None => query_value.is_some_and(|value| !value.is_empty()),
    }
}

struct SocketRuntimes {
    asr: Arc<AsrWorkerRuntime>,
    vad: Arc<VadWorkerRuntime>,
    llm: Arc<LlmRuntime>,
    tts: Arc<TtsWorkerRuntime>,
    active_turn_limiter: Arc<ActiveTurnLimiter>,
}

async fn handle_socket(
    socket: WebSocket,
    config: Arc<AppConfig>,
    providers: Arc<ProviderSet>,
    runtimes: SocketRuntimes,
) {
    let (mut sender, mut receiver) = socket.split();
    let first = timeout(
        Duration::from_millis(config.server.hello_timeout_ms),
        receiver.next(),
    )
    .await;
    let hello = match first {
        Ok(Some(Ok(message))) => message,
        _ => {
            close_direct(&mut sender, 1002).await;
            return;
        }
    };
    let text = match checked_text(hello, config.websocket.max_frame_bytes) {
        Ok(text) => text,
        Err(code) => {
            close_direct(&mut sender, code).await;
            return;
        }
    };
    let hello = match parse_client_message(&text) {
        Ok(ClientMessage::Hello(hello)) if hello.validate_v1().is_ok() => hello,
        _ => {
            close_direct(&mut sender, 1002).await;
            return;
        }
    };
    info!(transport = %hello.transport, "ClientHello accepted");

    let (control_tx, mut control_rx) = mpsc::channel(config.limits.outbound_control_queue);
    let (urgent_tx, mut urgent_rx) = mpsc::channel(config.limits.urgent_control_queue);
    let (audio_tx, mut audio_rx) = mpsc::channel(config.limits.outbound_audio_queue);
    let (writer_event_tx, writer_event_rx) = mpsc::channel(config.limits.session_event_queue);
    let generation_gate = Arc::new(crate::session::GenerationGate::new());
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let vad_config = config
        .providers
        .vad
        .silero_onnx
        .as_ref()
        .expect("validated VAD config");
    let actor = match SessionActor::new_with_runtimes_and_limiter_and_outbound(
        Uuid::new_v4().to_string(),
        control_tx.clone(),
        urgent_tx.clone(),
        audio_tx,
        Arc::clone(&generation_gate),
        shutdown_tx,
        config.max_capture_frames(),
        config.llm.max_history_messages,
        SessionRuntimes {
            asr: runtimes.asr,
            vad: runtimes.vad,
            llm: runtimes.llm,
            tts: runtimes.tts,
            active_turn_limiter: runtimes.active_turn_limiter,
            vad_segmenter_config: VadSegmenterConfig {
                speech_threshold: vad_config.speech_threshold,
                exit_threshold: vad_config.exit_threshold,
                min_speech_samples: vad_config.min_speech_ms * 16,
                end_silence_samples: vad_config.end_silence_ms * 16,
            },
            pre_roll_samples: vad_config.pre_roll_ms * 16,
        },
    ) {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "failed to initialize session audio runtime");
            close_direct(&mut sender, 1011).await;
            return;
        }
    };
    let actor = match actor.with_delivery_providers_config(&providers, config.speech_output.clone())
    {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "failed to initialize session speech output");
            close_direct(&mut sender, 1011).await;
            return;
        }
    };
    let actor = actor
        .with_client_capabilities(
            hello.features.aec,
            crate::session::BargeInPolicy {
                enabled: config.barge_in.enabled,
                trust_client_aec_feature: config.barge_in.trust_client_aec_feature,
            },
        )
        .with_writer_events(writer_event_rx);
    let server_hello = serde_json::to_string(&ServerHello::v1(actor.session_id()))
        .expect("ServerHello is serializable");
    if actor.send_control(server_hello).is_err() {
        close_direct(&mut sender, 1011).await;
        return;
    }
    info!("ServerHello sent; voice session connected");
    let writer = tokio::spawn(async move {
        let mut active_turn: Option<PlaybackTurn> = None;
        let mut deferred_finish: Option<OutboundMessage> = None;
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                },
                Some(message) = urgent_rx.recv() => {
                    match message {
                        OutboundMessage::AbortTurn { turn_id, text } => {
                            if active_turn.as_ref().is_some_and(|active| active.id == turn_id) {
                                deferred_finish = None;
                                let start_was_sent = active_turn.as_ref().is_some_and(|active| active.start_was_sent);
                                let stop_was_sent = if start_was_sent {
                                    if send_outbound(&mut sender, OutboundMessage::Text(text), &generation_gate).await {
                                        let _ = writer_event_tx.send(WriterEvent::Failed { turn_id: Some(turn_id) }).await;
                                        break;
                                    }
                                    true
                                } else {
                                    false
                                };
                                active_turn = None;
                                let _ = writer_event_tx.send(WriterEvent::TurnClosed {
                                    turn_id,
                                    outcome: WriterTurnOutcome::Aborted { start_was_sent, stop_was_sent },
                                }).await;
                            } else {
                                // Begin may still be queued behind this urgent command. The
                                // actor needs a terminal outcome so it can release this turn's
                                // permit; a later Begin for the same invalid generation is
                                // rejected at the gate.
                                let _ = writer_event_tx
                                    .send(WriterEvent::TurnClosed {
                                        turn_id,
                                        outcome: WriterTurnOutcome::Aborted {
                                            start_was_sent: false,
                                            stop_was_sent: false,
                                        },
                                    })
                                    .await;
                            }
                        }
                        message => if send_outbound(&mut sender, message, &generation_gate).await {
                            break;
                        },
                    }
                },
                // A normal stop follows the final paced packet, but only after the urgent
                // lane has had its chance to preempt it.
                _ = std::future::ready(()), if deferred_finish.is_some() && audio_rx.is_empty() => {
                    let OutboundMessage::FinishTurn { turn_id, text } = deferred_finish.take().expect("checked above") else { unreachable!() };
                    if send_outbound(&mut sender, OutboundMessage::Text(text), &generation_gate).await {
                        let _ = writer_event_tx.send(WriterEvent::Failed { turn_id: Some(turn_id) }).await;
                        break;
                    }
                    active_turn = None;
                    let _ = writer_event_tx.send(WriterEvent::TurnClosed { turn_id, outcome: WriterTurnOutcome::Normal }).await;
                },
                Some(message) = control_rx.recv() => {
                    match message {
                        OutboundMessage::BeginTurn { generation, turn_id, text } => {
                            if active_turn.is_none() && generation_gate.admits(generation) {
                                if send_outbound(&mut sender, OutboundMessage::Text(text), &generation_gate).await {
                                    let _ = writer_event_tx.send(WriterEvent::Failed { turn_id: Some(turn_id) }).await;
                                    break;
                                }
                                active_turn = Some(PlaybackTurn { id: turn_id, start_was_sent: true });
                            }
                        }
                        message @ OutboundMessage::FinishTurn { .. } => {
                            if !audio_rx.is_empty() {
                                deferred_finish = Some(message);
                            } else if let OutboundMessage::FinishTurn { turn_id, text } = message
                                && active_turn.as_ref().is_some_and(|active| active.id == turn_id)
                            {
                                if send_outbound(&mut sender, OutboundMessage::Text(text), &generation_gate).await {
                                    let _ = writer_event_tx.send(WriterEvent::Failed { turn_id: Some(turn_id) }).await;
                                    break;
                                }
                                active_turn = None;
                                let _ = writer_event_tx.send(WriterEvent::TurnClosed { turn_id, outcome: WriterTurnOutcome::Normal }).await;
                            }
                        }
                        message => if send_outbound(&mut sender, message, &generation_gate).await {
                            break;
                        },
                    }
                },
                Some(message) = audio_rx.recv() => {
                    if let OutboundMessage::Binary { turn_id, .. } = &message
                        && active_turn.as_ref().is_some_and(|active| active.start_was_sent && active.id == *turn_id)
                        && send_outbound(&mut sender, message, &generation_gate).await
                    {
                        break;
                    }
                },
                else => {
                    if let Some(OutboundMessage::FinishTurn { turn_id, text }) = deferred_finish.take()
                        && send_outbound(&mut sender, OutboundMessage::Text(text), &generation_gate).await
                    {
                        let _ = writer_event_tx.send(WriterEvent::Failed { turn_id: Some(turn_id) }).await;
                        break;
                    }
                    break;
                },
            }
        }
    });

    let (ingress_tx, ingress_rx) = mpsc::channel(config.limits.session_event_queue);
    let session = tokio::spawn(actor.run(ingress_rx));

    while let Some(next) = receiver.next().await {
        match next {
            Ok(Message::Text(text)) => {
                if text.len() > config.websocket.max_frame_bytes {
                    let _ = urgent_tx.send(OutboundMessage::Close(1009)).await;
                    break;
                }
                match parse_client_message(&text) {
                    Ok(message) => {
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
                info!(?frame, "websocket peer closed connection");
                break;
            }
            Err(error) => {
                warn!(%error, "websocket receive failed");
                break;
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
        }
    }
    drop(ingress_tx);
    let _ = session.await;
    drop(control_tx);
    drop(urgent_tx);
    let _ = writer.await;
    info!("voice session disconnected");
}

fn checked_text(message: Message, max: usize) -> Result<String, u16> {
    match message {
        Message::Text(text) if text.len() <= max => Ok(text.to_string()),
        Message::Text(_) => Err(1009),
        Message::Binary(_) => Err(1002),
        _ => Err(1002),
    }
}

async fn send_outbound(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: OutboundMessage,
    generation_gate: &crate::session::GenerationGate,
) -> bool {
    let generation = match &message {
        OutboundMessage::TurnText { generation, .. }
        | OutboundMessage::Binary { generation, .. } => Some(*generation),
        OutboundMessage::Text(_)
        | OutboundMessage::BeginTurn { .. }
        | OutboundMessage::FinishTurn { .. }
        | OutboundMessage::AbortTurn { .. }
        | OutboundMessage::Close(_) => None,
    };
    if generation.is_some_and(|generation| !generation_gate.admits(generation)) {
        return false;
    }
    let closes = matches!(message, OutboundMessage::Close(_));
    let result = match message {
        OutboundMessage::Text(text) | OutboundMessage::TurnText { text, .. } => {
            let llm_message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .filter(|value| value["type"] == "llm");
            let result = sender.send(Message::Text(text.into())).await;
            if result.is_ok()
                && let Some(message) = llm_message
                && let Some(llm_text) = message["text"].as_str()
            {
                info!(
                    message_type = "llm",
                    session_id = message["session_id"].as_str().unwrap_or(""),
                    chars = llm_text.chars().count(),
                    text = %llm_text,
                    "WebSocket text sent to client"
                );
            }
            result
        }
        OutboundMessage::Binary { packet, .. } => sender.send(Message::Binary(packet.into())).await,
        // Semantic playback commands must be consumed by the writer state machine above.
        OutboundMessage::BeginTurn { .. }
        | OutboundMessage::FinishTurn { .. }
        | OutboundMessage::AbortTurn { .. } => return false,
        OutboundMessage::Close(code) => {
            sender
                .send(Message::Close(Some(CloseFrame {
                    code,
                    reason: "".into(),
                })))
                .await
        }
    };
    if let Err(ref error) = result {
        warn!(%error, "websocket writer failed");
    }
    closes || result.is_err()
}

async fn close_direct(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, code: u16) {
    let _ = sender
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: "".into(),
        })))
        .await;
}
