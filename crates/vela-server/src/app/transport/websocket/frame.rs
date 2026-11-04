use super::*;

pub(super) fn checked_text(message: Message, max: usize) -> Result<String, u16> {
    match message {
        Message::Text(text) if text.len() <= max => Ok(text.to_string()),
        Message::Text(_) => Err(1009),
        Message::Binary(_) => Err(1002),
        _ => Err(1002),
    }
}

/// What became of one outbound message that did not fail.
pub(super) enum Sent {
    /// Sent, or deliberately dropped as stale; the writer keeps going.
    Continue,
    /// A close frame went out; nothing may follow it.
    Closed,
}

pub(super) async fn send_outbound(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    activity: &TransportActivity,
    write_timeout: Duration,
    message: OutboundMessage,
    generation_gate: &crate::session::GenerationGate,
    pipeline_request: &std::sync::atomic::AtomicU64,
) -> Result<Sent, WriteFailure> {
    if let OutboundMessage::PipelineStatus { request, .. } = &message
        && *request != pipeline_request.load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(Sent::Continue);
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
        return Ok(Sent::Continue);
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
        | OutboundMessage::AbortTurn { .. } => return Ok(Sent::Continue),
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
    result?;
    activity.touch();
    Ok(if closes { Sent::Closed } else { Sent::Continue })
}

pub(super) async fn close_direct(
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

/// The transport idle clock of one connection (ADR 0020): any valid WebSocket frame received or
/// sent counts as activity. Shared by the reader, which receives and enforces the idle window, and
/// the WebSocket Writer, which sends.
pub(super) struct TransportActivity {
    origin: tokio::time::Instant,
    last_ms: std::sync::atomic::AtomicU64,
}

impl TransportActivity {
    pub(super) fn new() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
            last_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(super) fn touch(&self) {
        let elapsed = self.origin.elapsed().as_millis() as u64;
        self.last_ms
            .fetch_max(elapsed, std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn last(&self) -> tokio::time::Instant {
        self.origin + Duration::from_millis(self.last_ms.load(std::sync::atomic::Ordering::Relaxed))
    }
}

/// Why one WebSocket send failed.
pub(super) enum WriteFailure {
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
pub(super) async fn send_frame(
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
pub(super) async fn next_application_frame(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<Result<Message, axum::Error>> {
    loop {
        match receiver.next().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => return other,
        }
    }
}
