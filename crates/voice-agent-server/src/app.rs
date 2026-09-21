use crate::{
    config::AppConfig,
    protocol::{
        parse_client_message, ClientMessage, Firmware, OtaResponse, OtaWebsocket, ServerHello,
        ServerTime,
    },
    session::{OutboundMessage, SessionActor, SessionEvent},
};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    borrow::Cow,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::mpsc, time::timeout};
use tower_http::trace::TraceLayer;
use tracing::{debug, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
}

pub fn router(config: AppConfig) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/voice/ota/", get(ota).post(ota))
        .route("/voice/v1/", get(websocket))
        .with_state(AppState {
            config: Arc::new(config),
        })
        .layer(TraceLayer::new_for_http())
}

async fn health() -> &'static str {
    "ok"
}

async fn ota(State(state): State<AppState>) -> Json<OtaResponse> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    Json(OtaResponse {
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
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let config = state.config;
    if !header_is(&headers, "protocol-version", "1")
        || !headers.contains_key("device-id")
        || !headers.contains_key("client-id")
    {
        return (
            StatusCode::BAD_REQUEST,
            "Protocol-Version, Device-Id and Client-Id are required",
        )
            .into_response();
    }
    if !config.auth.token.is_empty()
        && !header_is(
            &headers,
            header::AUTHORIZATION.as_str(),
            &format!("Bearer {}", config.auth.token),
        )
    {
        return (StatusCode::UNAUTHORIZED, "invalid bearer token").into_response();
    }
    let max = config.websocket.max_frame_bytes;
    upgrade
        .max_frame_size(max.saturating_add(1024))
        .on_upgrade(move |socket| handle_socket(socket, config))
        .into_response()
}

fn header_is(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers.get(name).and_then(|value| value.to_str().ok()) == Some(expected)
}

async fn handle_socket(socket: WebSocket, config: Arc<AppConfig>) {
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
    debug!(transport = %hello.transport, "accepted ClientHello");

    let (control_tx, mut control_rx) = mpsc::channel(config.limits.outbound_control_queue);
    let (audio_tx, mut audio_rx) = mpsc::channel(config.limits.outbound_audio_queue);
    let actor = match SessionActor::new(
        Uuid::new_v4().to_string(),
        control_tx.clone(),
        audio_tx,
        config.max_capture_frames(),
    ) {
        Ok(actor) => actor,
        Err(error) => {
            debug!(%error, "failed to initialize session audio runtime");
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
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                Some(message) = control_rx.recv() => if send_outbound(&mut sender, message).await { break; },
                Some(message) = audio_rx.recv() => if send_outbound(&mut sender, message).await { break; },
                else => break,
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
                    .try_send(SessionEvent::ClientAudio(payload))
                    .is_err()
                {
                    debug!("dropped binary because bounded ingress is full or session is closed");
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
        }
    }
    drop(ingress_tx);
    let _ = session.await;
    drop(control_tx);
    let _ = writer.await;
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
        OutboundMessage::Text(text) => sender.send(Message::Text(text)).await,
        OutboundMessage::Binary(bytes) => sender.send(Message::Binary(bytes)).await,
        OutboundMessage::Close(code) => {
            sender
                .send(Message::Close(Some(CloseFrame {
                    code,
                    reason: Cow::Borrowed(""),
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
            reason: Cow::Borrowed(""),
        })))
        .await;
}
