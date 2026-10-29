use std::sync::Arc;
use std::time::Instant;

use super::super::cdp::BrowserPage;

pub const HOT_BUBBLE_LIMIT: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MusePoolKey {
    pub cdp_port: u16,
    pub headless: bool,
    pub account_concurrency: usize,
    /// SHA-256 digest of credential ensuring secret rotation creates isolated contexts.
    pub credential_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountHealth {
    Healthy,
    Cooldown(Instant),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamFailureKind {
    AuthInvalid,
    QuotaExceeded,
    Transient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationTurn {
    pub user: String,
    pub assistant: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HotConversation {
    pub hint: String,
    pub turns: Vec<ConversationTurn>,
    pub image_count: usize,
    pub thread_url: String,
}

pub(crate) struct PooledTab {
    pub page: Arc<dyn BrowserPage>,
    pub hot: Option<HotConversation>,
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum CheckoutMode {
    Stateless,
    Continuation {
        prior_turns: Vec<ConversationTurn>,
        image_count: usize,
    },
}
