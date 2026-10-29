use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::providers::llm::{LlmRequest, ReferenceImage};

use super::super::dom::{DENY_APPROVALS_JS, QUOTA_HINTS, STALL_HINTS, is_glitch_reply};
use super::super::pool::{
    CheckoutMode, ConversationTurn, HotConversation, SharedMusePool, UpstreamFailureKind,
};
use super::page::{
    attach_images, navigate_new_thread_and_wait_ready, poll_chat_state, send_prompt_verified,
    verify_hot_tab_live,
};
use super::prompt::{
    extract_prior_turns_and_last_user, flatten_messages, followup_text, is_thread_url,
};
use super::{POLL_INTERVAL, STABLE_POLLS_DONE, STABLE_POLLS_FORCE, WorkflowFailure};

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
