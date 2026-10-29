use super::super::*;
use crate::services::provider_runtime::{RuntimeMetrics, RuntimePhase};
use std::time::Instant;
use tracing::Instrument;

mod admission;
mod close;
mod frame;
mod session;
mod writer;

use admission::Admission;
use close::{CloseReason, SessionCloseSet, SessionControl};
use frame::{
    Sent, TransportActivity, WriteFailure, checked_text, close_direct, next_application_frame,
    send_frame, send_outbound,
};
use session::handle_socket;
#[cfg(test)]
use session::tool_status_enabled;

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

pub(self) fn bearer_auth_result(enabled: bool) -> &'static str {
    if enabled { "accepted" } else { "not_required" }
}

/// Connection telemetry deliberately has no raw headers, token, Device ID or Client ID.
pub(self) fn log_ws_auth(
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
    pub(self) protocol_version: Option<String>,
    #[serde(rename = "device-id")]
    pub(self) device_id: Option<String>,
    #[serde(rename = "client-id")]
    pub(self) client_id: Option<String>,
    pub(self) authorization: Option<String>,
    pub(self) ticket: Option<String>,
}

pub(self) struct SocketRuntimes {
    pub(self) job_submit: Option<crate::services::task_flow::VoiceTaskSubmit>,
    pub(self) active_device_sessions:
        Arc<crate::services::active_device_sessions::ActiveDeviceSessions>,
    pub(self) database: Option<Arc<crate::database::Database>>,
    pub(self) system_mcp: Option<crate::tools::system_mcp::SessionSystemMcp>,
    pub(self) reminder_tools: Option<crate::services::notifications::SessionReminderTools>,
    pub(self) assistant_connection: Option<super::super::assistant::Connection>,
    pub(self) connect_started: Instant,
    pub(self) metrics: Option<Arc<RuntimeMetrics>>,
    pub(self) asr: Arc<AsrWorkerRuntime>,
    pub(self) vad: Arc<VadWorkerRuntime>,
    pub(self) llm: Arc<LlmRuntime>,
    pub(self) tts: Arc<TtsWorkerRuntime>,
    pub(self) vad_segmenter: VadSegmenterConfig,
    pub(self) vad_pre_roll_samples: u64,
    pub(self) active_turn_limiter: Arc<ActiveTurnLimiter>,
    pub(self) pilot_admission: crate::session::pilot::PilotAdmission,
    /// Test-only; see [`WriterOutcomeProbe`](crate::session::WriterOutcomeProbe).
    pub(self) writer_probe: Option<Arc<dyn crate::session::WriterOutcomeProbe>>,
    pub(self) profile: crate::session::EffectiveSessionProfile,
    /// This connection's own session identity, resolved before the upgrade so the archive and the
    /// Voice Session cannot disagree about which connection a transcript belongs to.
    pub(self) session_id: String,
    /// The optional Persistent Transcript binding, resolved with the profile above.  `None` means
    /// capture is off or this session has no database identity, and the actor then archives
    /// nothing at all.
    pub(self) transcript: Option<crate::database::history::TranscriptCapture>,
    /// Ticket 10 Observe, resolved at admission. `None` keeps the actor free of speaker inference
    /// and utterance PCM retention.
    pub(self) speaker_observe: Option<Arc<SpeakerObserve>>,
    /// The application admission gate. The actor only asks; the application closes it.
    pub(self) admission_gate: Arc<crate::lifecycle::AdmissionGate>,
}

#[cfg(test)]
#[path = "tool_status_tests.rs"]
mod tool_status_negotiation_tests;
