use std::time::Duration;

pub(super) const MAX_CONCURRENT_OPS_PER_ACCOUNT: usize = 8;
pub(super) const MAX_HOT_SESSIONS_PER_ACCOUNT: usize = 16;
pub(super) const HOT_SESSION_IDLE_TTL: Duration = Duration::from_secs(15 * 60);
pub(super) const QUOTA_COOLDOWN: Duration = Duration::from_secs(30 * 60);
pub(super) const TRANSIENT_COOLDOWN: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamFailureKind {
    QuotaExceeded,
    AuthInvalid,
    Transient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationTurn {
    pub user: String,
    pub assistant: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HotConversation {
    pub turns: Vec<ConversationTurn>,
    pub image_count: usize,
    pub session_id: String,
}
