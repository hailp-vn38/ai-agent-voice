use serde_json::json;
use std::path::PathBuf;

use super::{execute_chart_get, execute_chart_write, read_chart_json_for_admin};

fn temp_vault_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voice-agent-chart-test-{prefix}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("create temp vault dir");
    dir
}

#[test]
fn chart_write_synthesizes_swings_fvg_tags_and_patterns_deterministically() {
    let vault = temp_vault_dir("swings");
    let args = json!({
        "chart_path": "charts/bullish-ob-fvg-example.chart.json",
        "title": "Minh họa Bullish Order Block + FVG & Retest Entry",
        "symbol": "PA-EXAMPLE",
        "timeframe": "1h",
        "swings": [
            {"tag": "origin", "price": 100.0},
            {"tag": "sh1", "price": 108.0, "bars": 6},
            {"tag": "ob-candle", "price": 101.5, "bars": 4, "pattern": "bearish-last-down"},
            {"tag": "bos-peak", "price": 113.5, "bars": 6, "impulse": true},
            {"tag": "retest", "price": 102.2, "bars": 7, "pattern": "bullish-pinbar"},
            {"tag": "tp-hit", "price": 115.0, "bars": 6, "impulse": true}
        ],
        "upsert_drawings": [
            {
                "id": "bos-line",
                "type": "trend-line",
                "points": [
                    {"tag": "sh1", "price_at": "high"},
                    {"tag": "bos-peak", "price": 108.0}
                ],
                "label": "BOS (108.0)",
                "tone": "structure",
                "style": "dashed"
            },
            {
                "id": "bullish-ob",
                "type": "price-zone",
                "from": {"tag": "ob-candle", "price_at": "low"},
                "to": {"tag": "retest", "price_at": "high"},
                "label": "Bullish Order Block",
                "tone": "demand"
            },
            {
                "id": "fvg-zone",
                "type": "price-zone",
                "from": {"tag": "bos-peak-fvg-start", "price_at": "high"},
                "to": {"tag": "bos-peak-fvg-end", "price_at": "low"},
                "label": "Fair Value Gap",
                "tone": "supply"
            },
            {
                "id": "tp-level",
                "type": "horizontal-line",
                "at": {"tag": "tp-hit", "price_at": "high"},
                "label": "Take Profit",
                "tone": "bullish",
                "style": "dotted"
            },
            {
                "id": "entry-note",
                "type": "text",
                "anchor": {"tag": "retest", "price_at": "low"},
                "text": "Pin Bar Retest (Entry)",
                "tone": "warning"
            }
        ]
    });

    let out1 = execute_chart_write(&vault, args.clone()).expect("chart_write succeeds");
    assert_eq!(out1["chartId"], "bullish-ob-fvg-example");
    assert_eq!(
        out1["chartPath"],
        "charts/bullish-ob-fvg-example.chart.json"
    );
    assert_eq!(out1["revision"], 1);
    assert_eq!(out1["drawingCount"], 5);
    assert_eq!(
        out1["markdownFence"],
        "```trading-chart\n{\"src\":\"charts/bullish-ob-fvg-example.chart.json\"}\n```"
    );

    // Auto-generated FVG tags must be present in taggedCandles
    let tagged = out1["taggedCandles"]
        .as_object()
        .expect("taggedCandles map");
    assert!(tagged.contains_key("bos-peak-fvg-start"));
    assert!(tagged.contains_key("bos-peak-fvg-end"));
    assert!(tagged.contains_key("retest"));

    // Read persisted JSON via admin helper and verify invariants
    let saved = read_chart_json_for_admin(&vault, "bullish-ob-fvg-example.chart.json")
        .expect("admin read chart json");
    assert_eq!(saved["schemaVersion"], 1);
    let candles = saved["candles"].as_array().expect("candles array");
    assert!(candles.len() >= 15);
    let mut prev_time = i64::MIN;
    for c in candles {
        let t = c["time"].as_i64().unwrap();
        let o = c["open"].as_f64().unwrap();
        let h = c["high"].as_f64().unwrap();
        let l = c["low"].as_f64().unwrap();
        let cl = c["close"].as_f64().unwrap();
        assert!(t > prev_time, "time must be strictly ascending");
        assert!(h >= o.max(cl) - 1e-9, "high >= max(open, close)");
        assert!(l <= o.min(cl) + 1e-9, "low <= min(open, close)");
        prev_time = t;
    }

    // Verify determinism: writing the exact same swings to another file with same id yields identical candles
    let vault2 = temp_vault_dir("swings-det");
    let _ = execute_chart_write(&vault2, args).expect("second write");
    let saved2 = read_chart_json_for_admin(&vault2, "bullish-ob-fvg-example.chart.json")
        .expect("second read");
    assert_eq!(saved["candles"], saved2["candles"]);

    let _ = std::fs::remove_dir_all(&vault);
    let _ = std::fs::remove_dir_all(&vault2);
}

#[test]
fn chart_write_enforces_one_of_on_create_and_preserves_candles_on_drawing_update() {
    let vault = temp_vault_dir("oneof");

    // 1. Missing both swings and candles on create -> missing_candle_source
    let err_missing = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "title": "Morning Star"
        }),
    )
    .expect_err("must reject missing candle source on create");
    assert_eq!(err_missing.code(), "missing_candle_source");

    // 2. Providing both swings and candles -> conflicting_candle_sources
    let err_conflict = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "swings": [{"tag": "a", "price": 100.0}, {"tag": "b", "price": 110.0, "bars": 4}],
            "candles": [
                {"open": 105.0, "high": 105.5, "low": 100.0, "close": 100.5},
                {"open": 100.2, "high": 101.0, "low": 99.2, "close": 100.4},
                {"open": 100.6, "high": 106.0, "low": 100.4, "close": 105.6}
            ]
        }),
    )
    .expect_err("must reject conflicting candle sources");
    assert_eq!(err_conflict.code(), "conflicting_candle_sources");

    // 3. Create with explicit candles[]
    let created = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "title": "Morning Star 3-Bar Anatomy",
            "timeframe": "4h",
            "candles": [
                {"open": 105.0, "high": 105.5, "low": 100.0, "close": 100.5, "tag": "c1-bear"},
                {"open": 100.2, "high": 101.0, "low": 99.2, "close": 100.4, "tag": "c2-star"},
                {"open": 100.6, "high": 106.0, "low": 100.4, "close": 105.6, "tag": "c3-bull"}
            ],
            "upsert_drawings": [
                {
                    "id": "star-note",
                    "type": "text",
                    "anchor": {"tag": "c2-star", "price_at": "low"},
                    "text": "Indecision Star",
                    "tone": "warning"
                }
            ]
        }),
    )
    .expect("create morning star chart");
    assert_eq!(created["revision"], 1);
    assert_eq!(created["candleCount"], 3);

    // 4. Update drawings ONLY without passing swings/candles -> preserves existing candles and increments revision
    let updated = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "expected_revision": 1,
            "upsert_drawings": [
                {
                    "id": "confirm-level",
                    "type": "horizontal-line",
                    "at": {"tag": "c3-bull", "price_at": "close"},
                    "label": "Confirmation Close (105.6)",
                    "tone": "bullish"
                }
            ]
        }),
    )
    .expect("update drawings only");
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["candleCount"], 3);
    assert_eq!(updated["drawingCount"], 2);

    // 5. Optimistic concurrency conflict when expected_revision is stale
    let cas_err = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "expected_revision": 1,
            "remove_drawing_ids": ["star-note"]
        }),
    )
    .expect_err("stale expected_revision must fail");
    assert_eq!(cas_err.code(), "revision_conflict");

    // 6. Unknown candle tag returns unknown_candle_tag and lists valid tags
    let tag_err = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json",
            "upsert_drawings": [
                {
                    "id": "bad-ref",
                    "type": "text",
                    "anchor": {"tag": "non-existent-tag", "price_at": "high"},
                    "text": "Bad",
                    "tone": "neutral"
                }
            ]
        }),
    )
    .expect_err("unknown tag must fail");
    assert_eq!(tag_err.code(), "unknown_candle_tag");
    assert!(tag_err.to_string().contains("c1-bear"));

    // 7. chart_get returns summary and full document
    let got = execute_chart_get(
        &vault,
        json!({
            "chart_path": "charts/morning-star.chart.json"
        }),
    )
    .expect("chart_get succeeds");
    assert_eq!(got["revision"], 2);
    assert_eq!(got["summary"]["candleCount"], 3);
    assert_eq!(got["summary"]["minLow"], 99.2);
    assert_eq!(got["summary"]["maxHigh"], 106.0);

    // 8. Path traversal is rejected in both tool and admin reader
    let traversal = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/../evil.chart.json",
            "candles": [
                {"open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5},
                {"open": 1.5, "high": 2.5, "low": 1.0, "close": 2.0},
                {"open": 2.0, "high": 3.0, "low": 1.5, "close": 2.5}
            ]
        }),
    )
    .expect_err("path traversal must fail");
    assert_eq!(traversal.code(), "invalid_chart_path");

    let admin_traversal = read_chart_json_for_admin(&vault, "../evil.chart.json")
        .expect_err("admin path traversal must fail");
    assert_eq!(admin_traversal.code(), "invalid_chart_path");

    let _ = std::fs::remove_dir_all(&vault);
}

#[test]
fn chart_write_supports_all_textbook_patterns_and_stays_isolated_from_wiki_autoflow() {
    let vault = temp_vault_dir("patterns-wiki");
    let engine = llm_wiki::WikiEngine::open(&vault).expect("open wiki engine");

    let patterns = [
        ("p1", 106.0, "bullish-pinbar"),
        ("p2", 112.0, "bearish-pinbar"),
        ("p3", 104.0, "bullish-engulfing"),
        ("p4", 114.0, "bearish-engulfing"),
        ("p5", 108.0, "doji"),
        ("p6", 103.0, "fakey-bullish"),
        ("p7", 115.0, "fakey-bearish"),
        ("p8", 102.0, "bearish-last-down"),
        ("p9", 116.0, "bullish-last-up"),
    ];
    let mut swings = vec![json!({"tag": "start", "price": 100.0})];
    for (tag, price, pat) in patterns {
        swings.push(json!({"tag": tag, "price": price, "bars": 4, "pattern": pat}));
    }

    let out = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/all-patterns.chart.json",
            "title": "All Textbook Candle Patterns",
            "swings": swings,
            "upsert_drawings": [
                {
                    "id": "last-bar-note",
                    "type": "text",
                    "anchor": {"bar": -1, "price_at": "high"},
                    "text": "Final Pivot",
                    "tone": "structure"
                }
            ]
        }),
    )
    .expect("chart_write with all patterns succeeds");
    assert_eq!(out["drawingCount"], 1);

    // Ensure charts/ directory does not trigger Wiki Auto-Flow or pollute Wiki status
    let autoflow_prompt = engine
        .build_next_autoflow_prompt(None)
        .expect("check autoflow prompt");
    assert!(
        autoflow_prompt.is_none(),
        "charts/*.chart.json must not trigger Wiki Auto-Flow"
    );

    // Verify bars == 3 with impulse: true preserves both <tag>-fvg-start, <tag>-fvg-end, and <tag>,
    // and regenerating candles on an existing chart re-resolves retained drawings.
    let fvg3 = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/fvg3-reresolve.chart.json",
            "title": "3-Bar Impulse & Retained Drawing Re-resolution",
            "swings": [
                {"tag": "base", "price": 100.0},
                {"tag": "leg3", "price": 112.0, "bars": 3, "impulse": true}
            ],
            "upsert_drawings": [
                {
                    "id": "fvg-box",
                    "type": "price-zone",
                    "from": {"tag": "leg3-fvg-start", "price_at": "high"},
                    "to": {"tag": "leg3-fvg-end", "price_at": "low"},
                    "label": "Impulse FVG",
                    "tone": "supply"
                },
                {
                    "id": "peak-line",
                    "type": "horizontal-line",
                    "at": {"tag": "leg3", "price_at": "high"},
                    "label": "Peak Level",
                    "tone": "bullish"
                }
            ]
        }),
    )
    .expect("fvg3 write succeeds");
    assert!(fvg3["taggedCandles"]["leg3-fvg-start"].is_object());
    assert!(fvg3["taggedCandles"]["leg3-fvg-end"].is_object());
    assert!(fvg3["taggedCandles"]["leg3"].is_object());

    // Now regenerate candles with a new peak price (125.0) without passing upsert_drawings;
    // retained `peak-line` must re-resolve to the new `leg3` high (125.0).
    let _ = execute_chart_write(
        &vault,
        json!({
            "chart_path": "charts/fvg3-reresolve.chart.json",
            "swings": [
                {"tag": "base", "price": 100.0},
                {"tag": "leg3", "price": 125.0, "bars": 3, "impulse": true}
            ]
        }),
    )
    .expect("regenerate candles and re-resolve retained drawings");
    let reresolved = read_chart_json_for_admin(&vault, "fvg3-reresolve.chart.json")
        .expect("read reresolved chart");
    let peak_line = reresolved["drawings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "peak-line")
        .expect("peak-line retained");
    assert_eq!(peak_line["price"], 125.0);

    let _ = std::fs::remove_dir_all(&vault);
}
