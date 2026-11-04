use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Canonical severity level ordered from least to most severe (`Trace < Debug < Info < Warn < Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Trace = 1,
    Debug = 2,
    Info = 3,
    Warn = 4,
    Error = 5,
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    pub fn parse_lossy(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "TRACE" => Some(Self::Trace),
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARN" | "WARNING" => Some(Self::Warn),
            "ERROR" | "ERR" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn from_tracing(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }
}

/// Correlation context inherited from enclosing spans or attached directly to an event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrelationContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl CorrelationContext {
    pub fn merge_from_parent(&mut self, parent: &Self) {
        if self.session_id.is_none() {
            self.session_id.clone_from(&parent.session_id);
        }
        if self.turn_id.is_none() {
            self.turn_id = parent.turn_id;
        }
        if self.generation.is_none() {
            self.generation = parent.generation;
        }
        if self.request_id.is_none() {
            self.request_id.clone_from(&parent.request_id);
        }
        if self.agent_key.is_none() {
            self.agent_key.clone_from(&parent.agent_key);
        }
        if self.device_id.is_none() {
            self.device_id = parent.device_id;
        }
        if self.task_id.is_none() {
            self.task_id.clone_from(&parent.task_id);
        }
    }
}

/// One bounded, redacted operational observation stored in the in-memory Log Ring Buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredLogEntry {
    pub seq: u64,
    pub timestamp_ms: i64,
    pub level: LogLevel,
    pub subsystem: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(flatten)]
    pub correlation: CorrelationContext,
    pub fields: BTreeMap<String, Value>,
}

/// Raw unsequenced input constructed by `LogHubLayer` or direct callers before sanitization.
#[derive(Debug, Clone, Default)]
pub struct RawLogRecord {
    pub timestamp_ms: Option<i64>,
    pub level: Option<LogLevel>,
    pub target: String,
    pub subsystem: Option<String>,
    pub event: Option<String>,
    pub message: String,
    pub outcome: Option<String>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub correlation: CorrelationContext,
    pub fields: BTreeMap<String, Value>,
}

/// Shared filter for both snapshot queries (`GET /system/logs`) and SSE streams (`GET /system/logs/stream`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogFilter {
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub min_level: Option<String>,
    #[serde(default)]
    pub subsystem: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<u64>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub agent_key: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLevelSummary {
    pub error: u64,
    pub warn: u64,
    pub info: u64,
    pub debug: u64,
    pub trace: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemLogPage {
    pub items: Vec<StructuredLogEntry>,
    pub oldest_seq: Option<u64>,
    pub latest_seq: u64,
    pub buffered_count: usize,
    pub capacity: usize,
    pub dropped_evicted: u64,
    pub counts: LogLevelSummary,
}
