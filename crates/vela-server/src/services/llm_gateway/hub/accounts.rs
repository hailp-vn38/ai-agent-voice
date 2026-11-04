use super::*;

impl LlmGatewayHub {
    pub async fn list_accounts(
        &self,
        vendor_filter: Option<GatewayVendor>,
    ) -> Result<Vec<GatewayAccountView>, GatewayError> {
        let bound_map = self.load_bound_providers().await;
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
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

        Ok(list
            .into_iter()
            .map(|acc| {
                let cached_quota = quotas_guard.get(&acc.account_key).cloned();
                Self::to_account_view(acc, now, &bound_map, cached_quota)
            })
            .collect())
    }

    pub(super) async fn get_account_view(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        let bound_map = self.load_bound_providers().await;
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
        let acc = guard
            .accounts
            .get(account_key)
            .ok_or(GatewayError::AccountNotFound)?;
        let cached_quota = quotas_guard.get(account_key).cloned();
        Ok(Self::to_account_view(acc, now, &bound_map, cached_quota))
    }

    pub(super) fn to_account_view(
        acc: &LoadedGatewayAccount,
        now: Instant,
        bound_map: &HashMap<String, Vec<GatewayBoundProviderRef>>,
        quota: Option<GatewayAccountQuota>,
    ) -> GatewayAccountView {
        let mut bound = bound_map
            .get(&format!("account:{}", acc.account_key))
            .cloned()
            .unwrap_or_default();
        if let Some(pool_bound) = bound_map.get(&format!("pool:{}", acc.vendor.as_str())) {
            bound.extend(pool_bound.iter().cloned());
        }
        GatewayAccountView {
            account_key: acc.account_key.clone(),
            vendor: acc.vendor,
            label: acc.label.clone(),
            account_email: acc.account_email.clone(),
            credential_metadata: credentials::metadata(Some(&acc.credential_json)),
            enabled: acc.enabled,
            priority: acc.priority,
            status: acc.status,
            cooldown_remaining_secs: acc.cooldown_remaining_secs(now),
            expires_at: acc.expires_at,
            last_refreshed_at: acc.last_refreshed_at,
            last_used_at: acc.last_used_at,
            last_error: acc.last_error.clone(),
            bound_providers: bound,
            quota,
        }
    }

    pub(super) async fn load_bound_providers(
        &self,
    ) -> HashMap<String, Vec<GatewayBoundProviderRef>> {
        let mut map: HashMap<String, Vec<GatewayBoundProviderRef>> = HashMap::new();
        let Some(pool) = &self.pool else {
            return map;
        };
        let Ok(rows) = sqlx::query_as::<_, (String, String, String)>(
            "SELECT key, name, config_json FROM providers WHERE type = 'llm' ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        else {
            return map;
        };
        for (key, name, config_json) in rows {
            let Ok(val) = serde_json::from_str::<Value>(&config_json) else {
                continue;
            };
            let Some(binding_raw) = val.get("gateway_binding").and_then(Value::as_str) else {
                continue;
            };
            let Ok(binding) = GatewayBinding::parse(binding_raw) else {
                continue;
            };
            let (canonical, mode) = match &binding {
                GatewayBinding::Account(_) => (binding.as_config_string(), "account"),
                GatewayBinding::Pool(_) => (binding.as_config_string(), "pool"),
            };
            map.entry(canonical)
                .or_default()
                .push(GatewayBoundProviderRef {
                    provider_key: key,
                    display_name: name,
                    binding_mode: mode.to_owned(),
                });
        }
        map
    }

    pub async fn update_account(
        &self,
        account_key: &str,
        patch: UpdateGatewayAccountPatch,
    ) -> Result<GatewayAccountView, GatewayError> {
        let toggled_enabled = patch.enabled.is_some();
        let (next_label, next_enabled, next_priority, next_status) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let label = match patch.label {
                Some(ref l) => {
                    let trimmed = l.trim();
                    if trimmed.is_empty() || trimmed.len() > 256 {
                        return Err(GatewayError::ValidationFailed);
                    }
                    trimmed.to_owned()
                }
                None => acc.label.clone(),
            };
            let enabled = patch.enabled.unwrap_or(acc.enabled);
            let priority = patch.priority.unwrap_or(acc.priority);
            if !(0..=10_000).contains(&priority) {
                return Err(GatewayError::ValidationFailed);
            }
            let status = if !enabled {
                GatewayAccountStatus::Disabled
            } else if acc.status == GatewayAccountStatus::Disabled
                || (toggled_enabled && acc.status == GatewayAccountStatus::ReauthRequired)
            {
                if acc.bundle.is_some() {
                    GatewayAccountStatus::Active
                } else {
                    GatewayAccountStatus::ReauthRequired
                }
            } else {
                acc.status
            };
            (label, enabled, priority, status)
        };

        let now_epoch = unix_seconds().unwrap_or(0);
        if let Some(pool) = &self.pool {
            if toggled_enabled {
                sqlx::query(
                    "UPDATE llm_gateway_accounts \
                     SET label = ?, enabled = ?, priority = ?, status = ?, last_error = NULL, updated_at = ? \
                     WHERE account_key = ?",
                )
                .bind(&next_label)
                .bind(i64::from(next_enabled))
                .bind(next_priority)
                .bind(next_status.as_str())
                .bind(now_epoch)
                .bind(account_key)
                .execute(pool)
                .await
                .map_err(|_| GatewayError::DatabaseUnavailable)?;
            } else {
                sqlx::query(
                    "UPDATE llm_gateway_accounts \
                     SET label = ?, enabled = ?, priority = ?, status = ?, updated_at = ? \
                     WHERE account_key = ?",
                )
                .bind(&next_label)
                .bind(i64::from(next_enabled))
                .bind(next_priority)
                .bind(next_status.as_str())
                .bind(now_epoch)
                .bind(account_key)
                .execute(pool)
                .await
                .map_err(|_| GatewayError::DatabaseUnavailable)?;
            }
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_key) {
                acc.label = next_label;
                acc.enabled = next_enabled;
                acc.priority = next_priority;
                acc.status = next_status;
                if toggled_enabled {
                    acc.last_error = None;
                    acc.cooldown_until = None;
                }
            }
            guard.rebuild_vendor_index();
        }

        self.get_account_view(account_key).await
    }

    pub async fn delete_account(&self, account_key: &str) -> Result<(), GatewayError> {
        let bound_map = self.load_bound_providers().await;
        if let Some(bound_list) = bound_map.get(&format!("account:{account_key}"))
            && !bound_list.is_empty()
        {
            return Err(GatewayError::AccountInUse(
                bound_list.iter().map(|b| b.provider_key.clone()).collect(),
            ));
        }

        {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            if !guard.accounts.contains_key(account_key) {
                return Err(GatewayError::AccountNotFound);
            }
        }

        if let Some(pool) = &self.pool {
            sqlx::query("DELETE FROM llm_gateway_accounts WHERE account_key = ?")
                .bind(account_key)
                .execute(pool)
                .await
                .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            guard.accounts.remove(account_key);
            guard.rebuild_vendor_index();
        }
        self.quotas
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account_key);
        self.refresh_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account_key);
        Ok(())
    }
}
