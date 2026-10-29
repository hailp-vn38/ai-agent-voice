use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use super::super::cdp::BrowserPage;
use super::types::{AccountHealth, HotConversation, PooledTab, UpstreamFailureKind};
use super::{PoolInner, spawn_page_close};

pub(crate) struct TabLease {
    pub(super) account_idx: usize,
    pub(super) tab: Option<PooledTab>,
    pub(super) matched_hot: Option<HotConversation>,
    pub(super) clean_return: bool,
    pub(super) inner: Arc<StdMutex<PoolInner>>,
    pub(super) notify: Arc<Notify>,
}

impl TabLease {
    pub fn page(&self) -> &Arc<dyn BrowserPage> {
        &self
            .tab
            .as_ref()
            .expect("TabLease always holds a tab while borrowed")
            .page
    }

    pub fn matched_hot(&self) -> Option<&HotConversation> {
        self.matched_hot.as_ref()
    }

    pub fn finish_clean(&mut self, hot: Option<HotConversation>) {
        if let Some(tab) = &mut self.tab {
            tab.hot = hot;
        }
        self.clean_return = true;
    }

    pub fn fail_account(&self, kind: UpstreamFailureKind) {
        let mut stale_pages = Vec::new();
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(self.account_idx) {
                acc.health = match kind {
                    UpstreamFailureKind::AuthInvalid => AccountHealth::Invalid,
                    UpstreamFailureKind::QuotaExceeded => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(3600))
                    }
                    UpstreamFailureKind::Transient => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(30))
                    }
                };
                if matches!(kind, UpstreamFailureKind::AuthInvalid) {
                    for idle in acc.idle_tabs.drain(..) {
                        acc.active_tabs = acc.active_tabs.saturating_sub(1);
                        stale_pages.push(idle.page);
                    }
                }
            }
        }
        for page in stale_pages {
            spawn_page_close(page);
        }
        self.notify.notify_waiters();
    }
}

impl Drop for TabLease {
    fn drop(&mut self) {
        let Some(tab) = self.tab.take() else {
            return;
        };
        let page_to_close = {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(self.account_idx) {
                if self.clean_return && !tab.stale && !matches!(acc.health, AccountHealth::Invalid)
                {
                    acc.idle_tabs.push(tab);
                    None
                } else {
                    acc.active_tabs = acc.active_tabs.saturating_sub(1);
                    Some(tab.page)
                }
            } else {
                Some(tab.page)
            }
        };
        self.notify.notify_waiters();
        if let Some(page) = page_to_close {
            spawn_page_close(page);
        }
    }
}
