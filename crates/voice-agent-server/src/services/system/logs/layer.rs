use super::{
    SystemLogHub,
    types::{CorrelationContext, LogLevel, RawLogRecord},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt, sync::Arc};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

#[derive(Debug, Clone, Default)]
struct SpanMetadata {
    subsystem: Option<String>,
    correlation: CorrelationContext,
    fields: BTreeMap<String, Value>,
}

struct FieldVisitor {
    subsystem: Option<String>,
    event_name: Option<String>,
    message: Option<String>,
    outcome: Option<String>,
    duration_ms: Option<u64>,
    error_code: Option<String>,
    correlation: CorrelationContext,
    fields: BTreeMap<String, Value>,
}

impl Default for FieldVisitor {
    fn default() -> Self {
        Self {
            subsystem: None,
            event_name: None,
            message: None,
            outcome: None,
            duration_ms: None,
            error_code: None,
            correlation: CorrelationContext::default(),
            fields: BTreeMap::new(),
        }
    }
}

fn clean_debug_str(raw: String) -> String {
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        raw[1..raw.len() - 1].to_owned()
    } else if let Some(inner) = raw.strip_prefix("Some(").and_then(|s| s.strip_suffix(')')) {
        clean_debug_str(inner.to_owned())
    } else {
        raw
    }
}

impl FieldVisitor {
    fn try_record_numeric_field(&mut self, name: &str, value_u64: Option<u64>, value_i64: Option<i64>) -> bool {
        match name {
            "turn_id" => {
                if let Some(v) = value_u64.or_else(|| value_i64.filter(|&x| x >= 0).map(|x| x as u64)) {
                    self.correlation.turn_id = Some(v);
                    return true;
                }
            }
            "generation" => {
                if let Some(v) = value_u64.or_else(|| value_i64.filter(|&x| x >= 0).map(|x| x as u64)) {
                    self.correlation.generation = Some(v);
                    return true;
                }
            }
            "device_id" | "device_row_id" => {
                if let Some(v) = value_i64.or_else(|| value_u64.map(|x| x as i64)) {
                    self.correlation.device_id = Some(v);
                    return true;
                }
            }
            "duration_ms" | "elapsed_ms" => {
                if let Some(v) = value_u64.or_else(|| value_i64.filter(|&x| x >= 0).map(|x| x as u64)) {
                    self.duration_ms = Some(v);
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn record_str_field(&mut self, name: &str, value: String) {
        if value == "None" {
            return;
        }
        match name {
            "message" => self.message = Some(value),
            "subsystem" => self.subsystem = Some(value),
            "event" => self.event_name = Some(value),
            "outcome" => self.outcome = Some(value),
            "error_code" | "reason" | "reason_code" | "code" | "error" | "cause" => {
                if self.error_code.is_none() {
                    self.error_code = Some(value.clone());
                }
                if name != "error_code" {
                    self.fields.insert(name.to_owned(), Value::String(value));
                }
            }
            "session_id" => self.correlation.session_id = Some(value),
            "request_id" => self.correlation.request_id = Some(value),
            "agent_key" => self.correlation.agent_key = Some(value),
            "task_id" | "job_id" => self.correlation.task_id = Some(value),
            _ => {
                let parsed_u64 = value.parse::<u64>().ok();
                let parsed_i64 = value.parse::<i64>().ok();
                if !self.try_record_numeric_field(name, parsed_u64, parsed_i64) {
                    self.fields.insert(name.to_owned(), Value::String(value));
                }
            }
        }
    }

    fn record_u64_field(&mut self, name: &str, value: u64) {
        if !self.try_record_numeric_field(name, Some(value), None) {
            self.fields.insert(name.to_owned(), json!(value));
        }
    }

    fn record_i64_field(&mut self, name: &str, value: i64) {
        if !self.try_record_numeric_field(name, None, Some(value)) {
            self.fields.insert(name.to_owned(), json!(value));
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_str_field(field.name(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_u64_field(field.name(), value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_i64_field(field.name(), value);
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        let clamped = value.min(u128::from(u64::MAX)) as u64;
        self.record_u64_field(field.name(), clamped);
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        let clamped = value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        self.record_i64_field(field.name(), clamped);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_owned(), json!(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record_str_field(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let formatted = clean_debug_str(format!("{value:?}"));
        self.record_str_field(field.name(), formatted);
    }
}

/// Adapter at the `tracing_subscriber::Layer` seam that extracts Span correlation and Event fields
/// and forwards them into [`SystemLogHub`].
#[derive(Clone)]
pub struct LogHubLayer {
    hub: Arc<SystemLogHub>,
}

impl LogHubLayer {
    pub fn new(hub: Arc<SystemLogHub>) -> Self {
        Self { hub }
    }
}

impl<S> Layer<S> for LogHubLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);

        let mut meta = SpanMetadata {
            subsystem: visitor.subsystem,
            correlation: visitor.correlation,
            fields: visitor.fields,
        };
        if let Some(parent) = span.parent()
            && let Some(parent_meta) = parent.extensions().get::<SpanMetadata>()
        {
            meta.correlation.merge_from_parent(&parent_meta.correlation);
            if meta.subsystem.is_none() {
                meta.subsystem.clone_from(&parent_meta.subsystem);
            }
            for (k, v) in &parent_meta.fields {
                meta.fields.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        span.extensions_mut().insert(meta);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);

        let mut extensions = span.extensions_mut();
        if let Some(meta) = extensions.get_mut::<SpanMetadata>() {
            let mut updated = visitor.correlation;
            updated.merge_from_parent(&meta.correlation);
            meta.correlation = updated;
            if visitor.subsystem.is_some() {
                meta.subsystem = visitor.subsystem;
            }
            for (k, v) in visitor.fields {
                meta.fields.insert(k, v);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let mut correlation = visitor.correlation;
        let mut inherited_subsystem = visitor.subsystem;
        let mut fields = visitor.fields;

        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                if let Some(span_meta) = span.extensions().get::<SpanMetadata>() {
                    correlation.merge_from_parent(&span_meta.correlation);
                    if inherited_subsystem.is_none() {
                        inherited_subsystem.clone_from(&span_meta.subsystem);
                    }
                    for (k, v) in &span_meta.fields {
                        fields.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }

        let meta = event.metadata();
        let message = visitor
            .message
            .or_else(|| visitor.event_name.clone())
            .unwrap_or_else(|| meta.name().to_owned());

        let raw = RawLogRecord {
            timestamp_ms: None,
            level: Some(LogLevel::from_tracing(meta.level())),
            target: meta.target().to_owned(),
            subsystem: inherited_subsystem,
            event: visitor.event_name,
            message,
            outcome: visitor.outcome,
            duration_ms: visitor.duration_ms,
            error_code: visitor.error_code,
            correlation,
            fields,
        };
        let _ = self.hub.record(raw);
    }
}
