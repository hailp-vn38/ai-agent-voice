use crate::providers::llm::{ChatMessage, LlmRequest};

use super::super::config::NEW_THREAD_URL;
use super::super::pool::ConversationTurn;
use super::{CUTOUT_HINT, MEDIA_DONE_MARKER, MUSE_MAX_CLIPS_PER_TURN, MUSE_SINGLE_CLIP_LIMIT_SECS};

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

pub fn is_thread_url(url: &str) -> bool {
    let trimmed = url.trim().trim_end_matches('/');
    trimmed.starts_with("https://muse.ai/thread/") && trimmed != NEW_THREAD_URL
}

pub fn flatten_messages(request: &LlmRequest) -> String {
    let mut blocks: Vec<(&'static str, String)> = Vec::new();
    for msg in &request.messages {
        let (label, text) = match msg {
            ChatMessage::System { content } => ("System", content.trim()),
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
