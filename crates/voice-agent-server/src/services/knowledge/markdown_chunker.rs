//! Pure Markdown passage selection inspired by QMD's heading-aware chunking.
//! Produces UTF-8 byte and body-relative line offsets; does not mutate sources.
use sha2::{Digest, Sha256};

const TARGET_CHARS: usize = 3600;
const WINDOW_CHARS: usize = 800;
const OVERLAP_CHARS: usize = 540;

#[derive(Clone, Debug)]
pub(crate) struct MarkdownChunk {
    pub ordinal: i64,
    pub heading_path: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub sha256: String,
    pub text: String,
}

struct Line {
    byte: usize,
    char_pos: usize,
    safe: bool,
    priority: i32,
    heading_path: String,
}

fn fence(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return None;
    }
    let delimiter = trimmed.chars().next()?;
    if delimiter != '\u{60}' && delimiter != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == delimiter).count();
    if count < 3 {
        return None;
    }
    let suffix = &trimmed[count..];
    Some((delimiter, count, suffix))
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return None;
    }
    let depth = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&depth) {
        return None;
    }
    let rest = &trimmed[depth..];
    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let label = rest.trim();
    if label.is_empty() {
        return None;
    }
    Some((depth, label))
}

/// Boundaries are line starts; no returned passage cuts a UTF-8 codepoint or
/// slices within an open fenced block. A very long fence is kept intact.
pub(crate) fn split_markdown(body: &str) -> Vec<MarkdownChunk> {
    if body.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut byte_pos = 0usize;
    let mut char_pos = 0usize;
    let mut open_fence: Option<(char, usize)> = None;
    let mut headings: Vec<String> = Vec::new();
    for full_line in body.split_inclusive('\n') {
        let line = full_line.trim_end_matches(['\r', '\n']);
        let safe = open_fence.is_none();
        let mut priority = 1;
        if let Some((symbol, width)) = open_fence {
            if let Some((found, count, suffix)) = fence(line)
                && found == symbol
                && count >= width
                && suffix.trim().is_empty()
            {
                open_fence = None;
            }
        } else if let Some((symbol, width, _)) = fence(line) {
            open_fence = Some((symbol, width));
            priority = 80;
        } else if let Some((depth, label)) = heading(line) {
            headings.truncate(depth - 1);
            headings.push(label.to_string());
            priority = 120 - (depth as i32 * 10);
        } else if line.trim().is_empty() {
            priority = 20;
        } else if ["- ", "* ", "+ "]
            .iter()
            .any(|marker| line.trim_start().starts_with(*marker))
        {
            priority = 5;
        }

        lines.push(Line {
            byte: byte_pos,
            char_pos,
            safe,
            priority,
            heading_path: headings.join(" / "),
        });
        byte_pos += full_line.len();
        char_pos += full_line.chars().count();
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let target = lines[start].char_pos.saturating_add(TARGET_CHARS);
        let mut end = lines.len();
        if char_pos > target {
            let window_start = target.saturating_sub(WINDOW_CHARS);
            let mut choice: Option<(usize, i64)> = None;
            let mut fallback: Option<usize> = None;
            let mut next_safe: Option<usize> = None;
            for idx in (start + 1)..lines.len() {
                let candidate = &lines[idx];
                if !candidate.safe {
                    continue;
                }
                if candidate.char_pos <= target {
                    fallback = Some(idx);
                    if candidate.char_pos >= window_start {
                        let distance = target - candidate.char_pos;
                        let score = i64::from(candidate.priority) * 1000
                            - i64::try_from(distance).unwrap_or(i64::MAX);
                        if choice.is_none_or(|(_, best)| score > best) {
                            choice = Some((idx, score));
                        }
                    }
                } else {
                    next_safe = Some(idx);
                    break;
                }
            }
            end = choice
                .map(|(idx, _)| idx)
                .or(fallback)
                .or(next_safe)
                .unwrap_or(lines.len());
        }
        if end <= start {
            end = lines.len();
        }
        let start_byte = lines[start].byte;
        let end_byte = if end == lines.len() {
            body.len()
        } else {
            lines[end].byte
        };
        let text = body[start_byte..end_byte].to_owned();
        let digest = Sha256::digest(text.as_bytes());
        let sha256 = digest.iter().map(|b| format!("{b:02x}")).collect();
        chunks.push(MarkdownChunk {
            ordinal: i64::try_from(chunks.len()).unwrap_or(i64::MAX),
            heading_path: lines[start].heading_path.clone(),
            start_byte,
            end_byte,
            start_line: start + 1,
            end_line: end,
            sha256,
            text,
        });
        if end == lines.len() {
            break;
        }
        let mut new_start = end;
        for idx in ((start + 1)..end).rev() {
            if lines[idx].safe && lines[end].char_pos - lines[idx].char_pos <= OVERLAP_CHARS {
                new_start = idx;
            }
        }
        start = new_start;
    }
    chunks
}

#[cfg(test)]
mod tests;
