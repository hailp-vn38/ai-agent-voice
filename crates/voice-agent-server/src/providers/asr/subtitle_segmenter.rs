use crate::providers::AsrTimedSegment;

/// Maximum subtitle segment display duration (6 seconds).
pub const MAX_SEGMENT_DURATION_MS: u64 = 6_000;
/// Maximum Latin/Vietnamese characters per subtitle segment (up to 2 lines).
pub const MAX_LATIN_CHARS: usize = 84;
/// Maximum CJK characters per subtitle segment (up to 2 lines).
pub const MAX_CJK_CHARS: usize = 35;

/// Minimum segment duration before a clause break (`,`, `;`, `:`, `，`, `、`) closes the segment.
const CLAUSE_MIN_DURATION_MS: u64 = 2_500;
const CLAUSE_MIN_LATIN_CHARS: usize = 35;
const CLAUSE_MIN_CJK_CHARS: usize = 16;

/// Minimum breath-pause silence gap (ms) after accounting for previous token acoustic duration.
const BREATH_PAUSE_GAP_MS: u64 = 450;
/// Start-to-start token delta (ms) that unconditionally counts as a breath break between words.
const BREATH_START_DELTA_MS: u64 = 750;
/// Estimated minimum acoustic duration (ms) of a non-punctuation token when only start timestamps exist.
const EST_TOKEN_DURATION_MS: u64 = 150;

/// Minimum display duration (ms) for a subtitle segment when room exists within `utterance_duration_ms`.
const MIN_SEGMENT_DURATION_MS: u64 = 200;
/// Trailing acoustic tail (ms) added after the final token's start timestamp.
const TRAILING_TOKEN_TAIL_MS: u64 = 400;

#[derive(Debug, Clone)]
struct ParsedToken<'a> {
    clean: &'a str,
    time_ms: u64,
    starts_new_word: bool,
    is_cjk: bool,
    is_punct_only: bool,
    is_hard_punct: bool,
    is_clause_punct: bool,
}

#[derive(Debug, Default)]
struct SegmentBuilder {
    start_ms: u64,
    last_token_ms: u64,
    last_was_punct: bool,
    word_count: usize,
    latin_chars: usize,
    cjk_chars: usize,
    text: String,
}

impl SegmentBuilder {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Groups raw `sherpa-onnx` token streams and per-token start timestamps (in seconds) into
/// natural, display-bounded subtitle segments (`AsrTimedSegment`).
pub(crate) fn segment_timed_tokens(
    fallback_text: &str,
    tokens: &[String],
    timestamps_secs: Option<&[f32]>,
    utterance_duration_ms: u64,
) -> Vec<AsrTimedSegment> {
    let fallback_trimmed = fallback_text.trim();
    let dur = utterance_duration_ms.max(1);

    let Some(ts_slice) = timestamps_secs.filter(|ts| ts.len() == tokens.len() && !ts.is_empty())
    else {
        return fallback_segment(fallback_trimmed, tokens, dur);
    };

    let has_bpe_markers = tokens
        .iter()
        .any(|t| t.starts_with('\u{2581}') || t.starts_with(char::is_whitespace));

    let mut parsed: Vec<ParsedToken<'_>> = Vec::with_capacity(tokens.len());
    for (raw, &ts_sec) in tokens.iter().zip(ts_slice.iter()) {
        let Some((clean, has_word_prefix)) = clean_bpe_token(raw) else {
            continue;
        };
        let time_ms = if ts_sec.is_finite() && ts_sec > 0.0 {
            (ts_sec * 1_000.0).round() as u64
        } else {
            0
        };
        let is_cjk = clean.chars().any(is_cjk_char);
        let is_punct_only = clean.chars().all(is_punct_char);
        let starts_new_word = has_word_prefix || (!has_bpe_markers && !is_cjk && !is_punct_only);
        let is_hard_punct = clean.chars().last().is_some_and(is_hard_punct_char);
        let is_clause_punct = clean.chars().last().is_some_and(is_clause_punct_char);

        parsed.push(ParsedToken {
            clean,
            time_ms,
            starts_new_word,
            is_cjk,
            is_punct_only,
            is_hard_punct,
            is_clause_punct,
        });
    }

    if parsed.is_empty() {
        return fallback_segment(fallback_trimmed, &[], dur);
    }

    let mut out: Vec<AsrTimedSegment> = Vec::new();
    let mut prev_end_ms = 0_u64;
    let mut cur = SegmentBuilder::default();

    for idx in 0..parsed.len() {
        let tok = &parsed[idx];

        // 1. Check whether to close current segment BEFORE attaching `tok`.
        // Token Attachment Invariant: never break before punctuation or mid-word BPE continuation.
        let can_break_before = !cur.is_empty()
            && cur.word_count >= 1
            && !tok.is_punct_only
            && (tok.starts_new_word || tok.is_cjk);

        if can_break_before {
            let delta_ms = tok.time_ms.saturating_sub(cur.last_token_ms);
            let est_prev_dur = if cur.last_was_punct {
                0
            } else {
                EST_TOKEN_DURATION_MS
            };
            let silence_gap_ms = delta_ms.saturating_sub(est_prev_dur);
            let cur_span_ms = tok.time_ms.saturating_sub(cur.start_ms);

            // Rule 2a: Breath pause break (guarded by minimum segment length/word count)
            let breath_break = (delta_ms >= BREATH_START_DELTA_MS
                || silence_gap_ms >= BREATH_PAUSE_GAP_MS)
                && (cur.word_count >= 2 || cur_span_ms >= 800);

            // Rule 3: Safety display cap (6_000ms or 84 Latin chars / 35 CJK chars)
            let tok_chars = tok.clean.chars().count();
            let proj_cjk = cur.cjk_chars + if tok.is_cjk { tok_chars } else { 0 };
            let proj_latin = cur.latin_chars
                + if !tok.is_cjk {
                    tok_chars + usize::from(!cur.text.is_empty())
                } else {
                    0
                };
            let proj_end_ms = tok.time_ms.saturating_add(TRAILING_TOKEN_TAIL_MS).min(dur);
            let proj_span_ms = proj_end_ms.saturating_sub(cur.start_ms);
            let cap_break = cur_span_ms >= MAX_SEGMENT_DURATION_MS
                || proj_span_ms > MAX_SEGMENT_DURATION_MS
                || proj_cjk > MAX_CJK_CHARS
                || proj_latin > MAX_LATIN_CHARS;

            if breath_break || cap_break {
                flush_segment(&mut cur, Some(tok.time_ms), dur, &mut prev_end_ms, &mut out);
            }
        }

        // 2. Attach `tok` to `cur`.
        if cur.is_empty() {
            cur.start_ms = tok.time_ms;
        } else if !tok.is_punct_only
            && tok.starts_new_word
            && needs_space_between(&cur.text, tok.clean)
        {
            cur.text.push(' ');
            cur.latin_chars += 1;
        }
        cur.text.push_str(tok.clean);
        cur.last_token_ms = tok.time_ms.max(cur.last_token_ms);
        cur.last_was_punct = tok.is_punct_only;

        if !tok.is_punct_only && (tok.starts_new_word || tok.is_cjk || cur.word_count == 0) {
            cur.word_count += 1;
        }
        let char_count = tok.clean.chars().count();
        if tok.is_cjk {
            cur.cjk_chars += char_count;
        } else {
            cur.latin_chars += char_count;
        }

        // 3. Check whether to close `cur` AFTER attaching `tok` (Rule 1 & Rule 2b).
        if cur.word_count >= 1 {
            let next_tok = parsed.get(idx + 1);
            let is_decimal_dot = tok.clean == "."
                && next_tok.is_some_and(|nt| {
                    !nt.starts_new_word
                        && nt.clean.chars().next().is_some_and(|c| c.is_ascii_digit())
                });

            if tok.is_hard_punct && !is_decimal_dot {
                let next_ms = next_tok.map(|nt| nt.time_ms);
                flush_segment(&mut cur, next_ms, dur, &mut prev_end_ms, &mut out);
            } else if tok.is_clause_punct {
                let cur_dur_ms = cur.last_token_ms.saturating_sub(cur.start_ms);
                if cur_dur_ms >= CLAUSE_MIN_DURATION_MS
                    || cur.latin_chars >= CLAUSE_MIN_LATIN_CHARS
                    || cur.cjk_chars >= CLAUSE_MIN_CJK_CHARS
                {
                    let next_ms = next_tok.map(|nt| nt.time_ms);
                    flush_segment(&mut cur, next_ms, dur, &mut prev_end_ms, &mut out);
                }
            }
        }
    }

    if !cur.is_empty() {
        flush_segment(&mut cur, None, dur, &mut prev_end_ms, &mut out);
    }

    if out.is_empty() && !fallback_trimmed.is_empty() {
        out.push(AsrTimedSegment {
            start_ms: 0,
            end_ms: dur,
            text: fallback_trimmed.to_owned(),
        });
    }

    out
}

fn flush_segment(
    cur: &mut SegmentBuilder,
    next_token_start_ms: Option<u64>,
    dur: u64,
    prev_end_ms: &mut u64,
    out: &mut Vec<AsrTimedSegment>,
) {
    let text = std::mem::take(&mut cur.text).trim().to_owned();
    let raw_start = cur.start_ms;
    let last_ms = cur.last_token_ms;
    *cur = SegmentBuilder::default();

    if text.is_empty() {
        return;
    }

    let raw_end = match next_token_start_ms {
        Some(next_ms) => next_ms.min(last_ms.saturating_add(TRAILING_TOKEN_TAIL_MS)),
        None => last_ms.saturating_add(TRAILING_TOKEN_TAIL_MS),
    };

    let start_ms = raw_start.max(*prev_end_ms).min(dur.saturating_sub(1));
    let end_ms = raw_end
        .max(start_ms.saturating_add(MIN_SEGMENT_DURATION_MS))
        .min(start_ms.saturating_add(MAX_SEGMENT_DURATION_MS))
        .min(dur);

    // Right-edge overflow protection: if previous segment already reached `dur`, `raw_start >= dur`,
    // or `end_ms <= start_ms`, merge into the last segment when one exists so `0 <= start_ms < end_ms <= dur` always holds.
    if (*prev_end_ms >= dur || raw_start >= dur || end_ms <= start_ms)
        && let Some(last) = out.last_mut()
    {
        if needs_space_between(&last.text, &text) {
            last.text.push(' ');
        }
        last.text.push_str(&text);
        last.end_ms = dur;
        *prev_end_ms = dur;
        return;
    }

    let final_end = end_ms.max(start_ms + 1);
    *prev_end_ms = final_end;
    out.push(AsrTimedSegment {
        start_ms,
        end_ms: final_end,
        text,
    });
}

fn fallback_segment(fallback_trimmed: &str, tokens: &[String], dur: u64) -> Vec<AsrTimedSegment> {
    let text = if !fallback_trimmed.is_empty() {
        fallback_trimmed.to_owned()
    } else {
        join_tokens_fallback(tokens)
    };
    if text.is_empty() {
        Vec::new()
    } else {
        vec![AsrTimedSegment {
            start_ms: 0,
            end_ms: dur,
            text,
        }]
    }
}

fn join_tokens_fallback(tokens: &[String]) -> String {
    let mut out = String::new();
    for raw in tokens {
        let Some((clean, starts_new)) = clean_bpe_token(raw) else {
            continue;
        };
        if !out.is_empty()
            && !clean.chars().all(is_punct_char)
            && starts_new
            && needs_space_between(&out, clean)
        {
            out.push(' ');
        }
        out.push_str(clean);
    }
    out.trim().to_owned()
}

fn clean_bpe_token(raw: &str) -> Option<(&str, bool)> {
    let clean = raw.trim_matches(|c: char| c == '\u{2581}' || c.is_whitespace());
    if clean.is_empty() || is_control_token(clean) {
        return None;
    }
    let has_word_prefix = raw.starts_with('\u{2581}') || raw.starts_with(char::is_whitespace);
    Some((clean, has_word_prefix))
}

fn is_control_token(clean: &str) -> bool {
    (clean.starts_with("<|") && clean.ends_with("|>"))
        || matches!(
            clean,
            "<unk>" | "<s>" | "</s>" | "<blk>" | "<sos/eos>" | "<pad>"
        )
}

fn needs_space_between(left: &str, right: &str) -> bool {
    let Some(last_char) = left.chars().last() else {
        return false;
    };
    let Some(first_char) = right.chars().next() else {
        return false;
    };
    if last_char.is_whitespace() || matches!(last_char, '(' | '[' | '{' | '“' | '‘' | '「' | '『')
    {
        return false;
    }
    if (is_cjk_char(last_char) || is_cjk_punct(last_char))
        && (is_cjk_char(first_char) || is_cjk_punct(first_char))
    {
        return false;
    }
    true
}

fn is_cjk_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x31F0..=0x31FF // Katakana Phonetic Extensions
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xAC00..=0xD7AF // Hangul Syllables
        | 0x1100..=0x11FF // Hangul Jamo
        | 0x3130..=0x318F // Hangul Compatibility Jamo
    )
}

fn is_cjk_punct(c: char) -> bool {
    matches!(
        c,
        '。' | '？'
            | '！'
            | '，'
            | '、'
            | '；'
            | '：'
            | '「'
            | '」'
            | '『'
            | '』'
            | '（'
            | '）'
            | '《'
            | '》'
    )
}

fn is_hard_punct_char(c: char) -> bool {
    matches!(c, '.' | '?' | '!' | '。' | '？' | '！' | '…')
}

fn is_clause_punct_char(c: char) -> bool {
    matches!(c, ',' | ';' | ':' | '，' | '、' | '；' | '：')
}

fn is_punct_char(c: char) -> bool {
    c.is_ascii_punctuation()
        || is_cjk_punct(c)
        || matches!(c, '…' | '”' | '’' | '“' | '‘' | '»' | '«')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|&x| x.to_owned()).collect()
    }

    #[test]
    fn segments_latin_and_vietnamese_bpe_with_unicode_and_ascii_space_prefixes() {
        let tokens = s(&["▁Hello", ",", "▁world", ".", "▁How", "▁are", "▁you", "?"]);
        let ts = [0.10, 0.35, 0.50, 0.75, 1.10, 1.30, 1.50, 1.70];
        let segs = segment_timed_tokens("Hello, world. How are you?", &tokens, Some(&ts), 2_200);
        assert_eq!(
            segs,
            vec![
                AsrTimedSegment {
                    start_ms: 100,
                    end_ms: 1_100,
                    text: "Hello, world.".to_owned(),
                },
                AsrTimedSegment {
                    start_ms: 1_100,
                    end_ms: 2_100,
                    text: "How are you?".to_owned(),
                },
            ]
        );

        // sherpa-onnx SymbolTable replaces `▁` with ASCII `' '` in `result.tokens`,
        // while continuation subwords (`"s"`, `"ƯỢ"`, `"C"`) have no leading space.
        let sym_tokens = s(&[" the", "s", " Đ", "ƯỢ", "C", "?"]);
        let sym_ts = [0.20, 0.30, 0.45, 0.55, 0.65, 0.75];
        let sym_segs = segment_timed_tokens("thes ĐƯỢC?", &sym_tokens, Some(&sym_ts), 1_500);
        assert_eq!(
            sym_segs,
            vec![AsrTimedSegment {
                start_ms: 200,
                end_ms: 1_150,
                text: "thes ĐƯỢC?".to_owned(),
            }]
        );
    }

    #[test]
    fn segments_cjk_tokens_without_extra_spaces_and_strips_sensevoice_control_tags() {
        let tokens = s(&[
            "<|ja|>",
            "<|NEUTRAL|>",
            "<|Speech|>",
            "<|withitn|>",
            "今",
            "日",
            "は",
            "。",
            "元",
            "気",
            "？",
        ]);
        let ts = [0.0, 0.0, 0.0, 0.0, 0.20, 0.35, 0.50, 0.60, 1.00, 1.20, 1.35];
        let segs = segment_timed_tokens("今日は。元気？", &tokens, Some(&ts), 2_000);
        assert_eq!(
            segs,
            vec![
                AsrTimedSegment {
                    start_ms: 200,
                    end_ms: 1_000,
                    text: "今日は。".to_owned(),
                },
                AsrTimedSegment {
                    start_ms: 1_000,
                    end_ms: 1_750,
                    text: "元気？".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn splits_on_breath_pause_between_words_without_splitting_mid_word_or_detaching_punctuation() {
        // Pause of 550ms before a continuation subword ("standing") and before "," must NOT split;
        // pause of 650ms before a new word ("▁next") after 2 words MUST split.
        let tokens = s(&["▁under", "standing", ",", "▁now", "▁next", "▁part"]);
        let ts = [0.10, 0.65, 1.20, 1.35, 2.00, 2.20];
        let segs = segment_timed_tokens("understanding, now next part", &tokens, Some(&ts), 3_000);
        assert_eq!(
            segs,
            vec![
                AsrTimedSegment {
                    start_ms: 100,
                    end_ms: 1_750,
                    text: "understanding, now".to_owned(),
                },
                AsrTimedSegment {
                    start_ms: 2_000,
                    end_ms: 2_600,
                    text: "next part".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn enforces_6s_duration_and_char_length_safety_caps_at_word_boundaries() {
        // 30 words spaced every 300ms (total 9 seconds, no punctuation, no >=450ms pause)
        let mut tokens = Vec::new();
        let mut ts = Vec::new();
        for i in 0..30 {
            tokens.push(format!("▁w{i}"));
            ts.push((i as f32) * 0.30);
        }
        let fallback = tokens.join(" ").replace('▁', "");
        let segs = segment_timed_tokens(&fallback, &tokens, Some(&ts), 9_500);
        assert!(segs.len() >= 2, "expected safety cap split, got {segs:?}");
        for seg in &segs {
            assert!(
                seg.end_ms - seg.start_ms <= 6_000,
                "segment duration exceeded 6000ms cap: {seg:?}"
            );
            assert!(seg.text.chars().count() <= 84);
        }
        for pair in segs.windows(2) {
            assert!(pair[0].end_ms <= pair[1].start_ms);
        }
    }

    #[test]
    fn clamps_sub_second_padded_utterance_and_falls_back_when_timestamps_missing() {
        // 400ms clip padded to 1000ms during offline decode, where last token timestamp is 420ms
        let tokens = s(&["▁Xin", "▁chào", "▁bạn"]);
        let ts = [0.10, 0.28, 0.42];
        let segs = segment_timed_tokens("Xin chào bạn", &tokens, Some(&ts), 400);
        assert_eq!(
            segs,
            vec![AsrTimedSegment {
                start_ms: 100,
                end_ms: 400,
                text: "Xin chào bạn".to_owned(),
            }]
        );

        // Fallback when timestamps are None
        let fb = segment_timed_tokens("Fallback câu đơn", &[], None, 1_250);
        assert_eq!(
            fb,
            vec![AsrTimedSegment {
                start_ms: 0,
                end_ms: 1_250,
                text: "Fallback câu đơn".to_owned(),
            }]
        );
    }
}
