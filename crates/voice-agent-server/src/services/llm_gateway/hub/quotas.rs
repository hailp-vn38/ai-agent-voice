use super::*;

impl LlmGatewayHub {
    pub async fn get_account_quota(
        &self,
        account_key: &str,
        force: bool,
    ) -> Result<GatewayAccountQuota, GatewayError> {
        if !valid_account_key(account_key) {
            return Err(GatewayError::AccountNotFound);
        }
        let now_epoch = unix_seconds().unwrap_or(0);
        if !force {
            let guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = guard.get(account_key)
                && now_epoch.saturating_sub(cached.fetched_at) < QUOTA_CACHE_TTL_SECS
            {
                return Ok(cached.clone());
            }
        }

        let (vendor, expires_at, has_refresh) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let has_refresh = acc
                .bundle
                .as_ref()
                .is_some_and(|b| b.refresh_token.is_some() || b.upstream_oauth_token.is_some());
            (acc.vendor, acc.expires_at, has_refresh)
        };

        if has_refresh && expires_at.is_some_and(|exp| exp <= now_epoch + 60) {
            let _ = self
                .refresh_account_internal(account_key, Some(now_epoch + 60))
                .await;
        }

        let bundle = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            acc.bundle
                .clone()
                .ok_or(GatewayError::AccountReauthRequired)?
        };

        let driver = self.driver_for(vendor)?;
        let snapshot_res = match driver.fetch_quota(&bundle).await {
            Err(GatewayError::AccountReauthRequired) if has_refresh => {
                self.refresh_account_internal(account_key, None).await?;
                let refreshed_bundle = {
                    let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                    guard
                        .accounts
                        .get(account_key)
                        .and_then(|acc| acc.bundle.clone())
                        .ok_or(GatewayError::AccountReauthRequired)?
                };
                driver.fetch_quota(&refreshed_bundle).await
            }
            other => other,
        };

        let vendor_snapshot = match snapshot_res {
            Ok(snap) => snap,
            Err(err) => {
                if matches!(err, GatewayError::AccountReauthRequired) {
                    self.mark_account_reauth_required(account_key, err.code())
                        .await;
                }
                return Err(err);
            }
        };

        // A successful quota fetch proves the OAuth token is accepted by upstream (not 401),
        // so clear any stale `upstream_unauthorized` error or false `ReauthRequired` status.
        self.clear_stale_unauthorized_on_valid_token(account_key)
            .await;

        if let Some(ref discovered_ctx) = vendor_snapshot.discovered_account_context_id
            && bundle.account_context_id.is_none()
        {
            self.persist_discovered_context_id(account_key, discovered_ctx)
                .await;
        }

        let quota = GatewayAccountQuota {
            account_key: account_key.to_owned(),
            vendor,
            plan: vendor_snapshot.plan,
            limit_reached: vendor_snapshot.limit_reached,
            windows: vendor_snapshot.windows,
            message: vendor_snapshot.message,
            fetched_at: unix_seconds().unwrap_or(now_epoch),
        };

        self.quotas
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(account_key.to_owned(), quota.clone());
        Ok(quota)
    }

    pub async fn list_quotas(
        &self,
        vendor_filter: Option<GatewayVendor>,
        force: bool,
    ) -> Result<Vec<GatewayAccountQuota>, GatewayError> {
        let targets: Vec<(String, GatewayVendor)> = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let mut list: Vec<&LoadedGatewayAccount> = guard
                .accounts
                .values()
                .filter(|acc| vendor_filter.is_none_or(|v| acc.vendor == v))
                .collect();
            list.sort_by(|a, b| {
                a.vendor
                    .cmp(&b.vendor)
                    .then_with(|| a.priority.cmp(&b.priority))
                    .then_with(|| a.account_key.cmp(&b.account_key))
            });
            list.into_iter()
                .map(|acc| (acc.account_key.clone(), acc.vendor))
                .collect()
        };

        let now_epoch = unix_seconds().unwrap_or(0);
        let mut results = Vec::with_capacity(targets.len());
        for (account_key, vendor) in targets {
            match self.get_account_quota(&account_key, force).await {
                Ok(q) => results.push(q),
                Err(err) => {
                    let fallback = GatewayAccountQuota {
                        account_key: account_key.clone(),
                        vendor,
                        plan: None,
                        limit_reached: false,
                        windows: Vec::new(),
                        message: Some(err.to_string()),
                        fetched_at: now_epoch,
                    };
                    results.push(fallback);
                }
            }
        }
        Ok(results)
    }
}
