use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;

use crate::providers::llm::{ChatMessage, LlmError, LlmOutput, LlmRequest, ReferenceImage};

use super::cdp::{BrowserPage, CdpError};
use super::config::NEW_THREAD_URL;
use super::dom::{
    CHAT_STATE_JS, CLICK_SEND_JS, DENY_APPROVALS_JS, GLITCH_HINTS, INPUT_EMPTY_JS,
    LAST_BUBBLE_MEDIA_JS, LOGIN_HINTS, PAGE_STATE_JS, QUOTA_HINTS, STALL_HINTS, attach_file_js,
    body_includes_js, fetch_as_base64_js, fill_input_js, is_glitch_reply,
};
use super::pool::{
    CheckoutMode, ConversationTurn, HOT_BUBBLE_LIMIT, HotConversation, SharedMusePool, TabLease,
    UpstreamFailureKind,
};

pub const POLL_INTERVAL: Duration = Duration::from_millis(150);
pub const MEDIA_POLL_INTERVAL: Duration = Duration::from_millis(600);
pub const STABLE_POLLS_DONE: usize = 4;
pub const STABLE_POLLS_FORCE: usize = 12;
pub const VIDEO_UNMARKED_STABLE_POLLS: usize = 100;
pub const CUTOUT_HINT: &str = " Isolated subject on a plain, uncluttered studio background.";
pub const MEDIA_DONE_MARKER: &str = "[ALL_MEDIA_ATTACHED]";
pub const MUSE_SINGLE_CLIP_LIMIT_SECS: u32 = 10;
pub const MUSE_MAX_CLIPS_PER_TURN: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
        }
    }

    pub const fn grace_duration(self, denied_approval: bool, has_done_marker: bool) -> Duration {
        match self {
            Self::Image => Duration::from_secs(8),
            Self::Video => {
                if denied_approval || has_done_marker {
                    Duration::from_secs(90)
                } else {
                    Duration::from_secs(180)
                }
            }
        }
    }

    pub const fn stall_after(self) -> Duration {
        match self {
            Self::Image => Duration::from_secs(120),
            Self::Video => Duration::from_secs(240),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct PageState {
    #[serde(default)]
    pub ready: bool,
    #[serde(default, rename = "hasInput")]
    pub has_input: bool,
    #[serde(default)]
    pub head: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub(super) struct AttachmentNode {
    #[serde(default)]
    pub tid: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub w: u32,
    #[serde(default)]
    pub h: u32,
    #[serde(default)]
    pub group: usize,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct ChatState {
    #[serde(default, rename = "agentCount")]
    pub agent_count: usize,
    #[serde(default, rename = "lastText")]
    pub last_text: String,
    #[serde(default)]
    pub bubbles: Vec<String>,
    #[serde(default)]
    pub generating: bool,
    #[serde(default)]
    pub approval: bool,
    #[serde(default)]
    pub attachments: Vec<AttachmentNode>,
    #[serde(default)]
    pub tail: String,
}

#[derive(Debug)]
pub(crate) enum WorkflowFailure {
    Upstream(UpstreamFailureKind, String),
    Rejected(String),
    Timeout(String),
    Cdp(CdpError),
}

impl From<CdpError> for WorkflowFailure {
    fn from(value: CdpError) -> Self {
        Self::Cdp(value)
    }
}

impl WorkflowFailure {
    pub(crate) fn into_llm_error(self) -> LlmError {
        match self {
            Self::Rejected(reason) => {
                tracing::warn!(reason = %reason, "Muse request rejected");
                LlmError::Rejected
            }
            Self::Upstream(kind, reason) => {
                tracing::warn!(?kind, reason = %reason, "Muse upstream failure");
                LlmError::Failed
            }
            Self::Timeout(reason) => {
                tracing::warn!(reason = %reason, "Muse operation timed out");
                LlmError::Failed
            }
            Self::Cdp(err) => {
                tracing::warn!(error = %err, "Muse CDP failure");
                LlmError::Failed
            }
        }
    }
}

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

pub(super) fn extract_prior_turns_and_last_user(
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

pub(super) fn followup_text(extra_turns: &[ConversationTurn], last_user_text: &str) -> String {
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

async fn poll_page_state(page: &Arc<dyn BrowserPage>) -> Result<PageState, CdpError> {
    let val = page
        .evaluate_json(PAGE_STATE_JS, false, Duration::from_secs(5))
        .await?;
    Ok(serde_json::from_value(val).unwrap_or_default())
}

async fn poll_chat_state(page: &Arc<dyn BrowserPage>) -> Result<ChatState, CdpError> {
    let val = page
        .evaluate_json(CHAT_STATE_JS, false, Duration::from_secs(5))
        .await?;
    Ok(serde_json::from_value(val).unwrap_or_default())
}

async fn navigate_new_thread_and_wait_ready(
    page: &Arc<dyn BrowserPage>,
    ready_timeout: Duration,
) -> Result<(), WorkflowFailure> {
    page.navigate(NEW_THREAD_URL).await?;
    let deadline = Instant::now() + ready_timeout;
    let mut last_state = PageState::default();
    loop {
        if let Ok(state) = poll_page_state(page).await {
            if state.ready {
                return Ok(());
            }
            last_state = state;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let head_lower = last_state.head.to_ascii_lowercase();
    if LOGIN_HINTS.iter().any(|h| head_lower.contains(h)) {
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::AuthInvalid,
            "muse.ai redirected to login; session cookies are invalid".to_string(),
        ));
    }
    Err(WorkflowFailure::Upstream(
        UpstreamFailureKind::Transient,
        "muse.ai page did not become ready in time".to_string(),
    ))
}

async fn verify_hot_tab_live(page: &Arc<dyn BrowserPage>, hot: &HotConversation) -> bool {
    let Ok(url) = page.current_url().await else {
        return false;
    };
    let on_saved_url = is_thread_url(&url)
        && (hot.thread_url.is_empty()
            || url.trim_end_matches('/') == hot.thread_url.trim_end_matches('/'));
    let Ok(page_state) = poll_page_state(page).await else {
        return false;
    };
    if !page_state.has_input || page_state.head.contains("Connecting...") {
        return false;
    }
    let Ok(chat_state) = poll_chat_state(page).await else {
        return false;
    };
    if chat_state.generating
        || chat_state.agent_count >= HOT_BUBBLE_LIMIT
        || STALL_HINTS.iter().any(|h| chat_state.tail.contains(h))
    {
        return false;
    }
    if on_saved_url {
        return true;
    }
    if let Some(last_turn) = hot.turns.last() {
        let snippet: String = last_turn.user.chars().take(80).collect();
        if let Ok(val) = page
            .evaluate_json(&body_includes_js(&snippet), false, Duration::from_secs(5))
            .await
        {
            return val.as_bool().unwrap_or(false);
        }
    }
    false
}

async fn attach_images(
    page: &Arc<dyn BrowserPage>,
    images: &[ReferenceImage],
) -> Result<(), WorkflowFailure> {
    if images.is_empty() {
        return Ok(());
    }
    for (idx, img) in images.iter().enumerate() {
        let ext = match img.mime_type.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            _ => "png",
        };
        let filename = format!("input_{idx}.{ext}");
        let b64 = BASE64.encode(&img.bytes);
        let script = attach_file_js(&b64, &img.mime_type, &filename);
        let res = page
            .evaluate_json(&script, false, Duration::from_secs(15))
            .await?;
        if !res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let err_msg = res
                .get("err")
                .and_then(Value::as_str)
                .unwrap_or("unknown attach error");
            return Err(WorkflowFailure::Upstream(
                UpstreamFailureKind::Transient,
                format!("failed to attach reference image {filename}: {err_msg}"),
            ));
        }
    }
    tokio::time::sleep(Duration::from_millis(1000)).await;
    Ok(())
}

async fn send_prompt_verified(
    page: &Arc<dyn BrowserPage>,
    prompt: &str,
) -> Result<(), WorkflowFailure> {
    let fill_res = page
        .evaluate_json(&fill_input_js(prompt), false, Duration::from_secs(10))
        .await?;
    if !fill_res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err_msg = fill_res
            .get("err")
            .and_then(Value::as_str)
            .unwrap_or("no-input");
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            format!("cannot fill chat input: {err_msg}"),
        ));
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut clicked = false;
    while Instant::now() < deadline {
        let click_val = page
            .evaluate_json(CLICK_SEND_JS, false, Duration::from_secs(5))
            .await?;
        let is_clicked = click_val.as_str() == Some("clicked") || click_val.as_bool() == Some(true);
        if is_clicked {
            clicked = true;
            break;
        }
        if click_val.as_str() == Some("missing") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    if !clicked {
        page.press_enter().await?;
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    let empty_val = page
        .evaluate_json(INPUT_EMPTY_JS, false, Duration::from_secs(5))
        .await?;
    if !empty_val.as_bool().unwrap_or(false) {
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            "send button not found and Enter did not clear textarea".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn run_chat_turn_once<F>(
    pool: &Arc<SharedMusePool>,
    request: &LlmRequest,
    images: &[ReferenceImage],
    timeout: Duration,
    mut on_delta: F,
) -> Result<String, WorkflowFailure>
where
    F: FnMut(String),
{
    let turn_info = extract_prior_turns_and_last_user(request);
    let checkout_mode = match &turn_info {
        Some((prior_turns, _)) if !prior_turns.is_empty() => CheckoutMode::Continuation {
            prior_turns: prior_turns.clone(),
            image_count: images.len(),
        },
        _ => CheckoutMode::Stateless,
    };

    let mut lease = pool.checkout(checkout_mode).await?;
    let page = Arc::clone(lease.page());

    let matched_hot = lease.matched_hot().cloned();
    let on_thread = if let Some(ref hot) = matched_hot {
        verify_hot_tab_live(&page, hot).await
    } else {
        false
    };

    let prompt_to_send = if on_thread {
        let hot = matched_hot.as_ref().expect("checked above");
        let (prior_turns, last_user) = turn_info.as_ref().expect("checked above");
        followup_text(&prior_turns[hot.turns.len()..], last_user)
    } else {
        if let Err(err) = navigate_new_thread_and_wait_ready(&page, Duration::from_secs(30)).await {
            if let WorkflowFailure::Upstream(kind, _) = &err {
                lease.fail_account(*kind);
            }
            return Err(err);
        }
        flatten_messages(request)
    };

    let base_state = poll_chat_state(&page).await?;
    if !on_thread && !images.is_empty() {
        if let Err(err) = attach_images(&page, images).await {
            if let WorkflowFailure::Upstream(kind, _) = &err {
                lease.fail_account(*kind);
            }
            return Err(err);
        }
    }

    if let Err(err) = send_prompt_verified(&page, &prompt_to_send).await {
        if let WorkflowFailure::Upstream(kind, _) = &err {
            lease.fail_account(*kind);
        }
        return Err(err);
    }

    let base_agent_count = base_state.agent_count;
    let base_text = base_state.last_text;
    let started = Instant::now();
    let mut emitted_text = String::new();
    let mut last_seen_text = String::new();
    let mut stable_polls = 0usize;

    loop {
        if started.elapsed() >= timeout {
            return Err(WorkflowFailure::Timeout(
                "chat generation timed out".to_string(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let state = poll_chat_state(&page).await?;
        if state.approval {
            let _ = page
                .evaluate_json(DENY_APPROVALS_JS, false, Duration::from_secs(5))
                .await;
            continue;
        }

        let is_new = state.agent_count > base_agent_count
            || (!state.last_text.is_empty() && state.last_text != base_text);
        if !is_new || state.last_text.is_empty() {
            if started.elapsed() > Duration::from_secs(15)
                && STALL_HINTS.iter().any(|h| state.tail.contains(h))
            {
                lease.fail_account(UpstreamFailureKind::Transient);
                return Err(WorkflowFailure::Upstream(
                    UpstreamFailureKind::Transient,
                    "upstream workspace is stuck connecting".to_string(),
                ));
            }
            continue;
        }

        let lower_reply = state.last_text.to_ascii_lowercase();
        if QUOTA_HINTS.iter().any(|h| lower_reply.contains(h)) {
            lease.fail_account(UpstreamFailureKind::QuotaExceeded);
            return Err(WorkflowFailure::Upstream(
                UpstreamFailureKind::QuotaExceeded,
                format!("muse.ai account quota exceeded: {}", state.last_text),
            ));
        }

        if is_glitch_reply(&state.last_text) {
            lease.fail_account(UpstreamFailureKind::Transient);
            return Err(WorkflowFailure::Upstream(
                UpstreamFailureKind::Transient,
                format!("muse.ai glitch reply: {}", state.last_text),
            ));
        }

        if state.last_text != last_seen_text {
            let delta = if let Some(suffix) = state.last_text.strip_prefix(&emitted_text) {
                suffix.to_string()
            } else if state.last_text.len() > emitted_text.len() {
                let boundary = state.last_text.floor_char_boundary(emitted_text.len());
                state.last_text[boundary..].to_string()
            } else {
                String::new()
            };
            if !delta.is_empty() {
                emitted_text = state.last_text.clone();
                on_delta(delta);
            }
            last_seen_text = state.last_text.clone();
            stable_polls = 0;
        } else {
            stable_polls += 1;
            if (!state.generating && stable_polls >= STABLE_POLLS_DONE)
                || stable_polls >= STABLE_POLLS_FORCE
            {
                break;
            }
        }
    }

    let current_url = page.current_url().await.unwrap_or_default();
    let updated_hot = if is_thread_url(&current_url) {
        let mut turns = turn_info
            .as_ref()
            .map(|(t, _)| t.clone())
            .unwrap_or_default();
        let last_u = turn_info
            .map(|(_, u)| u)
            .unwrap_or_else(|| prompt_to_send.clone());
        turns.push(ConversationTurn {
            user: last_u,
            assistant: last_seen_text.clone(),
        });
        Some(HotConversation {
            hint: String::new(),
            turns,
            image_count: images.len(),
            thread_url: current_url,
        })
    } else {
        None
    };

    lease.finish_clean(updated_hot);
    Ok(last_seen_text)
}

pub(crate) async fn run_media_turn_once(
    pool: &Arc<SharedMusePool>,
    kind: MediaKind,
    formatted_prompt: &str,
    images: &[ReferenceImage],
    transparent_background: bool,
    timeout: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    let mut lease = pool.checkout(CheckoutMode::Stateless).await?;
    let page = Arc::clone(lease.page());

    if let Err(err) = navigate_new_thread_and_wait_ready(&page, Duration::from_secs(30)).await {
        if let WorkflowFailure::Upstream(k, _) = &err {
            lease.fail_account(*k);
        }
        return Err(err);
    }

    let base_state = poll_chat_state(&page).await?;
    if let Err(err) = attach_images(&page, images).await {
        if let WorkflowFailure::Upstream(k, _) = &err {
            lease.fail_account(*k);
        }
        return Err(err);
    }

    if let Err(err) = send_prompt_verified(&page, formatted_prompt).await {
        if let WorkflowFailure::Upstream(k, _) = &err {
            lease.fail_account(*k);
        }
        return Err(err);
    }

    wait_and_download_media(
        &mut lease,
        &page,
        base_state,
        kind,
        transparent_background,
        timeout,
    )
    .await
}

fn format_clean_assistant_bubbles(bubbles: &[&str]) -> Option<String> {
    let cleaned: Vec<String> = bubbles
        .iter()
        .map(|b| b.replace(MEDIA_DONE_MARKER, "").trim().to_string())
        .filter(|b| !b.is_empty())
        .collect();
    (!cleaned.is_empty()).then(|| cleaned.join("\n\n"))
}

fn select_effective_ready_attachments<'a>(
    ready_atts: &[&'a AttachmentNode],
) -> Vec<&'a AttachmentNode> {
    if ready_atts.len() <= 1 {
        return ready_atts.to_vec();
    }
    let last_group = ready_atts.last().map(|a| a.group).unwrap_or(0);
    let last_group_atts: Vec<&'a AttachmentNode> = ready_atts
        .iter()
        .copied()
        .filter(|a| a.group == last_group)
        .collect();
    let has_earlier_group = ready_atts.iter().any(|a| a.group < last_group);
    if has_earlier_group && last_group_atts.len() == 1 {
        last_group_atts
    } else {
        ready_atts.to_vec()
    }
}

async fn wait_and_download_media(
    lease: &mut TabLease,
    page: &Arc<dyn BrowserPage>,
    base_state: ChatState,
    kind: MediaKind,
    transparent_background: bool,
    timeout: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    let base_attachments = base_state.attachments.len();
    let mut base_agent_count = base_state.agent_count;
    let stall_after = kind.stall_after();
    let overrun_limit = timeout.mul_f64(2.5);

    let started = Instant::now();
    let mut changed_at = started;
    let mut text_done_at: Option<Instant> = None;
    let mut denied_approval = false;
    let mut asked_to_attach = false;
    let mut busy = false;
    let mut loading = false;
    let mut stable_polls = 0usize;
    let mut last_sig: Option<(usize, String, Vec<usize>, Vec<String>, bool)> = None;

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(started);
        let quiet = now.duration_since(changed_at);

        if elapsed >= timeout
            && !(busy && elapsed < overrun_limit && (loading || quiet < stall_after))
        {
            return Err(WorkflowFailure::Timeout(format!(
                "{} generation timed out",
                kind.as_str()
            )));
        }

        tokio::time::sleep(MEDIA_POLL_INTERVAL).await;
        let state = poll_chat_state(page).await?;

        // 1. Auto-deny inline approval card.
        if state.approval {
            let _ = page
                .evaluate_json(DENY_APPROVALS_JS, false, Duration::from_secs(5))
                .await;
            denied_approval = true;
            text_done_at = None;
            continue;
        }

        // 2. Detect newly rendered media attachment(s) and agent text bubbles.
        let slice_start = base_attachments.min(state.attachments.len());
        let new_atts = &state.attachments[slice_start..];
        let mut seen_srcs = std::collections::HashSet::new();
        let ready_atts: Vec<&AttachmentNode> = new_atts
            .iter()
            .filter(|a| a.kind == kind.as_str() && !a.src.is_empty())
            .filter(|a| seen_srcs.insert(a.src.as_str()))
            .collect();
        let any_matching_loading = new_atts.iter().any(|a| {
            (a.kind == kind.as_str() || a.tid.contains(kind.as_str())) && a.src.is_empty()
        });
        let new_bubbles: Vec<&str> = state
            .bubbles
            .iter()
            .skip(base_agent_count)
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let has_done_marker = new_bubbles.iter().any(|b| b.contains(MEDIA_DONE_MARKER))
            || state.last_text.contains(MEDIA_DONE_MARKER);

        let sig = (
            state.agent_count,
            state.last_text.clone(),
            new_bubbles.iter().map(|s| s.len()).collect::<Vec<_>>(),
            new_atts
                .iter()
                .map(|a| format!("{}:{}:{}:{}", a.group, a.tid, a.kind, a.src))
                .collect::<Vec<_>>(),
            state.generating,
        );
        if last_sig.as_ref() != Some(&sig) {
            last_sig = Some(sig);
            stable_polls = 0;
            changed_at = Instant::now();
        } else {
            stable_polls += 1;
        }

        let required_stable_polls = match kind {
            MediaKind::Image => 0,
            MediaKind::Video => {
                if asked_to_attach {
                    0
                } else if has_done_marker {
                    STABLE_POLLS_DONE
                } else {
                    VIDEO_UNMARKED_STABLE_POLLS
                }
            }
        };
        if !ready_atts.is_empty()
            && !state.generating
            && !any_matching_loading
            && stable_polls >= required_stable_polls
        {
            let selected_atts = select_effective_ready_attachments(&ready_atts);
            let mut parts = Vec::with_capacity(selected_atts.len());
            for att in &selected_atts {
                parts.push(
                    download_media_part_from_src(page, &att.src, kind, transparent_background)
                        .await?,
                );
            }
            let assistant_message = format_clean_assistant_bubbles(&new_bubbles);
            let output =
                LlmOutput::from_media_parts(parts, assistant_message).ok_or_else(|| {
                    WorkflowFailure::Upstream(
                        UpstreamFailureKind::Transient,
                        format!("no {} parts downloaded", kind.as_str()),
                    )
                })?;
            lease.finish_clean(None);
            return Ok(output);
        }

        loading = any_matching_loading
            || !ready_atts.is_empty()
            || new_atts.iter().any(|a| a.tid.contains(kind.as_str()));
        busy = state.generating || loading;

        if loading {
            text_done_at = None;
            continue;
        }

        // 3. Agent finished text reply without attachment yet.
        if state.agent_count > base_agent_count && !state.last_text.is_empty() && !state.generating
        {
            let lower_text = state.last_text.to_ascii_lowercase();
            if QUOTA_HINTS.iter().any(|h| lower_text.contains(h)) {
                lease.fail_account(UpstreamFailureKind::QuotaExceeded);
                return Err(WorkflowFailure::Upstream(
                    UpstreamFailureKind::QuotaExceeded,
                    format!("muse.ai account quota exceeded: {}", state.last_text),
                ));
            }

            let effective_grace = kind.grace_duration(denied_approval, has_done_marker);
            let done_at = *text_done_at.get_or_insert_with(Instant::now);
            if done_at.elapsed() >= effective_grace {
                if GLITCH_HINTS.iter().any(|h| lower_text.contains(h)) {
                    lease.fail_account(UpstreamFailureKind::Transient);
                    return Err(WorkflowFailure::Upstream(
                        UpstreamFailureKind::Transient,
                        format!("muse.ai failed while responding: {}", state.last_text),
                    ));
                }

                if !asked_to_attach {
                    let followup = if denied_approval {
                        format!(
                            "Please attach the final {} file here in the chat instead of uploading it.",
                            kind.as_str()
                        )
                    } else if kind == MediaKind::Video {
                        format!(
                            "Please attach only the final merged video file here in the chat as a single attachment and end your confirmation message with {MEDIA_DONE_MARKER}."
                        )
                    } else {
                        format!(
                            "Please attach the final {} file here in the chat, as a single attachment.",
                            kind.as_str()
                        )
                    };
                    send_prompt_verified(page, &followup).await?;
                    asked_to_attach = true;
                    text_done_at = None;
                    base_agent_count = state.agent_count;
                    continue;
                }

                // Fallback: scan last bubble for direct media links.
                let probe = page
                    .evaluate_json(LAST_BUBBLE_MEDIA_JS, false, Duration::from_secs(5))
                    .await
                    .unwrap_or_default();
                let links: Vec<String> = probe
                    .get("links")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();

                let urls = pick_media_urls(&links, kind);
                if !urls.is_empty() {
                    let mut parts = Vec::with_capacity(urls.len());
                    for url in &urls {
                        parts.push(
                            download_media_part_from_src(page, url, kind, transparent_background)
                                .await?,
                        );
                    }
                    let assistant_message = format_clean_assistant_bubbles(&new_bubbles);
                    lease.finish_clean(None);
                    return LlmOutput::from_media_parts(parts, assistant_message).ok_or_else(
                        || {
                            WorkflowFailure::Upstream(
                                UpstreamFailureKind::Transient,
                                "empty media parts returned by muse.ai".to_owned(),
                            )
                        },
                    );
                }

                return Err(WorkflowFailure::Rejected(format!(
                    "muse.ai replied with text only without attaching {}: {}",
                    kind.as_str(),
                    state.last_text
                )));
            }
        } else {
            text_done_at = None;
        }
    }
}

async fn download_media_part_from_src(
    page: &Arc<dyn BrowserPage>,
    src: &str,
    kind: MediaKind,
    transparent_background: bool,
) -> Result<crate::providers::llm::MediaPart, WorkflowFailure> {
    let res = page
        .evaluate_json(&fetch_as_base64_js(src), true, Duration::from_secs(120))
        .await?;
    if !res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err_msg = res
            .get("err")
            .and_then(Value::as_str)
            .unwrap_or("fetch_as_base64 failed");
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            format!("failed to download generated {}: {err_msg}", kind.as_str()),
        ));
    }
    let b64 = res.get("b64").and_then(Value::as_str).unwrap_or_default();
    let raw_bytes = BASE64.decode(b64).map_err(|err| {
        WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            format!("invalid base64 media payload: {err}"),
        )
    })?;
    let declared_mime = res.get("mime").and_then(Value::as_str).unwrap_or_default();

    let (bytes, mime_type) = if kind == MediaKind::Image && transparent_background {
        let png_bytes = remove_background_to_rgba_png(&raw_bytes)?;
        (png_bytes, "image/png".to_string())
    } else {
        let mime = sniff_media_mime_type(&raw_bytes, declared_mime, kind);
        (raw_bytes, mime)
    };

    Ok(crate::providers::llm::MediaPart { bytes, mime_type })
}

pub fn pick_media_urls(links: &[String], kind: MediaKind) -> Vec<String> {
    const VIDEO_EXT: &[&str] = &[".mp4", ".webm", ".mov", ".m4v"];
    const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".webp", ".gif", ".avif"];
    const MEDIA_HOSTS: &[&str] = &[
        "videodelivery.net",
        "cloudflarestream.com",
        "cloudflarestorage.com",
        "amazonaws.com",
        "storage.googleapis.com",
        "blob.core.windows.net",
        "musecdn",
        "cdn.muse",
        "muse-cdn",
    ];
    const MEDIA_PATHS: &[&str] = &[
        "/media/",
        "/download",
        "/dl/",
        "/video",
        "/attachment",
        "/files/",
    ];

    let exts = match kind {
        MediaKind::Video => VIDEO_EXT,
        MediaKind::Image => IMAGE_EXT,
    };

    let score = |u: &str| -> u8 {
        let low = u.to_ascii_lowercase();
        let path = low
            .split('?')
            .next()
            .unwrap_or(&low)
            .split('#')
            .next()
            .unwrap_or(&low);
        if u.starts_with("blob:") || exts.iter().any(|ext| path.ends_with(ext)) {
            3
        } else if MEDIA_HOSTS.iter().any(|h| low.contains(h)) {
            2
        } else if MEDIA_PATHS.iter().any(|p| low.contains(p)) {
            1
        } else {
            0
        }
    };

    let valid: Vec<(&String, u8)> = links
        .iter()
        .filter(|u| !u.is_empty() && !u.starts_with("mailto:") && !u.starts_with("javascript:"))
        .map(|u| (u, score(u)))
        .filter(|(_, s)| *s > 0)
        .collect();
    let Some(max_score) = valid.iter().map(|(_, s)| *s).max() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    valid
        .into_iter()
        .filter(|(u, s)| *s == max_score && seen.insert((*u).as_str()))
        .map(|(u, _)| u.clone())
        .collect()
}

pub fn sniff_media_mime_type(bytes: &[u8], declared: &str, kind: MediaKind) -> String {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png".to_string();
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return "video/mp4".to_string();
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return "video/webm".to_string();
    }
    let trimmed = declared.trim();
    if !trimmed.is_empty() && trimmed != "application/octet-stream" {
        return trimmed.to_string();
    }
    match kind {
        MediaKind::Image => "image/png".to_string(),
        MediaKind::Video => "video/mp4".to_string(),
    }
}

pub(super) fn remove_background_to_rgba_png(bytes: &[u8]) -> Result<Vec<u8>, WorkflowFailure> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let Some((w, h, mut rgba)) = decode_uncompressed_or_simple_png(bytes) else {
        if bytes.starts_with(PNG_SIG) {
            return Ok(bytes.to_vec());
        }
        return Err(WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            "failed to decode image for background removal".to_string(),
        ));
    };
    if w == 0 || h == 0 {
        return Ok(bytes.to_vec());
    }

    let pixel_at = |buf: &[u8], x: u32, y: u32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    };

    let corners = [
        pixel_at(&rgba, 0, 0),
        pixel_at(&rgba, w - 1, 0),
        pixel_at(&rgba, 0, h - 1),
        pixel_at(&rgba, w - 1, h - 1),
    ];
    let bg = [
        ((corners[0][0] as u32
            + corners[1][0] as u32
            + corners[2][0] as u32
            + corners[3][0] as u32)
            / 4) as u8,
        ((corners[0][1] as u32
            + corners[1][1] as u32
            + corners[2][1] as u32
            + corners[3][1] as u32)
            / 4) as u8,
        ((corners[0][2] as u32
            + corners[1][2] as u32
            + corners[2][2] as u32
            + corners[3][2] as u32)
            / 4) as u8,
    ];

    let color_dist_sq = |p: [u8; 4]| -> u32 {
        let dr = p[0] as i32 - bg[0] as i32;
        let dg = p[1] as i32 - bg[1] as i32;
        let db = p[2] as i32 - bg[2] as i32;
        (dr * dr + dg * dg + db * db) as u32
    };

    let hard_thresh_sq = 32u32 * 32u32;
    let soft_thresh_sq = 64u32 * 64u32;
    let mut visited = vec![false; (w * h) as usize];
    let mut queue = VecDeque::new();

    for x in 0..w {
        queue.push_back((x, 0));
        queue.push_back((x, h - 1));
    }
    for y in 0..h {
        queue.push_back((0, y));
        queue.push_back((w - 1, y));
    }

    while let Some((x, y)) = queue.pop_front() {
        let idx = (y * w + x) as usize;
        if visited[idx] {
            continue;
        }
        let px = pixel_at(&rgba, x, y);
        let d = color_dist_sq(px);
        if d > soft_thresh_sq {
            continue;
        }
        visited[idx] = true;
        let alpha = if d <= hard_thresh_sq {
            0u8
        } else {
            let ratio = ((d - hard_thresh_sq) as f32) / ((soft_thresh_sq - hard_thresh_sq) as f32);
            (ratio.clamp(0.0, 1.0) * 255.0) as u8
        };
        let off = idx * 4;
        rgba[off + 3] = alpha;

        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < w {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < h {
            queue.push_back((x, y + 1));
        }
    }

    Ok(encode_rgba_png(w, h, &rgba))
}

fn decode_uncompressed_or_simple_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 33 {
        return None;
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat = Vec::new();

    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        if pos + 12 + len > bytes.len() {
            break;
        }
        let data = &bytes[pos + 8..pos + 8 + len];
        match chunk_type {
            b"IHDR" if data.len() >= 13 => {
                width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                height = u32::from_be_bytes(data[4..8].try_into().ok()?);
                let bit_depth = data[8];
                color_type = data[9];
                if bit_depth != 8 || !matches!(color_type, 2 | 6) {
                    return None;
                }
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }

    if width == 0 || height == 0 || idat.len() < 7 {
        return None;
    }
    // Stored zlib block decoding (BTYPE=00)
    let mut raw = Vec::new();
    let mut zpos = 2usize;
    while zpos + 5 <= idat.len().saturating_sub(4) {
        let header = idat[zpos];
        let btype = (header >> 1) & 0x03;
        if btype != 0 {
            return None;
        }
        let blen = u16::from_le_bytes([idat[zpos + 1], idat[zpos + 2]]) as usize;
        zpos += 5;
        if zpos + blen > idat.len().saturating_sub(4) {
            return None;
        }
        raw.extend_from_slice(&idat[zpos..zpos + blen]);
        zpos += blen;
        if (header & 1) != 0 {
            break;
        }
    }

    let bpp = if color_type == 6 { 4usize } else { 3usize };
    let stride = 1 + (width as usize) * bpp;
    if raw.len() < stride * (height as usize) {
        return None;
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..(height as usize) {
        let row = &raw[y * stride..y * stride + stride];
        if row[0] != 0 {
            return None;
        }
        for x in 0..(width as usize) {
            let p = &row[1 + x * bpp..1 + (x + 1) * bpp];
            rgba.push(p[0]);
            rgba.push(p[1]);
            rgba.push(p[2]);
            rgba.push(if bpp == 4 { p[3] } else { 255 });
        }
    }
    Some((width, height, rgba))
}

pub(super) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 4 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0u8); // filter type 0 (None)
        let start = (y * width * 4) as usize;
        let end = start + (width * 4) as usize;
        raw.extend_from_slice(&rgba[start..end]);
    }

    // Stored zlib stream
    let mut zlib = Vec::with_capacity(raw.len() + 32);
    zlib.push(0x78);
    zlib.push(0x01);
    let chunks: Vec<&[u8]> = if raw.is_empty() {
        vec![&[]]
    } else {
        raw.chunks(65_535).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i + 1 == chunks.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &zlib);
    write_png_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
