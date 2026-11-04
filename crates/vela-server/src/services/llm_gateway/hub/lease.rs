use super::*;

impl LlmGatewayHub {
    /// Resolves the canonical default upstream base URL for a `GatewayBinding` by inspecting
    /// the account's `GatewayVendor` in the in-RAM snapshot rather than guessing from string prefixes.
    pub fn default_base_url_for_binding(&self, binding: &GatewayBinding) -> &'static str {
        match binding {
            GatewayBinding::Pool(vendor) => vendor.default_base_url(),
            GatewayBinding::Account(account_key) => {
                let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                guard
                    .accounts
                    .get(account_key)
                    .map(|acc| acc.vendor.default_base_url())
                    .unwrap_or_else(|| GatewayVendor::OpenaiCodex.default_base_url())
            }
        }
    }

    /// Hot-path synchronous (`0ms`) lease acquisition from the in-RAM `GatewayRuntimeSnapshot`.
    /// Never queries SQLite and never invokes `SecretResolver::resolve` during a voice or task request.
    pub fn acquire_lease(&self, binding: &GatewayBinding) -> Result<GatewayLease, GatewayError> {
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());

        match binding {
            GatewayBinding::Account(account_key) => {
                let acc = guard
                    .accounts
                    .get(account_key)
                    .ok_or(GatewayError::AccountNotFound)?;
                if !acc.enabled || acc.status == GatewayAccountStatus::Disabled {
                    return Err(GatewayError::AccountDisabled);
                }
                if acc.status == GatewayAccountStatus::ReauthRequired {
                    return Err(GatewayError::AccountReauthRequired);
                }
                if acc.cooldown_remaining_secs(now) > 0 {
                    return Err(GatewayError::AccountCoolingDown);
                }
                let bundle = acc
                    .bundle
                    .as_ref()
                    .ok_or(GatewayError::AccountReauthRequired)?;
                Ok(GatewayLease {
                    account_key: acc.account_key.clone(),
                    vendor: acc.vendor,
                    access_token: SecretString::new(bundle.access_token.clone()),
                    base_url: acc.vendor.default_base_url().to_owned(),
                    extra_headers: acc.extra_headers.clone(),
                    account_context_id: bundle.account_context_id.clone(),
                })
            }
            GatewayBinding::Pool(vendor) => {
                let keys = guard
                    .ordered_keys_by_vendor
                    .get(vendor)
                    .ok_or(GatewayError::PoolExhausted)?;
                let ready: Vec<&LoadedGatewayAccount> = keys
                    .iter()
                    .filter_map(|k| guard.accounts.get(k))
                    .filter(|acc| acc.is_ready_for_lease(now))
                    .collect();
                if ready.is_empty() {
                    let enabled_accounts: Vec<&LoadedGatewayAccount> = keys
                        .iter()
                        .filter_map(|k| guard.accounts.get(k))
                        .filter(|acc| acc.enabled && acc.status != GatewayAccountStatus::Disabled)
                        .collect();
                    if enabled_accounts.iter().any(|acc| {
                        acc.status == GatewayAccountStatus::Active
                            && acc.bundle.is_some()
                            && acc.cooldown_remaining_secs(now) > 0
                    }) {
                        return Err(GatewayError::AccountCoolingDown);
                    }
                    if enabled_accounts.iter().any(|acc| {
                        acc.status == GatewayAccountStatus::ReauthRequired || acc.bundle.is_none()
                    }) {
                        return Err(GatewayError::AccountReauthRequired);
                    }
                    return Err(GatewayError::PoolExhausted);
                }
                let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
                let quota_available: Vec<&LoadedGatewayAccount> = ready
                    .iter()
                    .copied()
                    .filter(|acc| {
                        quotas_guard
                            .get(&acc.account_key)
                            .is_none_or(|q| !q.limit_reached)
                    })
                    .collect();
                let candidates = if quota_available.is_empty() {
                    &ready
                } else {
                    &quota_available
                };
                let cursor = self
                    .pool_cursors
                    .get(vendor)
                    .map(|c| c.fetch_add(1, Ordering::Relaxed))
                    .unwrap_or(0);
                let chosen = candidates[cursor % candidates.len()];
                let bundle = chosen.bundle.as_ref().ok_or(GatewayError::PoolExhausted)?;
                Ok(GatewayLease {
                    account_key: chosen.account_key.clone(),
                    vendor: chosen.vendor,
                    access_token: SecretString::new(bundle.access_token.clone()),
                    base_url: chosen.vendor.default_base_url().to_owned(),
                    extra_headers: chosen.extra_headers.clone(),
                    account_context_id: bundle.account_context_id.clone(),
                })
            }
        }
    }

    /// Acquires the credential for `account_key` even when the account is in quota/rate-limit
    /// cooldown, so an in-flight Muse session can still be polled or reconciled without spending
    /// new generation quota.
    pub fn acquire_account_for_polling(
        &self,
        account_key: &str,
    ) -> Result<GatewayLease, GatewayError> {
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        let acc = guard
            .accounts
            .get(account_key)
            .ok_or(GatewayError::AccountNotFound)?;
        if !acc.enabled || acc.status == GatewayAccountStatus::Disabled {
            return Err(GatewayError::AccountDisabled);
        }
        if acc.status == GatewayAccountStatus::ReauthRequired {
            return Err(GatewayError::AccountReauthRequired);
        }
        let bundle = acc
            .bundle
            .as_ref()
            .ok_or(GatewayError::AccountReauthRequired)?;
        Ok(GatewayLease {
            account_key: acc.account_key.clone(),
            vendor: acc.vendor,
            access_token: SecretString::new(bundle.access_token.clone()),
            base_url: acc.vendor.default_base_url().to_owned(),
            extra_headers: acc.extra_headers.clone(),
            account_context_id: bundle.account_context_id.clone(),
        })
    }

    /// Updates in-RAM cooldown or reauth state immediately after an upstream LLM call completes.
    pub fn report_outcome(&self, account_key: &str, outcome: GatewayCallOutcome) {
        let now = Instant::now();
        let now_epoch = unix_seconds().unwrap_or(0);
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        let Some(acc) = guard.accounts.get_mut(account_key) else {
            return;
        };
        match outcome {
            GatewayCallOutcome::Success => {
                acc.cooldown_until = None;
                acc.last_used_at = Some(now_epoch);
                acc.last_error = None;
            }
            GatewayCallOutcome::RateLimited { retry_after_secs } => {
                let secs = retry_after_secs
                    .unwrap_or(DEFAULT_COOLDOWN_SECS)
                    .clamp(MIN_COOLDOWN_SECS, MAX_COOLDOWN_SECS);
                acc.cooldown_until = Some(now + Duration::from_secs(secs));
                acc.last_error = Some(format!("rate_limited_cooldown_{secs}s"));
            }
            GatewayCallOutcome::Unauthorized => {
                acc.status = GatewayAccountStatus::ReauthRequired;
                acc.last_error = Some("upstream_unauthorized".to_owned());
            }
        }
    }
}
