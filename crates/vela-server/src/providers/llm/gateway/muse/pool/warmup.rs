use super::SharedMusePool;

impl SharedMusePool {
    pub(super) fn trigger_account_warmup(&self, account_index: usize) {
        let should_spawn = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(account) = state.accounts.get_mut(account_index) else {
                return;
            };
            if account.warmup_started || account.vm_client.is_some() {
                false
            } else {
                account.warmup_started = true;
                true
            }
        };

        if !should_spawn {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let pool = self.clone();
            handle.spawn(async move {
                let _ = pool.ensure_account_client(account_index).await;
                let mut state = pool.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(acc) = state.accounts.get_mut(account_index) {
                    acc.warmup_started = false;
                }
            });
        }
    }
}
