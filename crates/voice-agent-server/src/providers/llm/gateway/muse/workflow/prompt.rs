use crate::providers::llm::{ChatMessage, LlmRequest};

use super::super::pool::ConversationTurn;
use super::media::MediaBatch;
use super::{
    CUTOUT_HINT, MEDIA_DONE_MARKER, MUSE_MAX_CLIPS_PER_TURN, MUSE_MAX_VIDEO_SECS,
    MUSE_SINGLE_CLIP_LIMIT_SECS,
};

pub fn aspect_ratio_hint(width: u32, height: u32) -> &'static str {
    if width == 0 || height == 0 {
        return "1:1";
    }
    let r = width as f64 / height as f64;
    if (r - 16.0 / 9.0).abs() < 0.15 {
        "16:9"
    } else if (r - 9.0 / 16.0).abs() < 0.15 {
        "9:16"
    } else if (r - 4.0 / 3.0).abs() < 0.12 {
        "4:3"
    } else if (r - 3.0 / 4.0).abs() < 0.12 {
        "3:4"
    } else if (r - 21.0 / 9.0).abs() < 0.15 {
        "21:9"
    } else {
        "1:1"
    }
}

pub fn format_image_prompt(
    prompt: &str,
    width: u32,
    height: u32,
    transparent_background: bool,
) -> String {
    let ratio = aspect_ratio_hint(width, height);
    let mut out = format!(
        "Generate an image (aspect ratio {ratio}): {}",
        prompt.trim()
    );
    if transparent_background {
        out.push_str(CUTOUT_HINT);
    }
    out
}

pub fn format_video_prompt(prompt: &str, width: u32, height: u32, duration_secs: u32) -> String {
    let ratio = aspect_ratio_hint(width, height);
    let secs = duration_secs.max(1);
    if secs > MUSE_SINGLE_CLIP_LIMIT_SECS {
        let beats = secs
            .div_ceil(MUSE_SINGLE_CLIP_LIMIT_SECS)
            .clamp(2, MUSE_MAX_CLIPS_PER_TURN);
        let target_secs = beats * MUSE_SINGLE_CLIP_LIMIT_SECS;
        let max_turn_secs = MUSE_MAX_CLIPS_PER_TURN * MUSE_SINGLE_CLIP_LIMIT_SECS;
        format!(
            "Generate a video (aspect ratio {ratio}, target duration ~{target_secs} seconds across {beats} sequential ~{MUSE_SINGLE_CLIP_LIMIT_SECS}-second beats): {}\n\nMandatory workflow:\n1. Since each generated clip is fixed at ~{MUSE_SINGLE_CLIP_LIMIT_SECS} seconds (up to {MUSE_MAX_CLIPS_PER_TURN} clips / {max_turn_secs} seconds per turn), plan and generate all {beats} sequential ~{MUSE_SINGLE_CLIP_LIMIT_SECS}-second clips covering every beat from 1 to {beats} without skipping or dropping any beat.\n2. Verify that all {beats} clips have been generated (~{target_secs} seconds total), then concatenate/merge them in order into ONE single continuous video.\n3. Send ONLY the final merged video as the main file attachment in the chat — do NOT send or re-attach the individual unmerged clip parts.\n4. Only after the final merged video file has been attached in the chat, send a final confirmation message ending with {MEDIA_DONE_MARKER}.",
            prompt.trim()
        )
    } else {
        format!(
            "Generate a video (aspect ratio {ratio}, 1 clip ~{MUSE_SINGLE_CLIP_LIMIT_SECS} seconds): {}\n\nRequirements:\n- Generate 1 complete ~{MUSE_SINGLE_CLIP_LIMIT_SECS}-second video clip with synchronized audio/narration and subtitles matching the prompt.\n- Attach the final video file directly in the chat.\n- After the video file is attached in the chat, send a final confirmation message ending with {MEDIA_DONE_MARKER}.",
            prompt.trim()
        )
    }
}

/// Splits a video request into the Muse turns needed to produce it, or `None` when it is longer
/// than Muse can deliver in one tool call. Up to one turn's worth of clips fits a single batch;
/// longer videos continue the same chat, each batch scoped to its own beats.
pub fn format_video_batches(
    prompt: &str,
    width: u32,
    height: u32,
    duration_secs: u32,
) -> Option<Vec<MediaBatch>> {
    let secs = duration_secs.max(1);
    if secs > MUSE_MAX_VIDEO_SECS {
        return None;
    }
    let turn_secs = MUSE_MAX_CLIPS_PER_TURN * MUSE_SINGLE_CLIP_LIMIT_SECS;
    if secs <= turn_secs {
        return Some(vec![MediaBatch {
            prompt: format_video_prompt(prompt, width, height, secs),
            target_secs: Some(secs),
        }]);
    }

    let ratio = aspect_ratio_hint(width, height);
    let total_beats = secs.div_ceil(MUSE_SINGLE_CLIP_LIMIT_SECS);
    let batch_count = total_beats.div_ceil(MUSE_MAX_CLIPS_PER_TURN);
    let mut batches = Vec::with_capacity(batch_count as usize);
    for index in 0..batch_count {
        let first_beat = index * MUSE_MAX_CLIPS_PER_TURN + 1;
        let last_beat = ((index + 1) * MUSE_MAX_CLIPS_PER_TURN).min(total_beats);
        let raw_rem = secs.saturating_sub(index * turn_secs);
        let batch_secs = ((last_beat - first_beat + 1) * MUSE_SINGLE_CLIP_LIMIT_SECS)
            .min(raw_rem.max(MUSE_SINGLE_CLIP_LIMIT_SECS));
        let scope = format!(
            "THIS TURN: generate ONLY beats {first_beat}-{last_beat} (~{batch_secs} seconds), each beat a ~{MUSE_SINGLE_CLIP_LIMIT_SECS}-second clip with synchronized audio/narration and subtitles. Do not skip or drop any beat in this range.\n\nMandatory workflow:\n1. Generate the clips for beats {first_beat}-{last_beat}.\n2. Concatenate/merge them in order into ONE single continuous video.\n3. Send ONLY that merged video as the file attachment in the chat — do NOT attach the individual clip parts.\n4. Only after the merged video is attached, send a final confirmation message ending with {MEDIA_DONE_MARKER}."
        );
        let text = if index == 0 {
            format!(
                "Generate a long video (aspect ratio {ratio}, {secs} seconds in total = {total_beats} sequential ~{MUSE_SINGLE_CLIP_LIMIT_SECS}-second beats, delivered over {batch_count} turns in this chat). Full script: {}\n\n{scope}",
                prompt.trim()
            )
        } else {
            format!(
                "Continue the SAME video in this chat — do not restart and do not repeat earlier beats. Follow the full script from the first message, keeping the same visual style, characters and voice.\n\n{scope}"
            )
        };
        batches.push(MediaBatch {
            prompt: text,
            target_secs: Some(batch_secs),
        });
    }
    Some(batches)
}

const ORCHESTRATOR_BASE_PREFIX: &str = "<!-- base-instructions-version:";
const ORCHESTRATOR_BASE_END_MARKER: &str =
    "không chứng nhận yêu cầu nghiệp vụ đã đạt; hãy nêu rõ phần chưa làm được.";

fn remove_xml_section(input: &str, open_tag: &str, close_tag: &str) -> String {
    let mut out = input.to_string();
    while let Some(start) = out.find(open_tag) {
        if let Some(rel_end) = out[start..].find(close_tag) {
            let end = start + rel_end + close_tag.len();
            out.replace_range(start..end, "");
        } else {
            break;
        }
    }
    out
}

pub(crate) fn strip_orchestrator_system_boilerplate(raw: &str) -> String {
    let mut cleaned = remove_xml_section(raw, "<project_instructions>", "</project_instructions>");
    cleaned = remove_xml_section(&cleaned, "<available_skills>", "</available_skills>");
    let trimmed = cleaned.trim();
    if trimmed.starts_with(ORCHESTRATOR_BASE_PREFIX)
        && let Some(idx) = trimmed.find(ORCHESTRATOR_BASE_END_MARKER)
    {
        return trimmed[idx + ORCHESTRATOR_BASE_END_MARKER.len()..]
            .trim()
            .to_string();
    }
    trimmed.to_string()
}

pub fn flatten_messages(request: &LlmRequest) -> String {
    let mut blocks: Vec<(&'static str, String)> = Vec::new();
    for msg in &request.messages {
        let (label, text) = match msg {
            ChatMessage::System { content } => {
                let sanitized = strip_orchestrator_system_boilerplate(content);
                if sanitized.is_empty() {
                    continue;
                }
                blocks.push(("System", sanitized));
                continue;
            }
            ChatMessage::User { content } => ("User", content.trim()),
            ChatMessage::AssistantText { content } => ("Assistant", content.trim()),
            ChatMessage::AssistantToolCalls { calls } => {
                if calls.is_empty() {
                    continue;
                }
                let formatted = serde_json::to_string(calls).unwrap_or_default();
                blocks.push(("Assistant", formatted));
                continue;
            }
            ChatMessage::ToolResult { content, .. } => ("Tool", content.trim()),
        };
        if !text.is_empty() {
            blocks.push((label, text.to_string()));
        }
    }
    if blocks.is_empty() {
        return String::new();
    }
    if blocks.len() == 1 && blocks[0].0 == "User" {
        return blocks.remove(0).1;
    }
    let mut formatted: Vec<String> = blocks
        .into_iter()
        .map(|(role, text)| format!("[{role}]\n{text}"))
        .collect();
    formatted.push(
        "[Instruction]\nRespond to the last user message above, staying consistent with the conversation context."
            .to_string(),
    );
    formatted.join("\n\n")
}

pub(crate) fn extract_prior_turns_and_last_user(
    request: &LlmRequest,
) -> Option<(Vec<ConversationTurn>, String)> {
    let mut user_assistant_turns = Vec::new();
    let mut pending_user: Option<String> = None;

    for msg in &request.messages {
        match msg {
            ChatMessage::User { content } => {
                let text = content.trim();
                if text.is_empty() {
                    continue;
                }
                if let Some(prev_u) = pending_user.take() {
                    user_assistant_turns.push(ConversationTurn {
                        user: prev_u,
                        assistant: String::new(),
                    });
                }
                pending_user = Some(text.to_string());
            }
            ChatMessage::AssistantText { content } => {
                let text = content.trim();
                if text.is_empty() {
                    continue;
                }
                let u = pending_user.take()?;
                user_assistant_turns.push(ConversationTurn {
                    user: u,
                    assistant: text.to_string(),
                });
            }
            ChatMessage::System { .. }
            | ChatMessage::AssistantToolCalls { .. }
            | ChatMessage::ToolResult { .. } => {}
        }
    }

    let last_user = pending_user?;
    Some((user_assistant_turns, last_user))
}

pub(crate) fn followup_text(extra_turns: &[ConversationTurn], last_user_text: &str) -> String {
    if extra_turns.is_empty() {
        return last_user_text.to_string();
    }
    let mut blocks = Vec::new();
    for turn in extra_turns {
        blocks.push(format!("[User]\n{}", turn.user));
        if !turn.assistant.is_empty() {
            blocks.push(format!("[Assistant]\n{}", turn.assistant));
        }
    }
    blocks.push(format!("[User]\n{last_user_text}"));
    blocks.join("\n\n")
}
