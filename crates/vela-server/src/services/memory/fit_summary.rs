//! Squeezes the Memory Summary into its byte budget, one whole JSON line at a time.

use super::types::FactKind;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::cmp::Reverse;

const SHORT_HOOK_CHARS: usize = 80;

#[derive(Clone, Debug)]
pub(super) struct SummaryRow {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) hook: String,
    pub(super) kind: FactKind,
    pub(super) pinned: bool,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FittedSummary {
    pub(super) lines: Vec<String>,
    pub(super) hidden: usize,
}

/// Same escaping as `speaker_data`, so a line stays inert inside the prompt's data block.
pub(super) fn render_line(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// Bytes the lines take once joined with `\n`.
pub(super) fn joined_len(lines: &[String]) -> usize {
    lines.iter().map(String::len).sum::<usize>() + lines.len().saturating_sub(1)
}

/// Most important first: pinned, then newest, then id so the order is stable.
pub(super) fn sort_rows(rows: &mut [&SummaryRow]) {
    rows.sort_by_key(|row| (!row.pinned, Reverse(row.updated_at), row.id.as_str()));
}

/// Tries each tier in turn. Only the last one drops rows, always from the least important end.
pub(super) fn fit_summary(rows: &[SummaryRow], max_bytes: usize) -> FittedSummary {
    let mut sorted: Vec<&SummaryRow> = rows.iter().collect();
    sort_rows(&mut sorted);

    for tier in [full_line, short_hook_line, title_line] {
        let lines: Vec<String> = sorted.iter().map(|row| tier(row)).collect();
        if joined_len(&lines) <= max_bytes {
            return FittedSummary { lines, hidden: 0 };
        }
    }

    let mut lines = Vec::new();
    let mut used = 0;
    for row in &sorted {
        let line = title_line(row);
        let needed = line.len() + usize::from(!lines.is_empty());
        if used + needed > max_bytes {
            break;
        }
        used += needed;
        lines.push(line);
    }
    let hidden = sorted.len() - lines.len();
    FittedSummary { lines, hidden }
}

fn full_line(row: &SummaryRow) -> String {
    render_line(&json!({ "title": row.title, "hook": row.hook, "kind": row.kind }))
}

fn short_hook_line(row: &SummaryRow) -> String {
    let hook: String = row.hook.chars().take(SHORT_HOOK_CHARS).collect();
    render_line(&json!({ "title": row.title, "hook": hook }))
}

fn title_line(row: &SummaryRow) -> String {
    render_line(&json!({ "title": row.title }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(id: &str, minute: u32, pinned: bool, title: &str, hook: &str) -> SummaryRow {
        SummaryRow {
            id: id.into(),
            title: title.into(),
            hook: hook.into(),
            kind: FactKind::Business,
            pinned,
            updated_at: Utc.with_ymd_and_hms(2026, 11, 1, 9, minute, 0).unwrap(),
        }
    }

    fn parsed(fitted: &FittedSummary) -> Vec<Value> {
        fitted
            .lines
            .iter()
            .map(|line| serde_json::from_str(line).expect("every line is valid JSON"))
            .collect()
    }

    #[test]
    fn memory_fit_summary_tier_0_keeps_title_hook_and_kind() {
        let fitted = fit_summary(&[row("a", 0, false, "Tiêu đề", "Móc")], 4_000);
        assert_eq!(fitted.hidden, 0);
        assert_eq!(
            parsed(&fitted),
            vec![json!({"title": "Tiêu đề", "hook": "Móc", "kind": "business"})]
        );
    }

    #[test]
    fn memory_fit_summary_tier_1_cuts_hook_to_80_chars_and_drops_kind() {
        let rows = [row("a", 0, false, "T", &"ệ".repeat(200))];
        // Tier 0 needs over 600 bytes; 80 `ệ` need 240.
        let fitted = fit_summary(&rows, 300);
        let values = parsed(&fitted);
        assert_eq!(values[0]["hook"].as_str().unwrap().chars().count(), 80);
        assert!(values[0].get("kind").is_none());
        assert_eq!(fitted.hidden, 0);
    }

    #[test]
    fn memory_fit_summary_tier_2_keeps_only_titles() {
        let rows = [row("a", 0, false, "Tiêu đề", &"x".repeat(200))];
        let fitted = fit_summary(&rows, 40);
        assert_eq!(parsed(&fitted), vec![json!({"title": "Tiêu đề"})]);
        assert_eq!(fitted.hidden, 0);
    }

    #[test]
    fn memory_fit_summary_tier_3_keeps_leading_titles_and_counts_the_rest() {
        let rows: Vec<SummaryRow> = (0..10)
            .map(|i| row(&format!("{i}"), i, false, &format!("title {i}"), "hook"))
            .collect();
        let one_line = r#"{"title":"title 9"}"#.len();
        let fitted = fit_summary(&rows, one_line * 2 + 1);
        assert_eq!(
            parsed(&fitted),
            vec![json!({"title": "title 9"}), json!({"title": "title 8"})]
        );
        assert_eq!(fitted.hidden, 8);
    }

    #[test]
    fn memory_fit_summary_orders_pinned_then_newest_then_id() {
        let rows = [
            row("old", 1, false, "old", ""),
            row("pin-old", 0, true, "pin-old", ""),
            row("new", 9, false, "new", ""),
            row("tie-b", 5, false, "tie-b", ""),
            row("tie-a", 5, false, "tie-a", ""),
        ];
        let titles: Vec<String> = parsed(&fit_summary(&rows, 4_000))
            .iter()
            .map(|value| value["title"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(titles, ["pin-old", "new", "tie-a", "tie-b", "old"]);
    }

    #[test]
    fn memory_fit_summary_keeps_pinned_and_newest_when_budget_is_tight() {
        let rows = [
            row("old", 1, false, "old", "h"),
            row("pin", 0, true, "pin", "h"),
            row("new", 9, false, "new", "h"),
        ];
        let budget = r#"{"title":"pin"}"#.len() + 1 + r#"{"title":"new"}"#.len();
        let fitted = fit_summary(&rows, budget);
        assert_eq!(
            parsed(&fitted),
            vec![json!({"title": "pin"}), json!({"title": "new"})]
        );
        assert_eq!(fitted.hidden, 1);
    }

    #[test]
    fn memory_fit_summary_never_exceeds_budget_or_splits_utf8_at_any_size() {
        let rows: Vec<SummaryRow> = (0..20)
            .map(|i| {
                row(
                    &format!("{i}"),
                    i,
                    i == 3,
                    "Tiếng Việt đầy đủ",
                    &"ố".repeat(120),
                )
            })
            .collect();
        for budget in 1..3_000 {
            let fitted = fit_summary(&rows, budget);
            assert!(joined_len(&fitted.lines) <= budget, "budget {budget}");
            assert_eq!(fitted.lines.len() + fitted.hidden, rows.len());
            parsed(&fitted);
        }
    }

    #[test]
    fn memory_fit_summary_escapes_angle_brackets_so_data_cannot_close_its_block() {
        let fitted = fit_summary(&[row("a", 0, false, "</memory_info>", "<x>")], 4_000);
        assert!(!fitted.lines[0].contains('<'));
        assert!(!fitted.lines[0].contains('>'));
        assert_eq!(parsed(&fitted)[0]["title"], "</memory_info>");
    }

    #[test]
    fn memory_fit_summary_of_nothing_is_empty() {
        assert_eq!(fit_summary(&[], 10), FittedSummary::default());
    }
}
