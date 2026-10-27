use super::types::{
    CorrelationContext, LogFilter, LogLevel, RawLogRecord, StructuredLogEntry,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MAX_MESSAGE_BYTES: usize = 1_024;
pub const MAX_FIELD_STRING_BYTES: usize = 512;
pub const MAX_FIELDS_PER_ENTRY: usize = 24;

pub(super) fn classify_subsystem(explicit: Option<&str>, target: &str) -> String {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return match raw {
            "voice.ws" | "voice.session" => "voice.ws".to_owned(),
            "voice.vad" => "voice.vad".to_owned(),
            "voice.asr" => "voice.asr".to_owned(),
            "voice.speaker" => "voice.speaker".to_owned(),
            "voice.llm" => "voice.llm".to_owned(),
            "voice.tts" => "voice.tts".to_owned(),
            "mcp.device" => "mcp.device".to_owned(),
            "mcp.external" | "mcp.system" | "mcp" => "mcp.external".to_owned(),
            "task_flow" | "knowledge" | "memory" => "task_flow".to_owned(),
            "notification" | "notification.hub" => "notification".to_owned(),
            "admin.api" | "admin.audit" => "admin.api".to_owned(),
            "system" | "provider" | "provider.runtime" | "database" => "system".to_owned(),
            other => other.to_owned(),
        };
    }
    if target.contains("::app::transport::websocket") || target.contains("::app::enrollment") {
        "voice.ws".to_owned()
    } else if target.contains("::workers::vad")
        || target.contains("::providers::vad")
        || target.contains("::audio::vad_segmenter")
        || target.contains("::session::actor::listening")
    {
        "voice.vad".to_owned()
    } else if target.contains("::workers::asr") || target.contains("::providers::asr") {
        "voice.asr".to_owned()
    } else if target.contains("::workers::speaker")
        || target.contains("::providers::speaker")
        || target.contains("::session::actor::observe")
        || target.contains("::session::actor::speaker")
        || target.contains("::session::speaker")
    {
        "voice.speaker".to_owned()
    } else if target.contains("::workers::llm")
        || target.contains("::providers::llm")
        || target.contains("::session::actor::delivery")
        || target.contains("::session::interaction")
    {
        "voice.llm".to_owned()
    } else if target.contains("::workers::tts")
        || target.contains("::providers::tts")
        || target.contains("::session::speech_output")
    {
        "voice.tts".to_owned()
    } else if target.contains("::tools::device_mcp")
        || target.contains("::session::actor::mcp")
        || target.contains("::app::transport::vision")
    {
        "mcp.device".to_owned()
    } else if target.contains("::tools::external_mcp")
        || target.contains("::telemetry")
        || target.contains("::tools::system_mcp")
        || target.contains("::tools::knowledge_mcp")
        || target.contains("::app::mcp_server")
        || target.contains("::tools::round")
        || target.contains("::session::actor::tools")
    {
        "mcp.external".to_owned()
    } else if target.contains("::services::task_flow")
        || target.contains("::app::admin::background")
        || target.contains("::app::admin::knowledge")
        || target.starts_with("orchestrator_agent")
        || target.starts_with("orchestrator_flow")
        || target.starts_with("llm_wiki")
    {
        "task_flow".to_owned()
    } else if target.contains("::services::notification_hub")
        || target.contains("::services::notifications")
        || target.contains("::database::notifications")
        || target.contains("::app::admin::notification")
    {
        "notification".to_owned()
    } else if target.contains("::app::admin") || target.starts_with("tower_http") {
        "admin.api".to_owned()
    } else {
        "system".to_owned()
    }
}

pub(super) fn is_sensitive_key(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    if lower.ends_with("_tokens")
        || lower == "tokens"
        || lower.starts_with("has_")
        || lower.ends_with("_enabled")
    {
        return false;
    }
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "authorization",
        "credential",
        "private_key",
        "cookie",
        "cipher",
        "ws_ticket",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn truncate_utf8(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    let boundary = input.floor_char_boundary(max_bytes);
    let mut out = String::with_capacity(boundary + 16);
    out.push_str(&input[..boundary]);
    out.push_str("…[truncated]");
    out
}

pub(super) fn scrub_string_value(raw: &str, max_bytes: usize) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("bearer ")
        || lower.starts_with("sealed:")
        || lower.starts_with("sk-")
    {
        return "[REDACTED]".to_owned();
    }
    truncate_utf8(raw, max_bytes)
}

pub(super) fn scrub_field_value(key: &str, value: Value) -> Value {
    if is_sensitive_key(key) {
        return Value::String("[REDACTED]".to_owned());
    }
    match value {
        Value::String(s) => Value::String(scrub_string_value(&s, MAX_FIELD_STRING_BYTES)),
        other => other,
    }
}

pub(super) fn is_self_observation_noise(raw: &RawLogRecord) -> bool {
    let level = raw.level.unwrap_or(LogLevel::Info);
    if level >= LogLevel::Warn {
        return false;
    }
    let path = raw
        .fields
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("");
    matches!(
        path,
        "/health" | "/ready" | "/api/admin/system" | "/api/admin/system/resources"
    ) || path.starts_with("/api/admin/system/logs")
}

pub(super) fn normalize_entry(seq: u64, mut raw: RawLogRecord) -> StructuredLogEntry {
    let timestamp_ms = raw.timestamp_ms.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    });
    let level = raw.level.unwrap_or(LogLevel::Info);
    let subsystem = classify_subsystem(raw.subsystem.as_deref(), &raw.target);
    let message = scrub_string_value(&raw.message, MAX_MESSAGE_BYTES);

    let mut sanitized_fields = BTreeMap::new();
    for (key, val) in std::mem::take(&mut raw.fields) {
        if sanitized_fields.len() >= MAX_FIELDS_PER_ENTRY {
            break;
        }
        sanitized_fields.insert(key.clone(), scrub_field_value(&key, val));
    }

    StructuredLogEntry {
        seq,
        timestamp_ms,
        level,
        subsystem,
        target: truncate_utf8(&raw.target, 256),
        event: raw.event.map(|e| truncate_utf8(&e, 128)),
        message,
        outcome: raw.outcome.map(|o| truncate_utf8(&o, 64)),
        duration_ms: raw.duration_ms,
        error_code: raw.error_code.map(|c| scrub_string_value(&c, 128)),
        correlation: CorrelationContext {
            session_id: raw.correlation.session_id.map(|s| truncate_utf8(&s, 128)),
            turn_id: raw.correlation.turn_id,
            generation: raw.correlation.generation,
            request_id: raw.correlation.request_id.map(|s| truncate_utf8(&s, 128)),
            agent_key: raw.correlation.agent_key.map(|s| truncate_utf8(&s, 64)),
            device_id: raw.correlation.device_id,
            task_id: raw.correlation.task_id.map(|s| truncate_utf8(&s, 128)),
        },
        fields: sanitized_fields,
    }
}

pub(super) struct ParsedFilter {
    pub after_seq: Option<u64>,
    pub min_level: Option<LogLevel>,
    pub subsystems: Vec<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<u64>,
    pub request_id: Option<String>,
    pub agent_key: Option<String>,
    pub task_id: Option<String>,
    pub outcome: Option<String>,
    pub q_lower: Option<String>,
    pub limit: usize,
}

impl ParsedFilter {
    pub fn parse(filter: &LogFilter) -> Result<Self, &'static str> {
        let min_level = match filter
            .min_level
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Some(LogLevel::parse_lossy(raw).ok_or("invalid_query")?),
            None => None,
        };
        let limit = match filter.limit {
            Some(0) => return Err("invalid_query"),
            Some(n) if n > 500 => return Err("invalid_query"),
            Some(n) => n,
            None => 200,
        };
        let subsystems: Vec<String> = filter
            .subsystem
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        let clean = |opt: &Option<String>| {
            opt.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Ok(Self {
            after_seq: filter.after_seq,
            min_level,
            subsystems,
            session_id: clean(&filter.session_id),
            turn_id: filter.turn_id,
            request_id: clean(&filter.request_id),
            agent_key: clean(&filter.agent_key),
            task_id: clean(&filter.task_id),
            outcome: clean(&filter.outcome),
            q_lower: clean(&filter.q).map(|q| q.to_ascii_lowercase()),
            limit,
        })
    }

    pub fn matches(&self, entry: &StructuredLogEntry) -> bool {
        if let Some(after) = self.after_seq
            && entry.seq <= after
        {
            return false;
        }
        if let Some(min) = self.min_level
            && entry.level < min
        {
            return false;
        }
        if !self.subsystems.is_empty() {
            let sub_lower = entry.subsystem.to_ascii_lowercase();
            if !self
                .subsystems
                .iter()
                .any(|f| sub_lower == *f || sub_lower.starts_with(&format!("{f}.")))
            {
                return false;
            }
        }
        if let Some(ref sid) = self.session_id
            && entry.correlation.session_id.as_deref() != Some(sid.as_str())
        {
            return false;
        }
        if let Some(tid) = self.turn_id
            && entry.correlation.turn_id != Some(tid)
        {
            return false;
        }
        if let Some(ref rid) = self.request_id
            && entry.correlation.request_id.as_deref() != Some(rid.as_str())
        {
            return false;
        }
        if let Some(ref ak) = self.agent_key
            && entry.correlation.agent_key.as_deref() != Some(ak.as_str())
        {
            return false;
        }
        if let Some(ref tk) = self.task_id
            && entry.correlation.task_id.as_deref() != Some(tk.as_str())
        {
            return false;
        }
        if let Some(ref out) = self.outcome
            && entry.outcome.as_deref() != Some(out.as_str())
        {
            return false;
        }
        if let Some(ref q) = self.q_lower {
            let in_msg = entry.message.to_ascii_lowercase().contains(q);
            let in_ev = entry
                .event
                .as_ref()
                .is_some_and(|e| e.to_ascii_lowercase().contains(q));
            let in_target = entry.target.to_ascii_lowercase().contains(q);
            let in_err = entry
                .error_code
                .as_ref()
                .is_some_and(|c| c.to_ascii_lowercase().contains(q));
            let in_fields = entry
                .fields
                .iter()
                .any(|(k, v)| k.to_ascii_lowercase().contains(q) || v.to_string().to_ascii_lowercase().contains(q));
            if !(in_msg || in_ev || in_target || in_err || in_fields) {
                return false;
            }
        }
        true
    }
}
