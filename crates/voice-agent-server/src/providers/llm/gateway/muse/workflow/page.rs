use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use tokio::time::Instant;

use crate::providers::llm::ReferenceImage;

use super::super::cdp::{BrowserPage, CdpError};
use super::super::config::NEW_THREAD_URL;
use super::super::dom::{
    CHAT_STATE_JS, CLICK_SEND_JS, INPUT_EMPTY_JS, LOGIN_HINTS, PAGE_STATE_JS, STALL_HINTS,
    attach_file_js, body_includes_js, fill_input_js,
};
use super::super::pool::{HOT_BUBBLE_LIMIT, HotConversation, UpstreamFailureKind};
use super::prompt::is_thread_url;
use super::{ChatState, PageState, WorkflowFailure};

pub(super) async fn poll_page_state(page: &Arc<dyn BrowserPage>) -> Result<PageState, CdpError> {
    let val = page
        .evaluate_json(PAGE_STATE_JS, false, Duration::from_secs(5))
        .await?;
    Ok(serde_json::from_value(val).unwrap_or_default())
}

pub(super) async fn poll_chat_state(page: &Arc<dyn BrowserPage>) -> Result<ChatState, CdpError> {
    let val = page
        .evaluate_json(CHAT_STATE_JS, false, Duration::from_secs(5))
        .await?;
    Ok(serde_json::from_value(val).unwrap_or_default())
}

pub(super) async fn navigate_new_thread_and_wait_ready(
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

pub(super) async fn verify_hot_tab_live(
    page: &Arc<dyn BrowserPage>,
    hot: &HotConversation,
) -> bool {
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

pub(super) async fn attach_images(
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

pub(super) async fn send_prompt_verified(
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
