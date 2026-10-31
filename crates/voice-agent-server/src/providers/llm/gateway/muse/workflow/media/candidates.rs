use serde_json::Value;

use super::super::{MEDIA_DONE_MARKER, MediaKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::providers::llm::gateway::muse::workflow) struct ReadyMediaCandidate {
    pub path: String,
    pub group: usize,
}

fn strip_done_marker(text: &str) -> String {
    text.replace(MEDIA_DONE_MARKER, "").trim().to_owned()
}

pub(super) fn format_clean_assistant_bubbles(bubbles: &[String]) -> Option<String> {
    let joined = bubbles
        .iter()
        .map(|s| strip_done_marker(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.is_empty()).then_some(joined)
}

pub(super) fn select_effective_ready_attachments(
    ready: &[ReadyMediaCandidate],
) -> Vec<ReadyMediaCandidate> {
    let mut unique: Vec<ReadyMediaCandidate> = Vec::with_capacity(ready.len());
    for item in ready {
        let clean = item
            .path
            .trim()
            .trim_start_matches("sandbox://")
            .trim_start_matches("/home/user/")
            .trim_start_matches('/');
        if !clean.is_empty()
            && !unique.iter().any(|u| {
                u.path
                    .trim()
                    .trim_start_matches("sandbox://")
                    .trim_start_matches("/home/user/")
                    .trim_start_matches('/')
                    == clean
            })
        {
            unique.push(item.clone());
        }
    }
    if unique.len() <= 1 {
        return unique;
    }
    let max_group = unique.iter().map(|c| c.group).max().unwrap_or(0);
    let min_group = unique.iter().map(|c| c.group).min().unwrap_or(0);
    if max_group > min_group {
        let first_group_count = unique.iter().filter(|c| c.group == min_group).count();
        let last_group: Vec<ReadyMediaCandidate> = unique
            .iter()
            .filter(|c| c.group == max_group)
            .cloned()
            .collect();
        if first_group_count > 1 && last_group.len() == 1 {
            return last_group;
        }
    }
    unique
}

pub(super) fn is_presentation_pending(p: &Value) -> bool {
    let status = p
        .get("status")
        .or_else(|| p.get("data").and_then(|d| d.get("status")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(status.as_str(), "loading" | "generating" | "pending")
}

pub(super) fn collect_presentations_from_payload(payload: &Value, out: &mut Vec<Value>) {
    if let Some(arr) = payload.get("presentations").and_then(Value::as_array) {
        for p in arr {
            if !is_presentation_pending(p) {
                out.push(p.clone());
            }
        }
    }
    if let Some(msgs) = payload
        .get("transcript")
        .and_then(|t| t.get("messages"))
        .and_then(Value::as_array)
    {
        for m in msgs {
            if let Some(arr) = m.get("presentations").and_then(Value::as_array) {
                for p in arr {
                    if !is_presentation_pending(p) {
                        out.push(p.clone());
                    }
                }
            }
        }
    }
}

pub(super) fn extract_candidates_from_stream_and_history(
    kind: MediaKind,
    stream_presentations: &[Value],
    turn_events: &[Value],
    assistant_bubbles: &[String],
) -> Vec<ReadyMediaCandidate> {
    let mut candidates = Vec::new();
    let mut group_counter = 0usize;

    for p in stream_presentations {
        let before = candidates.len();
        extract_paths_from_presentation(kind, p, group_counter, &mut candidates);
        if candidates.len() > before {
            group_counter += 1;
        }
    }

    for ev in turn_events {
        let p = ev.get("payload").unwrap_or(ev);
        if !is_presentation_pending(p) {
            let before = candidates.len();
            extract_paths_from_presentation(kind, p, group_counter, &mut candidates);
            if candidates.len() > before {
                group_counter += 1;
            }
        }
        if let Some(arr) = p
            .get("presentations")
            .or_else(|| ev.get("presentations"))
            .and_then(Value::as_array)
        {
            for pres in arr {
                if !is_presentation_pending(pres) {
                    let before = candidates.len();
                    extract_paths_from_presentation(kind, pres, group_counter, &mut candidates);
                    if candidates.len() > before {
                        group_counter += 1;
                    }
                }
            }
        }
    }

    for bubble in assistant_bubbles {
        let before = candidates.len();
        extract_sandbox_paths_from_text(kind, bubble, group_counter, &mut candidates);
        if candidates.len() > before {
            group_counter += 1;
        }
    }

    candidates
}

fn extract_paths_from_presentation(
    kind: MediaKind,
    p: &Value,
    group: usize,
    out: &mut Vec<ReadyMediaCandidate>,
) {
    match kind {
        MediaKind::Image => {
            if let Some(path) = p
                .get("image_path")
                .or_else(|| p.get("path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && !has_video_ext(s))
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
            if let Some(images) = p
                .get("data")
                .and_then(|d| d.get("images"))
                .or_else(|| p.get("images"))
                .and_then(Value::as_array)
            {
                for img in images {
                    if let Some(path) = img
                        .get("path")
                        .or_else(|| img.get("image_path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !has_video_ext(s))
                    {
                        out.push(ReadyMediaCandidate {
                            path: path.to_string(),
                            group,
                        });
                    }
                }
            }
        }
        MediaKind::Video => {
            if let Some(path) = p
                .get("video_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
            if let Some(videos) = p
                .get("data")
                .and_then(|d| d.get("videos"))
                .or_else(|| p.get("videos"))
                .and_then(Value::as_array)
            {
                for vid in videos {
                    if let Some(path) = vid
                        .get("path")
                        .or_else(|| vid.get("video_path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        out.push(ReadyMediaCandidate {
                            path: path.to_string(),
                            group,
                        });
                    }
                }
            }
            if let Some(path) = p
                .get("image_path")
                .or_else(|| p.get("path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| has_video_ext(s))
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
        }
    }

    if let Some(fb) = p.get("fallback_text").and_then(Value::as_str) {
        extract_sandbox_paths_from_text(kind, fb, group, out);
    }
    if let Some(fb) = p
        .get("data")
        .and_then(|d| d.get("fallback_text"))
        .and_then(Value::as_str)
    {
        extract_sandbox_paths_from_text(kind, fb, group, out);
    }
}

fn extract_sandbox_paths_from_text(
    kind: MediaKind,
    text: &str,
    group: usize,
    out: &mut Vec<ReadyMediaCandidate>,
) {
    let mut rest = text;
    while let Some(idx) = rest.find("sandbox://") {
        let after = &rest[idx..];
        let token: String = after
            .chars()
            .take_while(|c| {
                !c.is_whitespace() && !matches!(c, ')' | ']' | '"' | '\'' | '>' | ',' | ';')
            })
            .collect();
        let matches_kind = match kind {
            MediaKind::Image => has_image_ext(&token),
            MediaKind::Video => has_video_ext(&token),
        };
        if matches_kind {
            out.push(ReadyMediaCandidate { path: token, group });
        }
        rest = &after["sandbox://".len()..];
    }
}

fn has_image_ext(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
}

fn has_video_ext(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov")
}

pub(super) fn sniff_media_mime_type(bytes: &[u8], hinted: &str, kind: MediaKind) -> String {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png".to_string();
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return "video/mp4".to_string();
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return "video/webm".to_string();
    }
    let clean = hinted
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if clean.starts_with("image/") || clean.starts_with("video/") {
        return clean;
    }
    match kind {
        MediaKind::Image => "image/png".to_string(),
        MediaKind::Video => "video/mp4".to_string(),
    }
}
