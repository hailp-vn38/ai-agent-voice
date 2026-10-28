//! Deterministic OHLCV candle synthesis (`swings[]` XOR `candles[]`) and symbolic
//! coordinate resolution (`PointRef` / `LevelRef` -> `{ time, price }`).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{ChartCandle, ChartDrawing, ChartError, ChartPoint};

const BASE_EPOCH_SECS: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwingSpec {
    #[serde(default)]
    pub tag: Option<String>,
    pub price: f64,
    #[serde(default)]
    pub bars: Option<usize>,
    #[serde(default)]
    pub impulse: Option<bool>,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExplicitCandleInput {
    #[serde(default)]
    pub time: Option<i64>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub tag: Option<String>,
}

pub fn timeframe_step_secs(timeframe: &str) -> i64 {
    match timeframe.trim() {
        "1m" => 60,
        "3m" => 180,
        "5m" => 300,
        "15m" => 900,
        "30m" => 1800,
        "1h" | "60m" => 3600,
        "2h" => 7200,
        "4h" => 14_400,
        "1d" | "D" | "1D" => 86_400,
        "1w" | "W" | "1W" => 604_800,
        _ => 3600,
    }
}

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn from_seed(chart_id: &str, leg_index: usize) -> Self {
        let digest = Sha256::digest(format!("{chart_id}:{leg_index}").as_bytes());
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        let seed = u64::from_le_bytes(bytes);
        Self {
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a deterministic float in `[0.0, 1.0)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / ((1u64 << 53) as f64))
    }
}

fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

fn validate_tag_str(tag: &str, seen: &mut HashSet<String>) -> Result<String, ChartError> {
    let trimmed = tag.trim();
    if trimmed.is_empty()
        || trimmed.len() > 40
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ChartError::Coded {
            code: "invalid_candle_tag",
            message: format!(
                "Candle tag '{trimmed}' must match ^[a-zA-Z0-9_-]{{1,40}}$"
            ),
        });
    }
    if !seen.insert(trimmed.to_string()) {
        return Err(ChartError::Coded {
            code: "duplicate_candle_tag",
            message: format!("Duplicate candle tag '{trimmed}'"),
        });
    }
    Ok(trimmed.to_string())
}

pub fn build_candles_from_explicit(
    inputs: &[ExplicitCandleInput],
    timeframe: &str,
) -> Result<Vec<ChartCandle>, ChartError> {
    if inputs.len() < 3 || inputs.len() > 500 {
        return Err(ChartError::Coded {
            code: "invalid_candle_count",
            message: format!(
                "Explicit candles count ({}) must be between 3 and 500",
                inputs.len()
            ),
        });
    }
    let step = timeframe_step_secs(timeframe);
    let mut seen_tags = HashSet::new();
    let mut out = Vec::with_capacity(inputs.len());
    let mut prev_time = i64::MIN;

    for (idx, item) in inputs.iter().enumerate() {
        if !item.open.is_finite()
            || !item.high.is_finite()
            || !item.low.is_finite()
            || !item.close.is_finite()
        {
            return Err(ChartError::Coded {
                code: "invalid_candle_ohlc",
                message: format!("Candle at index {idx} has non-finite OHLC values"),
            });
        }
        let max_body = item.open.max(item.close);
        let min_body = item.open.min(item.close);
        if item.high + 1e-9 < max_body || item.low - 1e-9 > min_body || item.high + 1e-9 < item.low
        {
            return Err(ChartError::Coded {
                code: "invalid_candle_ohlc",
                message: format!(
                    "Candle at index {idx} violates OHLC bounds (high >= max(open, close), low <= min(open, close))"
                ),
            });
        }
        let time = item
            .time
            .unwrap_or_else(|| BASE_EPOCH_SECS + (idx as i64) * step);
        if time <= prev_time {
            return Err(ChartError::Coded {
                code: "invalid_candle_time",
                message: format!("Candle time at index {idx} must be strictly ascending"),
            });
        }
        prev_time = time;
        let tag = match item.tag.as_deref() {
            Some(raw) if !raw.trim().is_empty() => Some(validate_tag_str(raw, &mut seen_tags)?),
            _ => None,
        };
        let volume = match item.volume {
            Some(v) if v.is_finite() && v >= 0.0 => Some(round2(v)),
            Some(_) => {
                return Err(ChartError::Coded {
                    code: "invalid_candle_volume",
                    message: format!("Candle at index {idx} has negative or non-finite volume"),
                });
            }
            None => Some(round2(150.0 + ((idx % 7) as f64) * 35.0)),
        };

        out.push(ChartCandle {
            time,
            open: item.open,
            high: item.high,
            low: item.low,
            close: item.close,
            volume,
            tag,
        });
    }

    Ok(out)
}

#[allow(clippy::too_many_lines)]
pub fn synthesize_candles_from_swings(
    chart_id: &str,
    swings: &[SwingSpec],
    timeframe: &str,
) -> Result<Vec<ChartCandle>, ChartError> {
    if swings.len() < 2 {
        return Err(ChartError::Coded {
            code: "invalid_swings",
            message: "swings must contain at least 2 turning points".to_string(),
        });
    }
    for (idx, s) in swings.iter().enumerate() {
        if !s.price.is_finite() || s.price <= 0.0 {
            return Err(ChartError::Coded {
                code: "invalid_swing_price",
                message: format!("Swing at index {idx} must have a finite positive price"),
            });
        }
    }

    let step = timeframe_step_secs(timeframe);
    let mut seen_tags = HashSet::new();
    let mut candles: Vec<ChartCandle> = Vec::new();

    // Anchor bar 0 at swings[0]
    let first = &swings[0];
    let first_range = (first.price.abs() * 0.008).max(0.4);
    let first_tag = match first.tag.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(validate_tag_str(raw, &mut seen_tags)?),
        _ => None,
    };
    candles.push(ChartCandle {
        time: BASE_EPOCH_SECS,
        open: round2(first.price - first_range * 0.25),
        high: round2(first.price + first_range * 0.6),
        low: round2(first.price - first_range * 0.6),
        close: round2(first.price),
        volume: Some(140.0),
        tag: first_tag,
    });

    for (leg_idx, swing) in swings.iter().enumerate().skip(1) {
        let prev_price = swings[leg_idx - 1].price;
        let target_price = swing.price;
        let impulse = swing.impulse.unwrap_or(false);
        let raw_bars = swing.bars.unwrap_or(5).clamp(1, 60);
        let bars = if impulse && raw_bars == 3 && swing.tag.is_some() {
            4
        } else {
            raw_bars
        };
        let is_up = target_price >= prev_price;
        let total_delta = target_price - prev_price;
        let avg_range = ((total_delta.abs() / (bars as f64)).max(target_price.abs() * 0.006)).max(0.5);
        let mut rng = DeterministicRng::from_seed(chart_id, leg_idx);

        let swing_tag = match swing.tag.as_deref() {
            Some(raw) if !raw.trim().is_empty() => Some(validate_tag_str(raw, &mut seen_tags)?),
            _ => None,
        };

        let fvg_tags = if impulse && bars >= 3 {
            if let Some(ref base_tag) = swing_tag {
                let start_tag = validate_tag_str(&format!("{base_tag}-fvg-start"), &mut seen_tags)?;
                let end_tag = validate_tag_str(&format!("{base_tag}-fvg-end"), &mut seen_tags)?;
                Some((start_tag, end_tag))
            } else {
                None
            }
        } else {
            None
        };

        let mut current_open = candles.last().map_or(prev_price, |c| c.close);
        let leg_start_index = candles.len();

        for b in 0..bars {
            let is_last = b + 1 == bars;
            let progress = ((b + 1) as f64) / (bars as f64);
            let jitter = (rng.next_f64() - 0.5) * avg_range * 0.35;
            let raw_close = if is_last {
                target_price
            } else {
                prev_price + total_delta * progress + jitter
            };

            let open = round2(current_open);
            let close = round2(raw_close);
            let wick_up = round2((0.25 + rng.next_f64() * 0.55) * avg_range);
            let wick_dn = round2((0.25 + rng.next_f64() * 0.55) * avg_range);
            let mut high = round2(open.max(close) + wick_up);
            let mut low = round2((open.min(close) - wick_dn).max(0.01));

            let mut final_open = open;
            let mut final_close = close;

            if is_last {
                apply_pivot_pattern(
                    swing.pattern.as_deref(),
                    is_up,
                    target_price,
                    avg_range,
                    &mut final_open,
                    &mut high,
                    &mut low,
                    &mut final_close,
                );
            }

            let bar_tag = if is_last {
                swing_tag.clone()
            } else if b == 0 {
                fvg_tags.as_ref().map(|(s, _)| s.clone())
            } else if b == 2 && bars > 3 {
                fvg_tags.as_ref().map(|(_, e)| e.clone())
            } else {
                None
            };

            let time = BASE_EPOCH_SECS + (candles.len() as i64) * step;
            let vol_mult = if impulse { 2.1 } else { 1.0 };
            let volume = round2((130.0 + rng.next_f64() * 160.0) * vol_mult);

            candles.push(ChartCandle {
                time,
                open: final_open,
                high,
                low,
                close: final_close,
                volume: Some(volume),
                tag: bar_tag,
            });
            current_open = final_close;
        }

        // If `impulse: true` and `bars >= 3`, enforce a clean 3-bar Fair Value Gap between bar 0 and bar 2 of the leg
        if impulse && bars >= 3 {
            let i0 = leg_start_index;
            let i1 = leg_start_index + 1;
            let i2 = leg_start_index + 2;
            // If bars == 3 and fvg_end wasn't assigned because bar 2 was the last bar, only assign if swing_tag was None
            if bars == 3
                && candles[i2].tag.is_none()
                && let Some((_, ref end_t)) = fvg_tags
            {
                candles[i2].tag = Some(end_t.clone());
            }
            if is_up {
                let mid = (candles[i0].close + candles[i2].open) * 0.5;
                let gap_half = (avg_range * 0.28).max(0.25);
                let fvg_bottom = round2(mid - gap_half);
                let fvg_top = round2(mid + gap_half);
                candles[i0].high = round2(candles[i0].open.max(candles[i0].close).max(fvg_bottom));
                if candles[i0].close > candles[i0].high {
                    candles[i0].close = candles[i0].high;
                }
                if candles[i0].open > candles[i0].high {
                    candles[i0].open = candles[i0].high;
                }
                candles[i0].low = candles[i0].low.min(candles[i0].open.min(candles[i0].close));

                candles[i2].low = round2(candles[i2].open.min(candles[i2].close).min(fvg_top));
                if candles[i2].open < fvg_top {
                    candles[i2].open = fvg_top;
                }
                if candles[i2].close < fvg_top {
                    candles[i2].close = round2(fvg_top + gap_half);
                }
                candles[i2].low = fvg_top;
                candles[i2].high = candles[i2].high.max(candles[i2].open.max(candles[i2].close));

                // Middle impulse candle bridges the entire gap
                candles[i1].open = candles[i0].close;
                candles[i1].close = candles[i2].open;
                candles[i1].low = candles[i1].open.min(candles[i1].close);
                candles[i1].high = candles[i1].open.max(candles[i1].close) + round2(gap_half * 0.4);
            } else {
                let mid = (candles[i0].close + candles[i2].open) * 0.5;
                let gap_half = (avg_range * 0.28).max(0.25);
                let fvg_top = round2(mid + gap_half);
                let fvg_bottom = round2((mid - gap_half).max(0.05));
                candles[i0].low = fvg_top;
                if candles[i0].open < fvg_top {
                    candles[i0].open = round2(fvg_top + gap_half);
                }
                if candles[i0].close < fvg_top {
                    candles[i0].close = fvg_top;
                }
                candles[i0].high = candles[i0].high.max(candles[i0].open.max(candles[i0].close));

                candles[i2].high = fvg_bottom;
                if candles[i2].open > fvg_bottom {
                    candles[i2].open = fvg_bottom;
                }
                if candles[i2].close > fvg_bottom {
                    candles[i2].close = round2((fvg_bottom - gap_half).max(0.05));
                }
                candles[i2].low = candles[i2].low.min(candles[i2].open.min(candles[i2].close));

                candles[i1].open = candles[i0].close;
                candles[i1].close = candles[i2].open;
                candles[i1].high = candles[i1].open.max(candles[i1].close);
                candles[i1].low =
                    (candles[i1].open.min(candles[i1].close) - round2(gap_half * 0.4)).max(0.01);
            }
        }
    }

    if candles.len() < 3 || candles.len() > 500 {
        return Err(ChartError::Coded {
            code: "invalid_candle_count",
            message: format!(
                "Synthesized candle count ({}) must be between 3 and 500",
                candles.len()
            ),
        });
    }

    Ok(candles)
}

#[allow(clippy::too_many_arguments)]
fn apply_pivot_pattern(
    pattern: Option<&str>,
    is_up: bool,
    target_price: f64,
    avg_range: f64,
    open: &mut f64,
    high: &mut f64,
    low: &mut f64,
    close: &mut f64,
) {
    let unit = avg_range.max(0.6);
    let target = round2(target_price);
    match pattern.unwrap_or("") {
        "bullish-pinbar" | "fakey-bullish" => {
            *low = target;
            *open = round2(target + unit * 1.4);
            *close = round2(target + unit * 1.85);
            *high = round2(target + unit * 2.05);
        }
        "bearish-pinbar" | "fakey-bearish" => {
            *high = target;
            *open = round2((target - unit * 1.4).max(0.05));
            *close = round2((target - unit * 1.85).max(0.05));
            *low = round2((target - unit * 2.05).max(0.01));
        }
        "bullish-engulfing" => {
            *low = target;
            *open = round2(target + unit * 0.25);
            *close = round2(target + unit * 1.9);
            *high = round2(target + unit * 2.1);
        }
        "bearish-engulfing" => {
            *high = target;
            *open = round2((target - unit * 0.25).max(0.05));
            *close = round2((target - unit * 1.9).max(0.05));
            *low = round2((target - unit * 2.1).max(0.01));
        }
        "bearish-last-down" => {
            *low = target;
            *open = round2(target + unit * 1.35);
            *close = round2(target + unit * 0.35);
            *high = round2(target + unit * 1.55);
        }
        "bullish-last-up" => {
            *high = target;
            *open = round2((target - unit * 1.35).max(0.05));
            *close = round2((target - unit * 0.35).max(0.05));
            *low = round2((target - unit * 1.55).max(0.01));
        }
        "doji" => {
            *open = target;
            *close = target;
            *high = round2(target + unit * 0.9);
            *low = round2((target - unit * 0.9).max(0.01));
        }
        _ => {
            if is_up {
                *high = target;
                *close = round2((target - unit * 0.2).max(0.05));
                if *open > *high {
                    *open = round2((*high - unit * 0.6).max(0.05));
                }
                *low = (*low).min(open.min(*close));
            } else {
                *low = target;
                *close = round2(target + unit * 0.2);
                if *open < *low {
                    *open = round2(*low + unit * 0.6);
                }
                *high = (*high).max(open.max(*close));
            }
        }
    }
}

fn find_candle_by_selector<'a>(
    candles: &'a [ChartCandle],
    selector: &Value,
    context: &str,
) -> Result<&'a ChartCandle, ChartError> {
    if let Some(tag) = selector.get("tag").and_then(Value::as_str) {
        let trimmed = tag.trim();
        if let Some(found) = candles
            .iter()
            .find(|c| c.tag.as_deref() == Some(trimmed))
        {
            return Ok(found);
        }
        let valid_tags: Vec<&str> = candles
            .iter()
            .filter_map(|c| c.tag.as_deref())
            .collect();
        return Err(ChartError::Coded {
            code: "unknown_candle_tag",
            message: format!(
                "Unknown candle tag '{trimmed}' in {context}. Valid tags: [{}]",
                valid_tags.join(", ")
            ),
        });
    }

    if let Some(bar_idx) = selector.get("bar").and_then(Value::as_i64) {
        let len = candles.len() as i64;
        let resolved = if bar_idx < 0 { len + bar_idx } else { bar_idx };
        if resolved >= 0 && resolved < len {
            return Ok(&candles[resolved as usize]);
        }
        return Err(ChartError::Coded {
            code: "invalid_bar_index",
            message: format!(
                "Bar index {bar_idx} in {context} is out of bounds for {} candles",
                candles.len()
            ),
        });
    }

    if let Some(time_val) = selector.get("time").and_then(Value::as_i64)
        && let Some(found) = candles.iter().find(|c| c.time == time_val)
    {
        return Ok(found);
    }

    Err(ChartError::Coded {
        code: "invalid_point_ref",
        message: format!("PointRef in {context} requires 'tag' or 'bar' selector"),
    })
}

fn extract_candle_price(
    candle: &ChartCandle,
    price_at: &str,
    context: &str,
) -> Result<f64, ChartError> {
    match price_at.trim() {
        "open" => Ok(candle.open),
        "high" => Ok(candle.high),
        "low" => Ok(candle.low),
        "close" => Ok(candle.close),
        other => Err(ChartError::Coded {
            code: "invalid_price_at",
            message: format!(
                "Unsupported price_at '{other}' in {context}; expected 'open' | 'high' | 'low' | 'close'"
            ),
        }),
    }
}

pub fn resolve_point_ref(
    candles: &[ChartCandle],
    raw: &Value,
    context: &str,
) -> Result<ChartPoint, ChartError> {
    // Allow already-resolved `{ time, price }` when neither `tag` nor `bar` is present
    if raw.get("tag").is_none()
        && raw.get("bar").is_none()
        && let (Some(time), Some(price)) = (
            raw.get("time").and_then(Value::as_i64),
            raw.get("price").and_then(Value::as_f64),
        )
        && price.is_finite()
    {
        return Ok(ChartPoint {
            time,
            price: round2(price),
        });
    }

    let candle = find_candle_by_selector(candles, raw, context)?;
    let price = if let Some(explicit_price) = raw.get("price").and_then(Value::as_f64) {
        if !explicit_price.is_finite() {
            return Err(ChartError::Coded {
                code: "invalid_point_ref",
                message: format!("Non-finite price in {context}"),
            });
        }
        round2(explicit_price)
    } else if let Some(price_at) = raw.get("price_at").and_then(Value::as_str) {
        extract_candle_price(candle, price_at, context)?
    } else {
        candle.close
    };

    Ok(ChartPoint {
        time: candle.time,
        price,
    })
}

pub fn resolve_drawing(
    candles: &[ChartCandle],
    raw: &Value,
    idx: usize,
) -> Result<ChartDrawing, ChartError> {
    let obj = raw.as_object().ok_or_else(|| ChartError::Coded {
        code: "invalid_drawing",
        message: format!("Drawing at index {idx} must be a JSON object"),
    })?;

    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ChartError::Coded {
            code: "invalid_drawing",
            message: format!("Drawing at index {idx} requires a non-empty 'id'"),
        })?
        .to_string();

    let dtype = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let tone = obj
        .get("tone")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| {
            matches!(
                s.as_str(),
                "bullish" | "bearish" | "demand" | "supply" | "structure" | "warning" | "neutral"
            )
        });
    let style = obj
        .get("style")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| matches!(s.as_str(), "solid" | "dashed" | "dotted"));
    let label = obj
        .get("label")
        .and_then(Value::as_str)
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty());

    match dtype {
        "horizontal-line" => {
            let price = if let Some(p) = obj.get("price").and_then(Value::as_f64) {
                if !p.is_finite() {
                    return Err(ChartError::Coded {
                        code: "invalid_drawing",
                        message: format!("horizontal-line '{id}' has non-finite price"),
                    });
                }
                round2(p)
            } else if let Some(at_val) = obj.get("at") {
                let pt = resolve_point_ref(candles, at_val, &format!("horizontal-line '{id}'.at"))?;
                pt.price
            } else {
                return Err(ChartError::Coded {
                    code: "invalid_drawing",
                    message: format!("horizontal-line '{id}' requires 'price' or 'at'"),
                });
            };
            Ok(ChartDrawing::HorizontalLine {
                id,
                price,
                label,
                tone,
                style,
                source_spec: Some(raw.clone()),
            })
        }
        "trend-line" => {
            let pts = obj
                .get("points")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .ok_or_else(|| ChartError::Coded {
                    code: "invalid_drawing",
                    message: format!("trend-line '{id}' requires 'points' array of length 2"),
                })?;
            let p1 = resolve_point_ref(candles, &pts[0], &format!("trend-line '{id}'.points[0]"))?;
            let p2 = resolve_point_ref(candles, &pts[1], &format!("trend-line '{id}'.points[1]"))?;
            let width = obj
                .get("width")
                .and_then(Value::as_u64)
                .map(|w| w.clamp(1, 4) as u8);
            Ok(ChartDrawing::TrendLine {
                id,
                points: [p1, p2],
                label,
                tone,
                style,
                width,
                source_spec: Some(raw.clone()),
            })
        }
        "price-zone" => {
            let from_val = obj.get("from").ok_or_else(|| ChartError::Coded {
                code: "invalid_drawing",
                message: format!("price-zone '{id}' requires 'from'"),
            })?;
            let to_val = obj.get("to").ok_or_else(|| ChartError::Coded {
                code: "invalid_drawing",
                message: format!("price-zone '{id}' requires 'to'"),
            })?;
            let p_from = resolve_point_ref(candles, from_val, &format!("price-zone '{id}'.from"))?;
            let p_to = resolve_point_ref(candles, to_val, &format!("price-zone '{id}'.to"))?;
            Ok(ChartDrawing::PriceZone {
                id,
                from: ChartPoint {
                    time: p_from.time.min(p_to.time),
                    price: p_from.price.min(p_to.price),
                },
                to: ChartPoint {
                    time: p_from.time.max(p_to.time),
                    price: p_from.price.max(p_to.price),
                },
                label,
                tone,
                source_spec: Some(raw.clone()),
            })
        }
        "text" => {
            let anchor_val = obj.get("anchor").ok_or_else(|| ChartError::Coded {
                code: "invalid_drawing",
                message: format!("text drawing '{id}' requires 'anchor'"),
            })?;
            let anchor = resolve_point_ref(candles, anchor_val, &format!("text '{id}'.anchor"))?;
            let text = obj
                .get("text")
                .and_then(Value::as_str)
                .map(|s| s.trim().chars().take(60).collect::<String>())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ChartError::Coded {
                    code: "invalid_drawing",
                    message: format!("text drawing '{id}' requires non-empty 'text'"),
                })?;
            Ok(ChartDrawing::Text {
                id,
                anchor,
                text,
                tone,
                source_spec: Some(raw.clone()),
            })
        }
        other => Err(ChartError::Coded {
            code: "invalid_drawing_type",
            message: format!(
                "Unsupported drawing type '{other}' on '{id}'; allowed: horizontal-line, trend-line, price-zone, text"
            ),
        }),
    }
}
