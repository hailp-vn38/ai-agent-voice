use super::{AppState, PendingConnection, ota};
use crate::{
    database::{Database, EnrollmentStatus},
    lifecycle::DrainRegistration,
    protocol::{ClientMessage, ServerHello, parse_client_message},
    services::device_enrollment::PromptError,
};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use serde_json::json;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::{Instant, sleep_until, timeout}};
use tokio_util::sync::CancellationToken;

type Sender = SplitSink<WebSocket, Message>;
struct Encoding {
    job: JoinHandle<Result<Vec<Vec<u8>>, PromptError>>,
}
impl Encoding {
    fn abort(&self) {
        self.job.abort();
    }
}
impl Drop for Encoding {
    fn drop(&mut self) {
        self.job.abort();
    }
}
const SEND_TIMEOUT: Duration = Duration::from_secs(1);

pub(in crate::app) async fn run(
    socket: WebSocket,
    state: AppState,
    connection: PendingConnection,
    registration: DrainRegistration,
) {
    let stopping = state.lifecycle.stopping().clone();
    let drain = registration.close_signal();
    let config = &state.config.database.devices.enrollment;
    let remaining = connection.pending.expires_at.saturating_sub(ota::unix_seconds()).max(0) as u64;
    let deadline = Instant::now() + Duration::from_secs(remaining.min(config.ws_timeout_seconds));
    let (mut sender, mut receiver) = socket.split();
    let first = tokio::select! {
        _ = stopping.cancelled() => None,
        _ = drain.cancelled() => None,
        _ = sleep_until(deadline) => None,
        message = timeout(Duration::from_millis(state.config.server.hello_timeout_ms), receiver.next()) => {
            message.ok().flatten()
        }
    };
    let hello_valid = match first {
        Some(Ok(Message::Text(text))) if text.len() <= state.config.websocket.max_frame_bytes => {
            matches!(parse_client_message(&text), Ok(ClientMessage::Hello(hello)) if hello.validate_v1().is_ok())
        }
        Some(Ok(Message::Text(text))) if text.len() > state.config.websocket.max_frame_bytes => {
            close(&mut sender, 1009).await;
            return;
        }
        Some(Ok(Message::Binary(bytes))) if bytes.len() > state.config.websocket.max_frame_bytes => {
            close(&mut sender, 1009).await;
            return;
        }
        _ => false,
    };
    if !hello_valid {
        close(&mut sender, if stopping.is_cancelled() || drain.is_cancelled() { 1001 } else { 1002 }).await;
        return;
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    let hello = serde_json::to_string(&ServerHello::v1(&session_id)).expect("serializable hello");
    if !send(&mut sender, Message::Text(hello.into())).await {
        return;
    }
    let Some(database) = state.database.clone() else {
        close(&mut sender, 1011).await;
        return;
    };
    let initial = tokio::select! {
        _ = stopping.cancelled() => { close(&mut sender, 1001).await; return; }
        _ = drain.cancelled() => { close(&mut sender, 1001).await; return; }
        _ = sleep_until(deadline) => { close(&mut sender, 1000).await; return; }
        result = database.enrollment_status(&connection.device_id, &connection.pending.code, ota::unix_seconds()) => result,
    };
    let status = initial.unwrap_or(EnrollmentStatus::Unavailable);
    if status != EnrollmentStatus::Pending {
        terminal(&mut sender, &session_id, status).await;
        return;
    }
    let cancel = CancellationToken::new();
    let (mut updates, worker) = status_worker(
        database, connection.device_id.clone(), connection.pending.code.clone(),
        Duration::from_millis(config.ws_poll_interval_ms), cancel.clone(),
    );
    let mut encoding = Some(start_encoding(&connection));
    let mut packets = VecDeque::new();
    let mut started = false;
    let mut next_audio = Instant::now();
    let mut last_prompt = Instant::now();
    let mut terminal_status = None;
    let mut close_code = 1000;
    loop {
        tokio::select! {
            biased;
            _ = stopping.cancelled() => { close_code = 1001; break; }
            _ = drain.cancelled() => { close_code = 1001; break; }
            _ = sleep_until(deadline) => break,
            changed = updates.changed() => {
                let status = if changed.is_err() { EnrollmentStatus::Unavailable } else { *updates.borrow_and_update() };
                if status != EnrollmentStatus::Pending {
                    terminal_status = Some(status);
                    break;
                }
            }
            result = async { (&mut encoding.as_mut().expect("encoding guard").job).await }, if encoding.is_some() => {
                encoding.take();
                match result {
                    Ok(Ok(encoded)) if !encoded.is_empty() && ota::unix_seconds() < connection.pending.expires_at => {
                        packets = encoded.into();
                        let text = format!("Mã kết nối: {}. Mở web, chọn Agent → Thêm thiết bị và nhập mã này.", connection.pending.code);
                        if !control(&mut sender, json!({"type":"stt","session_id":session_id,"text":text})).await
                            || !tts(&mut sender, &session_id, "start", None).await
                        { close_code = 1011; break; }
                        started = true;
                        let spoken = format!("Mã kết nối: {}", connection.pending.code.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
                        if !tts(&mut sender, &session_id, "sentence_start", Some(&spoken)).await { close_code = 1011; break; }
                        next_audio = Instant::now();
                    }
                    _ => { close_code = 1011; break; }
                }
            }
            _ = sleep_until(next_audio), if started => {
                let Some(packet) = packets.pop_front() else { close_code = 1011; break; };
                if !send(&mut sender, Message::Binary(packet.into())).await { close_code = 1011; break; }
                if packets.is_empty() {
                    if !tts(&mut sender, &session_id, "stop", None).await { close_code = 1011; break; }
                    started = false;
                } else {
                    // Pace from the completed send: slow peers never cause a catch-up burst.
                    next_audio = Instant::now() + Duration::from_millis(60);
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > state.config.websocket.max_frame_bytes { close_code = 1009; break; }
                        match parse_client_message(&text) {
                            Ok(ClientMessage::Abort { session_id: id }) if scoped(&id, &session_id) => {
                                if let Some(job) = encoding.take() { job.abort(); }
                                packets.clear();
                                if started && !tts(&mut sender, &session_id, "stop", None).await { close_code = 1011; break; }
                                started = false;
                            }
                            Ok(ClientMessage::Listen { session_id: id, .. }) if scoped(&id, &session_id)
                                && !started && encoding.is_none()
                                && last_prompt.elapsed() >= Duration::from_secs(config.ws_prompt_repeat_seconds) => {
                                    last_prompt = Instant::now();
                                    encoding = Some(start_encoding(&connection));
                                }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() > state.config.websocket.max_frame_bytes { close_code = 1009; break; }
                        // All uplink packets are dropped without decoder, capture, VAD or ASR.
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if !send(&mut sender, Message::Pong(payload)).await { break; }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                }
            }
        }
    }
    if let Some(job) = encoding.take() { job.abort(); }
    cancel.cancel();
    let _ = worker.await;
    if started {
        let _ = tts(&mut sender, &session_id, "stop", None).await;
    }
    if let Some(status) = terminal_status {
        terminal(&mut sender, &session_id, status).await;
    } else {
        close(&mut sender, close_code).await;
    }
    // connection owns the enrollment capacity permit; registration owns drain accounting.
    drop(connection);
    drop(registration);
}

fn scoped(id: &Option<String>, expected: &str) -> bool {
    id.as_deref().is_none_or(|id| id.is_empty() || id == expected)
}
fn start_encoding(connection: &PendingConnection) -> Encoding {
    let runtime = connection.runtime.clone();
    let code = connection.pending.code.clone();
    Encoding { job: tokio::spawn(async move { runtime.encode(code).await }) }
}

fn status_worker(
    database: Arc<Database>, device_id: String, code: String,
    cadence: Duration, cancel: CancellationToken,
) -> (watch::Receiver<EnrollmentStatus>, JoinHandle<()>) {
    let (sender, receiver) = watch::channel(EnrollmentStatus::Pending);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(cadence);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sender.closed() => break,
                _ = interval.tick() => {}
            }
            let status = tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sender.closed() => break,
                result = database.enrollment_status(&device_id, &code, ota::unix_seconds()) => {
                    result.unwrap_or(EnrollmentStatus::Unavailable)
                }
            };
            if sender.send(status).is_err() || status != EnrollmentStatus::Pending { break; }
        }
    });
    (receiver, task)
}

async fn terminal(sender: &mut Sender, session_id: &str, status: EnrollmentStatus) {
    let (code, text) = match status {
        EnrollmentStatus::Registered => (1000, "Đã liên kết thiết bị. Hãy mở lại kết nối để bắt đầu trò chuyện."),
        EnrollmentStatus::Expired => (1000, "Mã kết nối đã hết hạn. Hãy mở lại kết nối để lấy mã mới."),
        EnrollmentStatus::Blocked => (1008, "Thiết bị đang bị vô hiệu hóa."),
        _ => (1011, "Tạm thời không thể kiểm tra trạng thái liên kết."),
    };
    let _ = control(sender, json!({"type":"stt","session_id":session_id,"text":text})).await;
    close(sender, code).await;
}
async fn tts(sender: &mut Sender, session_id: &str, state: &str, text: Option<&str>) -> bool {
    let mut message = json!({"type":"tts","state":state,"session_id":session_id});
    if let Some(text) = text { message["text"] = text.into(); }
    control(sender, message).await
}
async fn control(sender: &mut Sender, value: serde_json::Value) -> bool {
    send(sender, Message::Text(value.to_string().into())).await
}
async fn send(sender: &mut Sender, message: Message) -> bool {
    matches!(timeout(SEND_TIMEOUT, sender.send(message)).await, Ok(Ok(())))
}
async fn close(sender: &mut Sender, code: u16) {
    let _ = send(sender, Message::Close(Some(CloseFrame { code, reason: "".into() }))).await;
}
