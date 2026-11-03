use super::*;

impl LlmGatewayHub {
    pub(super) async fn persist_discovered_context_id(&self, account_key: &str, context_id: &str) {
        let (vendor, updated_bundle) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let Some(acc) = guard.accounts.get(account_key) else {
                return;
            };
            let Some(mut b) = acc.bundle.clone() else {
                return;
            };
            b.account_context_id = Some(context_id.to_owned());
            (acc.vendor, b)
        };
        let Ok(credential_json) = updated_bundle.seal(self.secret_resolver.as_ref(), account_key)
        else {
            return;
        };
        let extra_headers = self
            .drivers
            .get(&vendor)
            .map(|drv| drv.extra_headers_for(&updated_bundle))
            .unwrap_or_default();
        if let Some(pool) = &self.pool {
            let _ = sqlx::query(
                "UPDATE llm_gateway_accounts SET credential_json = ? WHERE account_key = ?",
            )
            .bind(&credential_json)
            .bind(account_key)
            .execute(pool)
            .await;
        }
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.credential_json = credential_json;
            acc.bundle = Some(updated_bundle);
            acc.extra_headers = extra_headers;
        }
    }

    pub(crate) async fn persist_muse_runtime_credential(
        &self,
        account_key: &str,
        cred: crate::providers::llm::gateway::muse::config::MuseAccountCredential,
        discovered_email: Option<String>,
    ) {
        let is_muse = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            guard
                .accounts
                .get(account_key)
                .is_some_and(|acc| acc.vendor == GatewayVendor::Muse)
        };
        if !is_muse {
            return;
        }
        let Ok(exchanged) = super::super::vendors::muse::MuseOAuthDriver::build_exchanged_account(
            cred,
            discovered_email,
        ) else {
            return;
        };
        let _ = self
            .persist_refreshed_bundle(account_key, GatewayVendor::Muse, exchanged)
            .await;
    }

    pub(crate) async fn try_refresh_on_unauthorized(
        &self,
        account_key: &str,
        rejected_token: &str,
    ) -> Option<GatewayLease> {
        let lock = self.refresh_lock_for(account_key);
        let _single_flight = lock.lock().await;

        let (vendor, current_bundle) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard.accounts.get(account_key)?;
            let bundle = acc.bundle.clone()?;
            if bundle.access_token != rejected_token {
                drop(guard);
                return self
                    .acquire_lease(&GatewayBinding::Account(account_key.to_owned()))
                    .ok();
            }
            if bundle.refresh_token.is_none() && bundle.upstream_oauth_token.is_none() {
                return None;
            }
            (acc.vendor, bundle)
        };

        let driver = self.driver_for(vendor).ok()?;
        match driver.refresh_token(&current_bundle).await {
            Ok(exchanged) => {
                self.persist_refreshed_bundle(account_key, vendor, exchanged)
                    .await
                    .ok()?;
                self.acquire_lease(&GatewayBinding::Account(account_key.to_owned()))
                    .ok()
            }
            Err(err) => {
                self.record_refresh_failure(account_key, driver.as_ref(), &err)
                    .await;
                None
            }
        }
    }

    pub async fn clear_cooldown(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get_mut(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            acc.cooldown_until = None;
            acc.last_error = None;
        }
        self.get_account_view(account_key).await
    }

    pub(super) async fn clear_stale_unauthorized_on_valid_token(&self, account_key: &str) {
        let should_update_db = {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            let Some(acc) = guard.accounts.get_mut(account_key) else {
                return;
            };
            let had_unauthorized = acc.last_error.as_deref() == Some("upstream_unauthorized")
                || acc.status == GatewayAccountStatus::ReauthRequired;
            if !had_unauthorized {
                return;
            }
            if acc.last_error.as_deref() == Some("upstream_unauthorized") {
                acc.last_error = None;
            }
            if acc.status == GatewayAccountStatus::ReauthRequired && acc.bundle.is_some() {
                acc.status = if acc.enabled {
                    GatewayAccountStatus::Active
                } else {
                    GatewayAccountStatus::Disabled
                };
            }
            true
        };

        if should_update_db && let Some(pool) = &self.pool {
            let _ = sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET status = CASE WHEN enabled = 0 THEN 'disabled' ELSE 'active' END, \
                     last_error = NULL \
                 WHERE account_key = ? AND (status = 'reauth_required' OR last_error = 'upstream_unauthorized')",
            )
            .bind(account_key)
            .execute(pool)
            .await;
        }
    }
}
