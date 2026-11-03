use super::*;

impl LlmGatewayHub {
    /// Out-of-band proactive renewal for accounts whose `expires_at` is within `margin_secs`,
    /// plus self-healing watchdog recovery for cookie-backed Muse accounts stuck in
    /// `oauth_exchange_failed` or `upstream_unauthorized` (matching `muse-api-server-main`).
    pub async fn proactive_refresh_expiring(
        &self,
        margin_secs: i64,
    ) -> Result<usize, GatewayError> {
        let now = Instant::now();
        let now_epoch = unix_seconds().unwrap_or(0);
        let default_margin = margin_secs.max(0);
        let due_keys: Vec<(String, Option<i64>)> = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            guard
                .accounts
                .values()
                .filter_map(|acc| {
                    if !acc.enabled || acc.cooldown_remaining_secs(now) > 0 {
                        return None;
                    }
                    let drv = self.drivers.get(&acc.vendor);
                    if acc.status == GatewayAccountStatus::Active {
                        let eff_margin = drv
                            .map(|d| d.proactive_refresh_margin_secs(default_margin))
                            .unwrap_or(default_margin);
                        let eff_threshold = now_epoch + eff_margin;
                        if acc.expires_at.is_some_and(|exp| exp <= eff_threshold) {
                            return Some((acc.account_key.clone(), Some(eff_threshold)));
                        }
                    } else if acc.status == GatewayAccountStatus::ReauthRequired
                        && let (Some(d), Some(b)) = (drv, acc.bundle.as_ref())
                        && d.can_self_heal_reauth(b, acc.last_error.as_deref())
                    {
                        return Some((acc.account_key.clone(), None));
                    }
                    None
                })
                .collect()
        };

        let mut refreshed = 0usize;
        for (key, skip_after) in due_keys {
            if self
                .refresh_account_internal(&key, skip_after)
                .await
                .is_ok()
            {
                refreshed += 1;
            }
        }
        Ok(refreshed)
    }

    pub fn spawn_proactive_refresh_loop(
        self: &Arc<Self>,
        interval: Duration,
        margin_secs: i64,
    ) -> tokio::task::JoinHandle<()> {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval.max(Duration::from_secs(15)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let _ = hub.proactive_refresh_expiring(margin_secs).await;
            }
        })
    }

    pub(super) async fn refresh_account_internal(
        &self,
        account_key: &str,
        skip_if_expires_after: Option<i64>,
    ) -> Result<GatewayAccountView, GatewayError> {
        let lock = self.refresh_lock_for(account_key);
        let _single_flight = lock.lock().await;

        let (vendor, current_bundle, current_expires_at) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let bundle = acc
                .bundle
                .clone()
                .ok_or(GatewayError::AccountReauthRequired)?;
            (acc.vendor, bundle, acc.expires_at)
        };

        // Double-checked locking: if another task already refreshed this account, return immediately.
        if let (Some(threshold), Some(exp)) = (skip_if_expires_after, current_expires_at)
            && exp > threshold
        {
            return self.get_account_view(account_key).await;
        }

        let driver = self.driver_for(vendor)?;
        match driver.refresh_token(&current_bundle).await {
            Ok(exchanged) => {
                self.persist_refreshed_bundle(account_key, vendor, exchanged)
                    .await?;
                self.get_account_view(account_key).await
            }
            Err(err) => {
                self.record_refresh_failure(account_key, driver.as_ref(), &err)
                    .await;
                Err(err)
            }
        }
    }

    pub(super) async fn record_refresh_failure(
        &self,
        account_key: &str,
        driver: &dyn OAuthVendorDriver,
        err: &GatewayError,
    ) {
        if driver.is_transient_refresh_error(err) {
            self.mark_account_transient_refresh_cooldown(account_key, err.code());
        } else {
            self.mark_account_reauth_required(account_key, err.code())
                .await;
        }
    }

    pub(super) async fn persist_refreshed_bundle(
        &self,
        account_key: &str,
        vendor: GatewayVendor,
        exchanged: ExchangedOAuthAccount,
    ) -> Result<(), GatewayError> {
        let credential_json = exchanged
            .bundle
            .seal(self.secret_resolver.as_ref(), account_key)?;
        let now_epoch = unix_seconds().unwrap_or(0);
        let extra_headers = self
            .drivers
            .get(&vendor)
            .map(|drv| drv.extra_headers_for(&exchanged.bundle))
            .unwrap_or_default();

        if let Some(pool) = &self.pool {
            sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET credential_json = ?, \
                     account_email = COALESCE(?, account_email), \
                     expires_at = ?, \
                     status = CASE WHEN enabled = 0 THEN 'disabled' ELSE 'active' END, \
                     last_error = NULL, \
                     last_refreshed_at = ?, \
                     updated_at = ? \
                 WHERE account_key = ?",
            )
            .bind(&credential_json)
            .bind(&exchanged.account_email)
            .bind(exchanged.expires_at)
            .bind(now_epoch)
            .bind(now_epoch)
            .bind(account_key)
            .execute(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.credential_json = credential_json;
            acc.bundle = Some(exchanged.bundle);
            acc.extra_headers = extra_headers;
            if exchanged.account_email.is_some() {
                acc.account_email = exchanged.account_email;
            }
            acc.expires_at = exchanged.expires_at;
            acc.status = if acc.enabled {
                GatewayAccountStatus::Active
            } else {
                GatewayAccountStatus::Disabled
            };
            acc.last_error = None;
            acc.last_refreshed_at = Some(now_epoch);
            acc.cooldown_until = None;
        }
        Ok(())
    }

    pub(super) fn mark_account_transient_refresh_cooldown(
        &self,
        account_key: &str,
        err_code: &str,
    ) {
        let now = Instant::now();
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.cooldown_until = Some(now + Duration::from_secs(DEFAULT_COOLDOWN_SECS));
            acc.last_error = Some(err_code.to_owned());
        }
    }

    pub(super) async fn mark_account_reauth_required(&self, account_key: &str, err_code: &str) {
        let now_epoch = unix_seconds().unwrap_or(0);
        if let Some(pool) = &self.pool {
            let _ = sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET status = 'reauth_required', last_error = ?, updated_at = ? \
                 WHERE account_key = ?",
            )
            .bind(err_code)
            .bind(now_epoch)
            .bind(account_key)
            .execute(pool)
            .await;
        }
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.status = GatewayAccountStatus::ReauthRequired;
            acc.last_error = Some(err_code.to_owned());
        }
    }

    pub async fn refresh_account_now(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        if !valid_account_key(account_key) {
            return Err(GatewayError::AccountNotFound);
        }
        self.refresh_account_internal(account_key, None).await
    }
}
