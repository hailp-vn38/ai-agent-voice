use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::client::MuseVmClient;
use super::super::config::{MuseAccountCredential, has_required_cookies};
use super::types::{
    ConversationTurn, HOT_SESSION_IDLE_TTL, HotConversation, MAX_CONCURRENT_OPS_PER_ACCOUNT,
    MAX_HOT_SESSIONS_PER_ACCOUNT, QUOTA_COOLDOWN, TRANSIENT_COOLDOWN, UpstreamFailureKind,
};

#[derive(Debug, Clone)]
pub(crate) enum AccountStatus {
    Healthy,
    Cooldown {
        until: Instant,
        reason: UpstreamFailureKind,
    },
    AuthFailed,
}

pub(crate) struct MuseAccountState {
    pub credential: MuseAccountCredential,
    pub gateway_account_key: Option<String>,
    pub status: AccountStatus,
    pub vm_client: Option<Arc<MuseVmClient>>,
    pub active_ops: usize,
    pub hot_sessions: Vec<(HotConversation, Instant)>,
    pub warmup_started: bool,
}

impl MuseAccountState {
    pub fn new(credential: MuseAccountCredential) -> Self {
        Self {
            credential,
            gateway_account_key: None,
            status: AccountStatus::Healthy,
            vm_client: None,
            active_ops: 0,
            hot_sessions: Vec::new(),
            warmup_started: false,
        }
    }

    pub fn refresh_status(&mut self, now: Instant) {
        if let AccountStatus::Cooldown { until, .. } = self.status
            && now >= until
        {
            self.status = AccountStatus::Healthy;
        }
        self.hot_sessions
            .retain(|(_, updated_at)| now.duration_since(*updated_at) < HOT_SESSION_IDLE_TTL);
    }

    pub fn is_healthy(&mut self, now: Instant) -> bool {
        self.refresh_status(now);
        matches!(self.status, AccountStatus::Healthy)
    }

    pub fn has_capacity(&self) -> bool {
        self.active_ops < MAX_CONCURRENT_OPS_PER_ACCOUNT
    }

    pub fn take_matching_hot_session(
        &mut self,
        prior_turns: &[ConversationTurn],
        now: Instant,
    ) -> Option<HotConversation> {
        if prior_turns.is_empty() {
            return None;
        }
        self.refresh_status(now);
        let pos = self.hot_sessions.iter().position(|(hot, _)| {
            !hot.turns.is_empty()
                && prior_turns.len() >= hot.turns.len()
                && prior_turns[..hot.turns.len()] == hot.turns[..]
        })?;
        Some(self.hot_sessions.remove(pos).0)
    }

    pub fn save_hot_session(&mut self, hot: HotConversation, now: Instant) {
        self.refresh_status(now);
        self.hot_sessions
            .retain(|(existing, _)| existing.session_id != hot.session_id);
        self.hot_sessions.push((hot, now));
        if self.hot_sessions.len() > MAX_HOT_SESSIONS_PER_ACCOUNT {
            self.hot_sessions.remove(0);
        }
    }

    pub fn mark_failure(
        &mut self,
        kind: UpstreamFailureKind,
        now: Instant,
    ) -> Option<Arc<MuseVmClient>> {
        self.status = match kind {
            UpstreamFailureKind::AuthInvalid => AccountStatus::AuthFailed,
            UpstreamFailureKind::QuotaExceeded => AccountStatus::Cooldown {
                until: now + QUOTA_COOLDOWN,
                reason: kind,
            },
            UpstreamFailureKind::Transient => AccountStatus::Cooldown {
                until: now + TRANSIENT_COOLDOWN,
                reason: kind,
            },
        };
        if matches!(kind, UpstreamFailureKind::AuthInvalid)
            && has_required_cookies(&self.credential.cookies)
        {
            self.credential.wss_url = None;
            self.credential.wss_expires = None;
        }
        if matches!(
            kind,
            UpstreamFailureKind::AuthInvalid | UpstreamFailureKind::Transient
        ) {
            self.hot_sessions.clear();
            self.vm_client.take()
        } else {
            None
        }
    }

    pub fn cooldown_remaining(&self, now: Instant) -> Option<(UpstreamFailureKind, Duration)> {
        match self.status {
            AccountStatus::Cooldown { until, reason } if until > now => {
                Some((reason, until.duration_since(now)))
            }
            _ => None,
        }
    }
}
