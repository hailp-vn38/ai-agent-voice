use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use tokio::time::Instant;

use crate::providers::llm::{LlmOutput, MediaPart, ReferenceImage};

use super::super::cdp::BrowserPage;
use super::super::dom::{
    DENY_APPROVALS_JS, GLITCH_HINTS, LAST_BUBBLE_MEDIA_JS, QUOTA_HINTS, fetch_as_base64_js,
};
use super::super::pool::{CheckoutMode, SharedMusePool, TabLease, UpstreamFailureKind};
use super::page::{
    attach_images, navigate_new_thread_and_wait_ready, poll_chat_state, send_prompt_verified,
};
use super::png::remove_background_to_rgba_png;
use super::{
    AttachmentNode, ChatState, MEDIA_DONE_MARKER, MEDIA_POLL_INTERVAL, MediaKind,
    STABLE_POLLS_DONE, VIDEO_UNMARKED_STABLE_POLLS, WorkflowFailure,
};

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
        let within_video_grace = kind == MediaKind::Video
            && state.agent_count > base_agent_count
            && text_done_at.is_none_or(|t| {
                t.elapsed() < kind.grace_duration(denied_approval, has_done_marker)
            });
        busy = state.generating || loading || within_video_grace;

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
) -> Result<MediaPart, WorkflowFailure> {
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

    Ok(MediaPart { bytes, mime_type })
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
