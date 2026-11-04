---
name: trading-chart
description: "Synthesize interactive OHLCV candlestick charts (`trading-chart`) via `chart_write` and `chart_get` when illustrating candlestick patterns, market structure (BOS/CHoCH, Order Block, FVG, Liquidity Sweep), or trade setups (Entry/SL/TP) in Wiki pages and Markdown reports."
---

# Trading Concept Chart Visualization Skill (`trading-chart`)

## 1. Format Selection Rules
- **Use `trading-chart` (`chart_write` / `chart_get`)** to visualize candlestick patterns (Pin Bar, Engulfing, Fakey, Morning Star), market structure (BOS, CHoCH, Order Block, FVG, Liquidity Sweep), chart patterns (Double Bottom, Head & Shoulders, Wyckoff), or trade setups (Entry/SL/TP).
- **Use a `mermaid` block** (`flowchart TD`, `stateDiagram-v2`) for trading checklists, risk-management decision trees, or bot execution workflows.
- **Use a Markdown table** for static backtest metrics, parameter matrices, or win-rate comparisons.

## 2. Three-Step Execution Workflow

### Step 1 — Map the Trading Concept to `swings` XOR `candles` with `tag` Anchors
Choose **exactly one** candle source mode (or omit both when updating drawings on an existing chart):
1. **Macro Structure (`swings` — 15 to 150 bars)**:
   Provide ordered turning points `{ "tag", "price", "bars"?, "impulse"?, "pattern"? }`:
   - Name every key pivot referenced in the lesson with a unique `tag` (`"sh1"`, `"ob-candle"`, `"sweep-low"`, `"bos-peak"`, `"retest-entry"`, `"tp-hit"`).
   - Set `"impulse": true` on strong breakout legs (when `bars >= 3`, the synthesizer automatically creates a Fair Value Gap and tags the 1st and 3rd impulse bars as `"<tag>-fvg-start"` and `"<tag>-fvg-end"`).
   - Set `"pattern"` at signal pivots (`"bullish-pinbar"`, `"bearish-pinbar"`, `"bullish-engulfing"`, `"bearish-engulfing"`, `"doji"`, `"fakey-bullish"`, `"fakey-bearish"`, `"bearish-last-down"`, `"bullish-last-up"`).
2. **Micro Candlestick Pattern (`candles` — 3 to 25 bars)**:
   Provide explicit bars `{ "open", "high", "low", "close", "volume"?, "tag"? }` when explaining exact multi-bar anatomy (e.g., 3-bar Morning Star, Inside Bar).
3. **Completion criterion**: Every pivot, zone boundary, and signal candle discussed in the lesson has a deterministic `tag`.

### Step 2 — Synthesize Chart & Anchor Drawings Atomically (`chart_write`)
1. Define 2–6 focused drawings in `upsert_drawings` using symbolic `PointRef` (`{"tag": "<tag>" | "bar": <int>, "price_at": "open"|"high"|"low"|"close" | "price": <num>}`) and semantic `tone` (`"bullish" | "bearish" | "demand" | "supply" | "structure" | "warning" | "neutral"`):
   - `horizontal-line`: `{ "id", "type": "horizontal-line", "price": <num> | "at": {"tag", "price_at"}, "label", "tone", "style"?: "solid"|"dashed"|"dotted" }`
   - `trend-line`: `{ "id", "type": "trend-line", "points": [PointRef, PointRef], "label"?, "tone", "style"?, "width"?: 2 }`
   - `price-zone`: `{ "id", "type": "price-zone", "from": PointRef, "to": PointRef, "label"?, "tone" }`
   - `text`: `{ "id", "type": "text", "anchor": PointRef, "text" (<= 60 chars), "tone" }`
2. Concept-to-Drawing Reference:
   - **Order Block (OB)**: `price-zone` with `tone: "demand"` (or `"supply"`) from `{"tag": "ob-candle", "price_at": "low"}` to `{"tag": "retest-entry", "price_at": "high"}`.
   - **Fair Value Gap (FVG)**: `price-zone` with `tone: "supply"` from `{"tag": "bos-peak-fvg-start", "price_at": "high"}` to `{"tag": "retest-entry", "price": <fvg_top>}` (or `"bos-peak-fvg-end"` `"low"`).
   - **BOS / CHoCH / Neckline**: `trend-line` (`style: "dashed"`, `tone: "structure"`) from the broken swing pivot horizontally to the breakout bar.
   - **Liquidity Sweep**: `horizontal-line` (`style: "dashed"`, `tone: "bearish"`, `at: {"tag": "equal-low", "price_at": "low"}`) + `text` (`tone: "warning"`) at `{"tag": "sweep-low", "price_at": "low"}`.
3. Call `chart_write` **once** (if modifying drawings on an existing chart without regenerating candles, call `chart_get(chart_path)` first, omit `swings`/`candles`, and pass `expected_revision`).
4. **Completion criterion**: `chart_write` succeeds and returns `taggedCandles` with resolved prices and the `markdownFence` snippet (` ```trading-chart\n{"src":"charts/<slug>.chart.json"}\n``` `).

### Step 3 — Embed `markdownFence` & Align Educational Walkthrough
1. Insert the returned `markdownFence` verbatim into the Wiki page (`wiki_write_file` / `wiki_patch_file`) or Markdown report (`markdown_create`) directly under the concept explanation.
2. Write a structured walkthrough beneath the chart citing the exact labels and resolved price levels from `taggedCandles`.
3. **Completion criterion**: The Markdown document is saved, and every price level, zone, and pattern name in the prose matches the visual chart annotations.
