//! Server Chart Synthesis & Storage Module (`chart_write`, `chart_get`) and Admin HTTP reader.
//!
//! Deep module that hides deterministic OHLCV synthesis from `swings[]` + `pattern` (or explicit
//! `candles[]`), symbolic `PointRef`/`LevelRef` resolution, optimistic concurrency (`revision`),
//! and atomic persistence under `<vault_dir>/charts/<slug>.chart.json`.

mod synthesize;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use orchestrator_agent_rs::{
    HostTool, ToolContext, ToolDescriptor, ToolEffect, ToolError, ToolOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use synthesize::{
    ExplicitCandleInput, SwingSpec, build_candles_from_explicit, resolve_drawing,
    synthesize_candles_from_swings,
};

const MAX_CHART_FILE_BYTES: usize = 256 * 1024;
const MAX_DRAWINGS_PER_CHART: usize = 50;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChartError {
    #[error("{message}")]
    Coded { code: &'static str, message: String },
}

impl ChartError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Coded { code, .. } => code,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartPoint {
    pub time: i64,
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartCandle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ChartDrawing {
    #[serde(rename = "horizontal-line")]
    HorizontalLine {
        id: String,
        price: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tone: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceSpec"
        )]
        source_spec: Option<Value>,
    },
    #[serde(rename = "trend-line")]
    TrendLine {
        id: String,
        points: [ChartPoint; 2],
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tone: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<u8>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceSpec"
        )]
        source_spec: Option<Value>,
    },
    #[serde(rename = "price-zone")]
    PriceZone {
        id: String,
        from: ChartPoint,
        to: ChartPoint,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tone: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceSpec"
        )]
        source_spec: Option<Value>,
    },
    #[serde(rename = "text")]
    Text {
        id: String,
        anchor: ChartPoint,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tone: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceSpec"
        )]
        source_spec: Option<Value>,
    },
}

impl ChartDrawing {
    pub fn id(&self) -> &str {
        match self {
            Self::HorizontalLine { id, .. }
            | Self::TrendLine { id, .. }
            | Self::PriceZone { id, .. }
            | Self::Text { id, .. } => id,
        }
    }

    pub fn source_spec(&self) -> Option<&Value> {
        match self {
            Self::HorizontalLine { source_spec, .. }
            | Self::TrendLine { source_spec, .. }
            | Self::PriceZone { source_spec, .. }
            | Self::Text { source_spec, .. } => source_spec.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChartDisplay {
    pub height: u32,
    pub show_volume: bool,
    pub fit_content: bool,
}

impl Default for ChartDisplay {
    fn default() -> Self {
        Self {
            height: 420,
            show_volume: true,
            fit_content: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartDocument {
    pub schema_version: u32,
    pub revision: u64,
    pub id: String,
    pub title: String,
    pub symbol: String,
    pub timeframe: String,
    pub display: ChartDisplay,
    pub candles: Vec<ChartCandle>,
    pub drawings: Vec<ChartDrawing>,
}

fn validate_chart_path(
    vault_dir: &Path,
    raw_path: &str,
) -> Result<(String, String, PathBuf), ChartError> {
    let trimmed = raw_path.trim().trim_start_matches('/');
    if trimmed.contains("..") || trimmed.contains('\\') {
        return Err(ChartError::Coded {
            code: "invalid_chart_path",
            message: format!("Unsafe chart_path '{raw_path}'"),
        });
    }
    let rel_from_charts = trimmed.strip_prefix("charts/").unwrap_or(trimmed);
    let slug = rel_from_charts
        .strip_suffix(".chart.json")
        .ok_or_else(|| ChartError::Coded {
            code: "invalid_chart_path",
            message: format!("chart_path '{raw_path}' must match 'charts/<slug>.chart.json'"),
        })?;
    if slug.is_empty()
        || slug.len() > 96
        || !slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ChartError::Coded {
            code: "invalid_chart_path",
            message: format!("Chart slug '{slug}' must match ^[a-zA-Z0-9_-]{{1,96}}$"),
        });
    }

    let canonical_rel = format!("charts/{slug}.chart.json");
    let full_path = vault_dir.join("charts").join(format!("{slug}.chart.json"));
    Ok((slug.to_string(), canonical_rel, full_path))
}

fn load_existing_chart(full_path: &Path) -> Result<Option<ChartDocument>, ChartError> {
    if !full_path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(full_path).map_err(|e| ChartError::Coded {
        code: "io_error",
        message: format!("Failed to read existing chart: {e}"),
    })?;
    if bytes.len() > MAX_CHART_FILE_BYTES {
        return Err(ChartError::Coded {
            code: "chart_too_large",
            message: "Existing chart file exceeds 256 KB limit".to_string(),
        });
    }
    let doc: ChartDocument = serde_json::from_slice(&bytes).map_err(|e| ChartError::Coded {
        code: "invalid_chart_json",
        message: format!("Failed to parse existing chart JSON: {e}"),
    })?;
    Ok(Some(doc))
}

fn save_chart_atomic(full_path: &Path, doc: &ChartDocument) -> Result<(), ChartError> {
    let Some(parent) = full_path.parent() else {
        return Err(ChartError::Coded {
            code: "invalid_chart_path",
            message: "Missing parent directory for chart file".to_string(),
        });
    };
    std::fs::create_dir_all(parent).map_err(|e| ChartError::Coded {
        code: "io_error",
        message: format!("Failed to create charts directory: {e}"),
    })?;
    let serialized = serde_json::to_vec_pretty(doc).map_err(|e| ChartError::Coded {
        code: "serialize_error",
        message: format!("Failed to serialize chart JSON: {e}"),
    })?;
    if serialized.len() > MAX_CHART_FILE_BYTES {
        return Err(ChartError::Coded {
            code: "chart_too_large",
            message: format!(
                "Serialized chart ({} bytes) exceeds 256 KB limit",
                serialized.len()
            ),
        });
    }
    let tmp_path = full_path.with_extension("json.part");
    std::fs::write(&tmp_path, &serialized).map_err(|e| ChartError::Coded {
        code: "io_error",
        message: format!("Failed to write temporary chart file: {e}"),
    })?;
    std::fs::rename(&tmp_path, full_path).map_err(|e| ChartError::Coded {
        code: "io_error",
        message: format!("Failed to rename temporary chart file: {e}"),
    })?;
    Ok(())
}

fn build_tagged_candles_summary(candles: &[ChartCandle]) -> Value {
    let mut map = Map::new();
    for (idx, c) in candles.iter().enumerate() {
        if let Some(ref tag) = c.tag {
            map.insert(
                tag.clone(),
                json!({
                    "bar": idx,
                    "time": c.time,
                    "open": c.open,
                    "high": c.high,
                    "low": c.low,
                    "close": c.close,
                }),
            );
        }
    }
    Value::Object(map)
}

fn parse_display(raw: Option<&Value>, fallback: ChartDisplay) -> ChartDisplay {
    let Some(obj) = raw.and_then(Value::as_object) else {
        return fallback;
    };
    let height = obj
        .get("height")
        .and_then(Value::as_u64)
        .map_or(fallback.height, |h| h.clamp(240, 720) as u32);
    let show_volume = obj
        .get("showVolume")
        .or_else(|| obj.get("show_volume"))
        .and_then(Value::as_bool)
        .unwrap_or(fallback.show_volume);
    let fit_content = obj
        .get("fitContent")
        .or_else(|| obj.get("fit_content"))
        .and_then(Value::as_bool)
        .unwrap_or(fallback.fit_content);
    ChartDisplay {
        height,
        show_volume,
        fit_content,
    }
}

pub fn execute_chart_write(vault_dir: &Path, args: Value) -> Result<Value, ChartError> {
    let chart_path = args
        .get("chart_path")
        .and_then(Value::as_str)
        .ok_or_else(|| ChartError::Coded {
            code: "invalid_chart_path",
            message: "Missing required string parameter 'chart_path'".to_string(),
        })?;
    let (chart_id, canonical_rel, full_path) = validate_chart_path(vault_dir, chart_path)?;
    let existing = load_existing_chart(&full_path)?;

    if let Some(expected_rev) = args.get("expected_revision").and_then(Value::as_u64) {
        let actual_rev = existing.as_ref().map_or(0, |d| d.revision);
        if expected_rev != actual_rev {
            return Err(ChartError::Coded {
                code: "revision_conflict",
                message: format!(
                    "expected_revision ({expected_rev}) does not match current revision ({actual_rev})"
                ),
            });
        }
    }

    let has_swings = args.get("swings").is_some_and(|v| !v.is_null());
    let has_candles = args.get("candles").is_some_and(|v| !v.is_null());
    if has_swings && has_candles {
        return Err(ChartError::Coded {
            code: "conflicting_candle_sources",
            message: "Provide either 'swings' OR 'candles', never both".to_string(),
        });
    }

    let timeframe = args
        .get("timeframe")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| existing.as_ref().map(|d| d.timeframe.as_str()))
        .unwrap_or("1h")
        .to_string();

    let candles = if has_swings {
        let swings: Vec<SwingSpec> =
            serde_json::from_value(args["swings"].clone()).map_err(|e| ChartError::Coded {
                code: "invalid_swings",
                message: format!("Invalid 'swings' array: {e}"),
            })?;
        synthesize_candles_from_swings(&chart_id, &swings, &timeframe)?
    } else if has_candles {
        let explicit: Vec<ExplicitCandleInput> = serde_json::from_value(args["candles"].clone())
            .map_err(|e| ChartError::Coded {
                code: "invalid_candles",
                message: format!("Invalid 'candles' array: {e}"),
            })?;
        build_candles_from_explicit(&explicit, &timeframe)?
    } else if let Some(ref doc) = existing {
        doc.candles.clone()
    } else {
        return Err(ChartError::Coded {
            code: "missing_candle_source",
            message: "Creating a new chart requires either 'swings' or 'candles'".to_string(),
        });
    };

    let replace_all = args
        .get("replace_all_drawings")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut drawings = if replace_all {
        Vec::new()
    } else {
        existing
            .as_ref()
            .map_or_else(Vec::new, |d| d.drawings.clone())
    };

    if let Some(remove_ids) = args.get("remove_drawing_ids").and_then(Value::as_array) {
        let remove_set: HashSet<&str> = remove_ids.iter().filter_map(Value::as_str).collect();
        drawings.retain(|d| !remove_set.contains(d.id()));
    }

    // If candles were regenerated (`has_swings || has_candles`), re-resolve retained drawings against the new candles
    if (has_swings || has_candles) && !drawings.is_empty() {
        let mut reresolved = Vec::with_capacity(drawings.len());
        for (idx, existing_d) in drawings.into_iter().enumerate() {
            if let Some(spec) = existing_d.source_spec().cloned() {
                reresolved.push(resolve_drawing(&candles, &spec, idx)?);
            } else {
                reresolved.push(existing_d);
            }
        }
        drawings = reresolved;
    }

    if let Some(upserts) = args.get("upsert_drawings").and_then(Value::as_array) {
        for (idx, raw_d) in upserts.iter().enumerate() {
            let resolved = resolve_drawing(&candles, raw_d, idx)?;
            if let Some(pos) = drawings.iter().position(|d| d.id() == resolved.id()) {
                drawings[pos] = resolved;
            } else {
                drawings.push(resolved);
            }
        }
    }

    if drawings.len() > MAX_DRAWINGS_PER_CHART {
        return Err(ChartError::Coded {
            code: "too_many_drawings",
            message: format!(
                "Chart has {} drawings, exceeding the maximum of {MAX_DRAWINGS_PER_CHART}",
                drawings.len()
            ),
        });
    }

    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| existing.as_ref().map(|d| d.title.clone()))
        .unwrap_or_else(|| chart_id.clone());

    let symbol = args
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| existing.as_ref().map(|d| d.symbol.clone()))
        .unwrap_or_else(|| "PA-EXAMPLE".to_string());

    let display = parse_display(
        args.get("display"),
        existing
            .as_ref()
            .map_or_else(ChartDisplay::default, |d| d.display.clone()),
    );
    let next_revision = existing.as_ref().map_or(1, |d| d.revision + 1);

    let doc = ChartDocument {
        schema_version: 1,
        revision: next_revision,
        id: chart_id.clone(),
        title,
        symbol,
        timeframe,
        display,
        candles,
        drawings,
    };

    save_chart_atomic(&full_path, &doc)?;

    let tagged_candles = build_tagged_candles_summary(&doc.candles);
    let markdown_fence = format!("```trading-chart\n{{\"src\":\"{canonical_rel}\"}}\n```");

    Ok(json!({
        "chartId": doc.id,
        "chartPath": canonical_rel,
        "revision": doc.revision,
        "candleCount": doc.candles.len(),
        "taggedCandles": tagged_candles,
        "drawingCount": doc.drawings.len(),
        "markdownFence": markdown_fence,
    }))
}

pub fn execute_chart_get(vault_dir: &Path, args: Value) -> Result<Value, ChartError> {
    let chart_path = args
        .get("chart_path")
        .and_then(Value::as_str)
        .ok_or_else(|| ChartError::Coded {
            code: "invalid_chart_path",
            message: "Missing required string parameter 'chart_path'".to_string(),
        })?;
    let (_, canonical_rel, full_path) = validate_chart_path(vault_dir, chart_path)?;
    let doc = load_existing_chart(&full_path)?.ok_or_else(|| ChartError::Coded {
        code: "chart_not_found",
        message: format!("Chart '{canonical_rel}' does not exist"),
    })?;

    let first_bar_time = doc.candles.first().map_or(0, |c| c.time);
    let last_bar_time = doc.candles.last().map_or(0, |c| c.time);
    let min_low = doc
        .candles
        .iter()
        .map(|c| c.low)
        .fold(f64::INFINITY, f64::min);
    let max_high = doc
        .candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let tagged_candles = build_tagged_candles_summary(&doc.candles);
    let markdown_fence = format!("```trading-chart\n{{\"src\":\"{canonical_rel}\"}}\n```");

    Ok(json!({
        "chart": doc,
        "revision": doc.revision,
        "summary": {
            "candleCount": doc.candles.len(),
            "firstBarTime": first_bar_time,
            "lastBarTime": last_bar_time,
            "minLow": min_low,
            "maxHigh": max_high,
            "taggedCandles": tagged_candles,
        },
        "markdownFence": markdown_fence,
    }))
}

pub fn read_chart_json_for_admin(vault_dir: &Path, rel_path: &str) -> Result<Value, ChartError> {
    let (_, canonical_rel, full_path) = validate_chart_path(vault_dir, rel_path)?;
    let doc = load_existing_chart(&full_path)?.ok_or_else(|| ChartError::Coded {
        code: "chart_not_found",
        message: format!("Chart '{canonical_rel}' not found"),
    })?;
    serde_json::to_value(doc).map_err(|e| ChartError::Coded {
        code: "serialize_error",
        message: format!("Failed to serialize chart document: {e}"),
    })
}

pub struct ChartWriteTool {
    pub vault_dir: PathBuf,
}

impl ChartWriteTool {
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: vault_dir.into(),
        }
    }
}

#[async_trait::async_trait]
impl HostTool for ChartWriteTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "chart_write".to_string(),
            description: "Synthesize or update a self-contained trading concept chart (`charts/<slug>.chart.json`) from macro `swings` XOR explicit `candles` (or update drawings on an existing chart) and return `taggedCandles` plus the `markdownFence` snippet.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["chart_path"],
                "properties": {
                    "chart_path": {
                        "type": "string",
                        "description": "Target chart path under charts/, e.g. 'charts/bullish-ob-fvg-example.chart.json'."
                    },
                    "title": {
                        "type": "string",
                        "description": "Educational title shown in the chart header."
                    },
                    "symbol": {
                        "type": "string",
                        "description": "Illustrative symbol label (default 'PA-EXAMPLE')."
                    },
                    "timeframe": {
                        "type": "string",
                        "description": "Illustrative timeframe ('5m', '15m', '1h', '4h', '1d'; default '1h')."
                    },
                    "swings": {
                        "type": "array",
                        "description": "Ordered turning points [{'tag', 'price', 'bars'?, 'impulse'?, 'pattern'?}] for macro structure synthesis (mutually exclusive with 'candles')."
                    },
                    "candles": {
                        "type": "array",
                        "description": "Explicit OHLC bars [{'open', 'high', 'low', 'close', 'volume'?, 'tag'?}] for micro pattern anatomy (mutually exclusive with 'swings')."
                    },
                    "upsert_drawings": {
                        "type": "array",
                        "description": "Annotations to add or update by id ('horizontal-line', 'trend-line', 'price-zone', 'text') using PointRef/LevelRef and semantic 'tone'."
                    },
                    "remove_drawing_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Drawing ids to remove."
                    },
                    "replace_all_drawings": {
                        "type": "boolean",
                        "description": "If true, clear existing drawings before applying upsert_drawings."
                    },
                    "expected_revision": {
                        "type": "integer",
                        "description": "Optional optimistic concurrency revision check."
                    },
                    "display": {
                        "type": "object",
                        "description": "Optional display preferences {'height': 240..720, 'showVolume': bool, 'fitContent': bool}."
                    }
                }
            }),
            effect: ToolEffect::Mutating,
        }
    }

    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        let result = execute_chart_write(&self.vault_dir, args).map_err(|err| ToolError {
            code: err.code().to_string(),
            message: err.to_string(),
        })?;
        let content = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
        Ok(ToolOutcome {
            call_id: ctx.scope().tool_call_id.clone(),
            content,
            metadata: result,
        })
    }
}

pub struct ChartGetTool {
    pub vault_dir: PathBuf,
}

impl ChartGetTool {
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: vault_dir.into(),
        }
    }
}

#[async_trait::async_trait]
impl HostTool for ChartGetTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "chart_get".to_string(),
            description: "Read back an existing trading chart document (`charts/<slug>.chart.json`) along with its `revision`, `taggedCandles` summary, price range, and `markdownFence`.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["chart_path"],
                "properties": {
                    "chart_path": {
                        "type": "string",
                        "description": "Chart path under charts/, e.g. 'charts/bullish-ob-fvg-example.chart.json'."
                    }
                },
                "additionalProperties": false
            }),
            effect: ToolEffect::ReadOnly,
        }
    }

    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        let result = execute_chart_get(&self.vault_dir, args).map_err(|err| ToolError {
            code: err.code().to_string(),
            message: err.to_string(),
        })?;
        let content = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
        Ok(ToolOutcome {
            call_id: ctx.scope().tool_call_id.clone(),
            content,
            metadata: result,
        })
    }
}

pub fn chart_host_tools(vault_dir: impl Into<PathBuf>) -> Vec<Arc<dyn HostTool>> {
    let dir = vault_dir.into();
    vec![
        Arc::new(ChartWriteTool::new(dir.clone())),
        Arc::new(ChartGetTool::new(dir)),
    ]
}
