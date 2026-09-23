use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    protocol::{
        ClientMessage, Firmware, OtaResponse, OtaWebsocket, ServerHello, ServerTime,
        parse_client_message,
    },
    providers::ProviderSet,
    session::{ActiveTurnLimiter, OutboundMessage, SessionActor, SessionEvent, SessionRuntimes},
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        WorkerSupervisor,
    },
};
use axum::{
    Json, Router,
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::mpsc, time::timeout};
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};
use url::Url;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub providers: Arc<ProviderSet>,
    pub asr_runtime: Arc<AsrWorkerRuntime>,
    pub vad_runtime: Arc<VadWorkerRuntime>,
    pub llm_runtime: Arc<LlmRuntime>,
    pub tts_runtime: Arc<TtsWorkerRuntime>,
    pub worker_supervisor: Arc<WorkerSupervisor>,
    pub active_turn_limiter: Arc<ActiveTurnLimiter>,
}

impl AppState {
    /// Builds the application-owned inference runtimes from validated config.
    pub fn new(config: AppConfig, providers: Arc<ProviderSet>) -> Self {
        let asr_runtime = Arc::new(AsrWorkerRuntime::new(
            providers.asr_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.asr.max_workers,
                command_capacity: config.workers.asr.command_queue_capacity,
                final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
            },
        ));
        let vad_runtime = Arc::new(VadWorkerRuntime::new(
            providers.vad_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.vad.max_workers,
                command_capacity: config.workers.vad.command_queue_capacity,
                final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
            },
        ));
        let worker_supervisor = Arc::new(WorkerSupervisor::start(
            Arc::clone(&asr_runtime),
            Arc::clone(&vad_runtime),
        ));
        let llm_runtime = Arc::new(LlmRuntime::new(
            providers.llm_provider(),
            config.limits.llm_concurrency,
            Duration::from_millis(
                config
                    .providers
                    .llm
                    .openai
                    .as_ref()
                    .expect("validated OpenAI config")
                    .timeout_ms,
            ),
        ));
        let tts_runtime = Arc::new(TtsWorkerRuntime::new(
            providers.tts_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.tts.max_workers,
                command_capacity: config.workers.tts.command_queue_capacity,
                final_timeout: Duration::from_millis(config.tts.timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
            },
        ));
        let active_turn_limiter = Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns));
        Self {
            config: Arc::new(config),
            providers,
            asr_runtime,
            vad_runtime,
            llm_runtime,
            tts_runtime,
            worker_supervisor,
            active_turn_limiter,
        }
    }
}

/// Application seam for tests and other callers that have already initialized providers.
pub fn router_with_providers(config: AppConfig, providers: Arc<ProviderSet>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/voice/ota/", get(ota).post(ota).options(ota_options))
        .route("/voice/v1/", get(websocket))
        .with_state(AppState::new(config, providers))
        // Never include query parameters here: browser compatibility may carry an auth token.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!("http_request", method = %request.method(), path = request.uri().path())
            }),
        )
}

/// Builds the public application only after local provider validation and warmup succeed.
pub fn application(config: AppConfig) -> Result<Router, crate::providers::ProviderLoadError> {
    let providers = Arc::new(ProviderSet::load(&config)?);
    Ok(router_with_providers(config, providers))
}

async fn health() -> &'static str {
    "ok"
}

async fn ota(State(state): State<AppState>, request_headers: HeaderMap) -> Response {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut response = Json(OtaResponse {
        server_time: ServerTime {
            timestamp,
            timezone_offset: 420,
        },
        firmware: Firmware {
            version: "",
            url: "",
        },
        websocket: OtaWebsocket {
            url: state.config.server.public_ws_url.to_string(),
            token: state.config.auth.token.clone(),
        },
    })
    .into_response();
    apply_ota_cors(response.headers_mut(), &request_headers);
    response
}

/// Answers browser and device preflight requests without creating a voice session.
async fn ota_options(request_headers: HeaderMap) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ALLOW,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Device-Id, Client-Id, Authorization"),
    );
    apply_ota_cors(headers, &request_headers);
    response
}

/// Only local browser tooling may read OTA's optional bearer token cross-origin.
fn apply_ota_cors(response_headers: &mut HeaderMap, request_headers: &HeaderMap) {
    let Some(origin) = request_headers.get(header::ORIGIN) else {
        return;
    };
    let Ok(origin_text) = origin.to_str() else {
        return;
    };
    let Ok(origin_url) = Url::parse(origin_text) else {
        return;
    };
    let Some(host) = origin_url.host_str() else {
        return;
    };
    let is_loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !is_loopback {
        return;
    }

    response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    response_headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}

async fn websocket(
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
struct WebsocketQuery {
    #[serde(rename = "protocol-version")]
    protocol_version: Option<String>,
    #[serde(rename = "device-id")]
    device_id: Option<String>,
    #[serde(rename = "client-id")]
    client_id: Option<String>,
    authorization: Option<String>,
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
    let (audio_tx, mut audio_rx) = mpsc::channel(config.limits.outbound_audio_queue);
    let vad_config = config
        .providers
        .vad
        .silero_onnx
        .as_ref()
        .expect("validated VAD config");
    let actor = match SessionActor::new_with_runtimes_and_limiter(
        Uuid::new_v4().to_string(),
        control_tx.clone(),
        audio_tx,
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
    let server_hello = serde_json::to_string(&ServerHello::v1(actor.session_id()))
        .expect("ServerHello is serializable");
    if actor.send_control(server_hello).is_err() {
        close_direct(&mut sender, 1011).await;
        return;
    }
    info!("ServerHello sent; voice session connected");
    let writer = tokio::spawn(async move {
        let mut deferred_tts_stop = None;
        let mut invalidated_generation = 0;
        loop {
            // A normal stop follows the final paced packet even though control is otherwise
            // preferred over audio by this writer.
            if deferred_tts_stop.is_some() && audio_rx.is_empty() {
                if send_outbound(
                    &mut sender,
                    deferred_tts_stop.take().expect("checked above"),
                )
                .await
                {
                    break;
                }
                continue;
            }
            tokio::select! {
                biased;
                Some(message) = control_rx.recv() => {
                    if let OutboundMessage::InvalidateAudio(generation) = message {
                        invalidated_generation = invalidated_generation.max(generation);
                    } else if is_normal_tts_stop(&message) && !audio_rx.is_empty() {
                        deferred_tts_stop = Some(message);
                    } else if send_outbound(&mut sender, message).await {
                        break;
                    }
                },
                Some(message) = audio_rx.recv() => {
                    if let OutboundMessage::Binary { generation, .. } = &message
                        && *generation <= invalidated_generation
                    {
                        continue;
                    }
                    if send_outbound(&mut sender, message).await { break; }
                },
                else => {
                    if let Some(message) = deferred_tts_stop.take()
                        && send_outbound(&mut sender, message).await
                    {
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
                    let _ = control_tx.send(OutboundMessage::Close(1009)).await;
                    break;
                }
                match parse_client_message(&text) {
                    Ok(message) => {
                        if ingress_tx
                            .try_send(SessionEvent::ClientMessage(message))
                            .is_err()
                        {
                            let _ = control_tx.send(OutboundMessage::Close(1013)).await;
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
                    let _ = control_tx.send(OutboundMessage::Close(1009)).await;
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
    let _ = writer.await;
    info!("voice session disconnected");
}

fn is_normal_tts_stop(message: &OutboundMessage) -> bool {
    let Some(text) = message.as_text() else {
        return false;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    payload["type"] == "tts" && payload["state"] == "stop"
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
) -> bool {
    let closes = matches!(message, OutboundMessage::Close(_));
    let result = match message {
        OutboundMessage::Text(text) => sender.send(Message::Text(text.into())).await,
        OutboundMessage::Binary { packet, .. } => sender.send(Message::Binary(packet.into())).await,
        OutboundMessage::InvalidateAudio(_) => return false,
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
