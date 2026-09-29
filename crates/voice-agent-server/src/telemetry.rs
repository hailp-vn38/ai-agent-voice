//! Process-owned bounded telemetry for External MCP.
//!
//! The seam is deliberately narrow.  A caller names a metric from the set below and a label from a
//! bounded class, and the only free-form value it can pass is a server key — which the Admin API
//! already bounds to 64 bytes of `[a-z0-9_]`.  There is therefore no spelling with which a
//! destination, a header value, a credential, a protocol session id, a tool argument or a tool
//! result could become a label: those are not parameters here at all.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// Counter and summary names.  They are constants rather than call-site strings so a backend can
/// register them once, and so a test can assert the guide's names without restating them.
pub const MCP_RESOLVE_SUCCESS_TOTAL: &str = "mcp_resolve_success_total";
pub const MCP_RESOLVE_FAILURE_TOTAL: &str = "mcp_resolve_failure_total";
pub const MCP_RESOLVE_DURATION_MS: &str = "mcp_resolve_duration_ms";
/// The aggregate cap is a fact about a whole snapshot, so it has its own counter rather than
/// borrowing a per-server one.
pub const EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL: &str =
    "external_mcp_session_tool_cap_exceeded_total";
pub const EXTERNAL_MCP_TOOL_CALLS_TOTAL: &str = "external_mcp_tool_calls_total";
pub const EXTERNAL_MCP_TOOL_CALL_DURATION_MS: &str = "external_mcp_tool_call_duration_ms";
pub const EXTERNAL_MCP_CALL_LIMITER_REJECTED_TOTAL: &str =
    "external_mcp_call_limiter_rejected_total";

/// The bounded classes one `tools/call` can report.
///
/// These are the same words the typed failures already use, so a counter and the diagnostic a log
/// line carries can never disagree about what happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    Success,
    Timeout,
    Unavailable,
    AuthFailed,
    InvalidResponse,
    ProtocolError,
}

impl CallOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::AuthFailed => "auth_failed",
            Self::InvalidResponse => "invalid_response",
            Self::ProtocolError => "protocol_error",
        }
    }
}

impl std::fmt::Display for CallOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The outcome a server's resolution reported, as a bounded class.  It rides along with every
/// resolve counter so a success and a failure can be told apart without reading the metric name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    Success,
    Failure,
}

impl ResolveOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

impl std::fmt::Display for ResolveOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The one place External MCP numbers leave this process.
///
/// Implementations own the registry; this trait owns the vocabulary.  Nothing here accepts a
/// destination, a header, a credential, a payload or a session identity, so a sink cannot be asked
/// to record one even by accident.
pub trait Telemetry: Send + Sync {
    /// `mcp_resolve_success_total` and one `mcp_resolve_duration_ms` sample.
    fn resolve_succeeded(&self, server_key: &str, elapsed: Duration);
    /// `mcp_resolve_failure_total{reason}` and one `mcp_resolve_duration_ms` sample.
    fn resolve_failed(&self, server_key: &str, reason: &'static str, elapsed: Duration);
    /// `external_mcp_session_tool_cap_exceeded_total`: one admission's aggregate tool cap was
    /// exceeded, so the whole External MCP snapshot was dropped.
    ///
    /// It takes no server key and emits no duration on purpose.  The cap is a property of the
    /// snapshot rather than of any one server, and every server in it has already reported its own
    /// resolution: counting it again as a per-server failure would report a second outcome, and a
    /// duration for work that never happened.
    fn session_tool_cap_exceeded(&self);
    /// `external_mcp_call_limiter_rejected_total`: a call whose permit never became available
    /// inside the caller's own budget, so no request was sent at all.
    fn call_limiter_rejected(&self, server_key: &str);
    /// `external_mcp_tool_calls_total{server_key,outcome}` and one
    /// `external_mcp_tool_call_duration_ms{server_key,outcome}` sample.
    fn call_finished(&self, server_key: &str, outcome: CallOutcome, elapsed: Duration);
}

/// The production sink.
///
/// Metrics and log lines are the same observation of the same event, so both carry the metric name
/// and the bounded classes; that is enough to alert on and for a collector to scrape, without this
/// service choosing a metrics backend the deployment has not asked for.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracingTelemetry;

impl Telemetry for TracingTelemetry {
    fn resolve_succeeded(&self, server_key: &str, elapsed: Duration) {
        tracing::info!(
            event = "external_mcp_resolve",
            metric = MCP_RESOLVE_SUCCESS_TOTAL,
            server_key,
            duration_ms = duration_ms(elapsed),
            "External MCP server resolved and published tools"
        );
    }

    fn resolve_failed(&self, server_key: &str, reason: &'static str, elapsed: Duration) {
        tracing::info!(
            event = "external_mcp_resolve",
            metric = MCP_RESOLVE_FAILURE_TOTAL,
            server_key,
            reason,
            duration_ms = duration_ms(elapsed),
            "External MCP server contributed no tools to this admission"
        );
    }

    fn session_tool_cap_exceeded(&self) {
        tracing::info!(
            event = "external_mcp_snapshot_resolved",
            metric = EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL,
            "The aggregate External MCP tool cap was exceeded; the whole snapshot was dropped"
        );
    }

    fn call_limiter_rejected(&self, server_key: &str) {
        tracing::info!(
            event = "external_mcp_call",
            metric = EXTERNAL_MCP_CALL_LIMITER_REJECTED_TOTAL,
            server_key,
            "External MCP call sent no request because its server's concurrency was exhausted"
        );
    }

    fn call_finished(&self, server_key: &str, outcome: CallOutcome, elapsed: Duration) {
        tracing::info!(
            event = "external_mcp_call",
            metric = EXTERNAL_MCP_TOOL_CALLS_TOTAL,
            server_key,
            outcome = outcome.as_str(),
            duration_ms = duration_ms(elapsed),
            "External MCP tool call completed"
        );
    }
}

/// One recorded observation, as a harness sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub metric: &'static str,
    /// The full label set, so a test can assert what a metric may carry and not only that it
    /// fired.
    pub labels: Vec<(&'static str, String)>,
    /// `1` for a counter increment; the millisecond sample for a duration.
    pub value: u64,
}

/// A sink that keeps what it was told.
///
/// It exists so the label contract is testable: the only way to prove a seam cannot carry a
/// credential, a URL or a payload is to look at everything that went through it.
#[derive(Clone, Default)]
pub struct RecordingTelemetry {
    events: Arc<Mutex<Vec<Recorded>>>,
}

impl RecordingTelemetry {
    pub fn recorded(&self) -> Vec<Recorded> {
        self.events
            .lock()
            .expect("the telemetry mailbox is not poisoned")
            .clone()
    }

    pub fn clear(&self) {
        self.events
            .lock()
            .expect("the telemetry mailbox is not poisoned")
            .clear();
    }

    fn push(&self, metric: &'static str, labels: Vec<(&'static str, String)>, value: u64) {
        self.events
            .lock()
            .expect("the telemetry mailbox is not poisoned")
            .push(Recorded {
                metric,
                labels,
                value,
            });
    }
}

impl Telemetry for RecordingTelemetry {
    fn resolve_succeeded(&self, server_key: &str, elapsed: Duration) {
        let labels = resolved(server_key, ResolveOutcome::Success, None);
        self.push(MCP_RESOLVE_SUCCESS_TOTAL, labels.clone(), 1);
        self.push(MCP_RESOLVE_DURATION_MS, labels, duration_ms(elapsed));
    }

    fn resolve_failed(&self, server_key: &str, reason: &'static str, elapsed: Duration) {
        let labels = resolved(server_key, ResolveOutcome::Failure, Some(reason));
        self.push(MCP_RESOLVE_FAILURE_TOTAL, labels.clone(), 1);
        self.push(MCP_RESOLVE_DURATION_MS, labels, duration_ms(elapsed));
    }

    fn session_tool_cap_exceeded(&self) {
        self.push(EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL, Vec::new(), 1);
    }

    fn call_limiter_rejected(&self, server_key: &str) {
        self.push(
            EXTERNAL_MCP_CALL_LIMITER_REJECTED_TOTAL,
            keyed(server_key),
            1,
        );
    }

    fn call_finished(&self, server_key: &str, outcome: CallOutcome, elapsed: Duration) {
        self.push(
            EXTERNAL_MCP_TOOL_CALLS_TOTAL,
            vec![
                ("server_key", server_key.to_owned()),
                ("outcome", outcome.to_string()),
            ],
            1,
        );
        self.push(
            EXTERNAL_MCP_TOOL_CALL_DURATION_MS,
            vec![
                ("server_key", server_key.to_owned()),
                ("outcome", outcome.to_string()),
            ],
            duration_ms(elapsed),
        );
    }
}

fn keyed(server_key: &str) -> Vec<(&'static str, String)> {
    vec![("server_key", server_key.to_owned())]
}

/// The label set every resolve metric carries.  A duration carries the same set as the counter it
/// sits beside, so a slow server can be found from the duration and not only from a failure.
fn resolved(
    server_key: &str,
    outcome: ResolveOutcome,
    reason: Option<&'static str>,
) -> Vec<(&'static str, String)> {
    let mut labels = vec![
        ("server_key", server_key.to_owned()),
        ("outcome", outcome.to_string()),
    ];
    labels.extend(reason.map(|reason| ("reason", reason.to_owned())));
    labels
}

fn duration_ms(elapsed: Duration) -> u64 {
    elapsed.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorder_reports_the_metric_name_its_labels_and_its_value() {
        let sink = RecordingTelemetry::default();
        sink.resolve_succeeded("weather", Duration::from_millis(12));
        sink.resolve_failed("dead", "mcp_server_unavailable", Duration::from_millis(3));
        sink.session_tool_cap_exceeded();
        sink.call_limiter_rejected("busy");
        sink.call_finished(
            "weather",
            CallOutcome::Timeout,
            Duration::from_millis(30_000),
        );

        let recorded = sink.recorded();
        assert_eq!(recorded[0].metric, MCP_RESOLVE_SUCCESS_TOTAL);
        assert_eq!(
            recorded[0].labels,
            vec![
                ("server_key", "weather".to_owned()),
                ("outcome", "success".to_owned())
            ]
        );
        assert_eq!(recorded[1].metric, MCP_RESOLVE_DURATION_MS);
        assert_eq!(recorded[1].value, 12);
        assert_eq!(
            recorded[1].labels, recorded[0].labels,
            "a duration carries the label set of the counter it sits beside"
        );

        assert_eq!(recorded[2].metric, MCP_RESOLVE_FAILURE_TOTAL);
        assert_eq!(
            recorded[2].labels,
            vec![
                ("server_key", "dead".to_owned()),
                ("outcome", "failure".to_owned()),
                ("reason", "mcp_server_unavailable".to_owned())
            ]
        );
        assert_eq!(recorded[3].metric, MCP_RESOLVE_DURATION_MS);
        assert_eq!(recorded[3].labels, recorded[2].labels);

        // The aggregate cap is a snapshot-level event, so it carries no label at all.
        assert_eq!(
            recorded[4].metric,
            EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL
        );
        assert_eq!(recorded[4].labels, Vec::new());

        assert_eq!(recorded[5].metric, EXTERNAL_MCP_CALL_LIMITER_REJECTED_TOTAL);
        assert_eq!(recorded[6].metric, EXTERNAL_MCP_TOOL_CALLS_TOTAL);
        assert_eq!(
            recorded[6].labels,
            vec![
                ("server_key", "weather".to_owned()),
                ("outcome", "timeout".to_owned())
            ]
        );
        assert_eq!(recorded[7].metric, EXTERNAL_MCP_TOOL_CALL_DURATION_MS);
        assert_eq!(recorded[7].value, 30_000);
    }

    #[test]
    fn a_call_outcome_has_exactly_the_bounded_vocabulary() {
        let classes: Vec<&str> = [
            CallOutcome::Success,
            CallOutcome::Timeout,
            CallOutcome::Unavailable,
            CallOutcome::AuthFailed,
            CallOutcome::InvalidResponse,
            CallOutcome::ProtocolError,
        ]
        .iter()
        .map(|outcome| outcome.as_str())
        .collect();
        assert_eq!(
            classes,
            vec![
                "success",
                "timeout",
                "unavailable",
                "auth_failed",
                "invalid_response",
                "protocol_error"
            ]
        );
    }
}
