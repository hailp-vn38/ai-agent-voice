mod chat;
mod media;
mod page;
mod png;
mod prompt;

use std::time::Duration;

use serde::Deserialize;

use crate::providers::llm::LlmError;

use super::cdp::CdpError;
use super::pool::UpstreamFailureKind;

pub(crate) use chat::run_chat_turn_once;
pub(crate) use media::run_media_turn_once;
pub use media::{pick_media_urls, sniff_media_mime_type};
#[allow(unused_imports)]
pub(crate) use png::{encode_rgba_png, remove_background_to_rgba_png};
pub use prompt::{
    aspect_ratio_hint, flatten_messages, format_image_prompt, format_video_prompt, is_thread_url,
};
#[allow(unused_imports)]
pub(crate) use prompt::{extract_prior_turns_and_last_user, followup_text};

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
pub(crate) struct PageState {
    #[serde(default)]
    pub ready: bool,
    #[serde(default, rename = "hasInput")]
    pub has_input: bool,
    #[serde(default)]
    pub head: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct AttachmentNode {
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
pub(crate) struct ChatState {
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
