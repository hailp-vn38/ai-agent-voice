use super::*;

impl LlmGatewayHub {
    pub fn list_vendors(&self) -> Vec<GatewayVendorInfo> {
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        GatewayVendor::ALL
            .into_iter()
            .map(|vendor| {
                let vendor_accounts: Vec<&LoadedGatewayAccount> = guard
                    .accounts
                    .values()
                    .filter(|a| a.vendor == vendor)
                    .collect();
                let total_accounts = vendor_accounts.len();
                let cooling_down_accounts = vendor_accounts
                    .iter()
                    .filter(|a| a.cooldown_remaining_secs(now) > 0)
                    .count();
                let active_accounts = vendor_accounts
                    .iter()
                    .filter(|a| a.is_ready_for_lease(now))
                    .count();
                GatewayVendorInfo {
                    vendor,
                    display_name: vendor.display_name().to_owned(),
                    login_mode: vendor.login_mode(),
                    default_base_url: vendor.default_base_url().to_owned(),
                    pool_binding: GatewayBinding::Pool(vendor),
                    total_accounts,
                    active_accounts,
                    cooling_down_accounts,
                    model_count: 0,
                    models: Vec::new(),
                }
            })
            .collect()
    }

    pub async fn list_vendors_detailed(&self) -> Vec<GatewayVendorInfo> {
        let mut vendors = self.list_vendors();
        let all_models = self.load_pool_models(None).await.unwrap_or_default();
        for info in &mut vendors {
            let vendor_models: Vec<GatewayPoolModelView> = all_models
                .iter()
                .filter(|m| m.vendor == info.vendor)
                .cloned()
                .collect();
            info.model_count = vendor_models.len();
            info.models = vendor_models;
        }
        vendors
    }

    pub async fn get_pool_detail(
        &self,
        vendor: GatewayVendor,
    ) -> Result<GatewayPoolDetailView, GatewayError> {
        let accounts = self.list_accounts(Some(vendor)).await?;
        let models = self.load_pool_models(Some(vendor)).await?;
        let total_accounts = accounts.len();
        let active_accounts = accounts
            .iter()
            .filter(|a| {
                a.enabled
                    && a.status == GatewayAccountStatus::Active
                    && a.cooldown_remaining_secs == 0
            })
            .count();
        let cooling_down_accounts = accounts
            .iter()
            .filter(|a| a.cooldown_remaining_secs > 0)
            .count();
        let reauth_required_accounts = accounts
            .iter()
            .filter(|a| a.status == GatewayAccountStatus::ReauthRequired)
            .count();
        let disabled_accounts = accounts
            .iter()
            .filter(|a| !a.enabled || a.status == GatewayAccountStatus::Disabled)
            .count();

        Ok(GatewayPoolDetailView {
            vendor,
            display_name: vendor.display_name().to_owned(),
            login_mode: vendor.login_mode(),
            default_base_url: vendor.default_base_url().to_owned(),
            pool_binding: GatewayBinding::Pool(vendor),
            total_accounts,
            active_accounts,
            cooling_down_accounts,
            reauth_required_accounts,
            disabled_accounts,
            accounts,
            models,
            preset_models: vendor.preset_models(),
        })
    }

    pub async fn create_pool_model(
        &self,
        vendor: GatewayVendor,
        req: CreatePoolModelRequest,
        request_id: &str,
    ) -> Result<GatewayPoolModelView, GatewayError> {
        let Some(pool) = &self.pool else {
            return Err(GatewayError::DatabaseUnavailable);
        };
        let model_trimmed = req.model.trim();
        if model_trimmed.is_empty()
            || model_trimmed.len() > 256
            || model_trimmed.chars().any(char::is_control)
        {
            return Err(GatewayError::ValidationFailed);
        }

        let binding = match req.gateway_binding {
            Some(parsed) => {
                self.validate_binding_belongs_to_vendor(&parsed, vendor)?;
                parsed
            }
            None => GatewayBinding::Pool(vendor),
        };

        let preset = vendor
            .preset_models()
            .iter()
            .find(|p| p.model == model_trimmed);

        let display_name = match req
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(trimmed) => {
                if trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
                    return Err(GatewayError::ValidationFailed);
                }
                trimmed.to_owned()
            }
            None => match preset {
                Some(p) => p.display_name.to_owned(),
                None => {
                    let fallback = format!("{model_trimmed} ({})", vendor.display_name());
                    fallback.chars().take(128).collect()
                }
            },
        };

        let tags = match req.tags {
            Some(t) => {
                if !crate::providers::llm::valid_llm_tags(Some(&t)) {
                    return Err(GatewayError::ValidationFailed);
                }
                Some(t)
            }
            None => match preset {
                Some(p) => Some(p.tags.iter().map(|t| (*t).to_owned()).collect()),
                None => Some(vec!["chat".to_owned()]),
            },
        };

        let base_url = match req
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw_url) => {
                Some(url::Url::parse(raw_url).map_err(|_| GatewayError::ValidationFailed)?)
            }
            None => None,
        };
        let reasoning_effort = req
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                preset
                    .and_then(|p| p.default_reasoning_effort)
                    .map(str::to_owned)
            });
        let timeout_ms = req
            .timeout_ms
            .or_else(|| preset.and_then(|p| p.default_timeout_ms))
            .unwrap_or_else(crate::config::defaults::default_llm_timeout_ms);

        let gw_config = crate::providers::llm::LlmGatewayConfig {
            gateway_binding: binding,
            model: model_trimmed.to_owned(),
            base_url,
            timeout_ms,
            timeout_profile: None,
            first_byte_ms: None,
            idle_ms: None,
            max_total_ms: None,
            max_tokens: req
                .max_tokens
                .or_else(|| preset.and_then(|p| p.default_max_tokens)),
            context_limit_tokens: req
                .context_limit_tokens
                .or_else(|| preset.and_then(|p| p.default_context_limit_tokens)),
            reasoning_effort,
            temperature_percent: req
                .temperature_percent
                .or_else(|| preset.and_then(|p| p.default_temperature_percent)),
            tags,
            account_concurrency: None,
            cdp_port: None,
            headless: None,
        };
        let config_val =
            serde_json::to_value(&gw_config).map_err(|_| GatewayError::ValidationFailed)?;
        let validated_config =
            crate::database::provider_config::validate("llm_gateway", &config_val)
                .map_err(|_| GatewayError::ValidationFailed)?;

        let provider_key = format!("llm_{}", uuid::Uuid::new_v4().simple());
        let enabled = i64::from(req.enabled.unwrap_or(true));
        let now_epoch = unix_seconds().unwrap_or(0);

        let mut tx = pool
            .begin()
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        let insert_res = sqlx::query(
            "INSERT INTO providers (key, name, type, adapter, config_json, credential_json, enabled, revision, created_at, updated_at) \
             VALUES (?, ?, 'llm', 'llm_gateway', ?, NULL, ?, 1, ?, ?)",
        )
        .bind(&provider_key)
        .bind(&display_name)
        .bind(&validated_config)
        .bind(enabled)
        .bind(now_epoch)
        .bind(now_epoch)
        .execute(&mut *tx)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;
        let provider_id = insert_res.last_insert_rowid();

        crate::database::audit::audit(
            &mut *tx,
            request_id,
            "provider",
            Some(provider_id),
            "create",
            None,
            Some(1),
            crate::database::audit::AuditOutcome::Success,
            1,
        )
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;

        self.get_pool_model(vendor, &provider_key).await
    }

    pub async fn update_pool_model(
        &self,
        vendor: GatewayVendor,
        provider_key: &str,
        patch: UpdatePoolModelPatch,
        request_id: &str,
    ) -> Result<GatewayPoolModelView, GatewayError> {
        let Some(pool) = &self.pool else {
            return Err(GatewayError::DatabaseUnavailable);
        };
        type ExistingRow = (i64, String, String, String, i64, i64);
        let (provider_id, mut name, adapter, config_json, old_enabled, old_revision): ExistingRow =
            sqlx::query_as(
                "SELECT id, name, adapter, config_json, enabled, revision FROM providers WHERE key = ? AND type = 'llm'",
            )
            .bind(provider_key)
            .fetch_optional(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?
            .ok_or(GatewayError::PoolModelNotFound)?;

        let mut config_val: Value =
            serde_json::from_str(&config_json).map_err(|_| GatewayError::ValidationFailed)?;
        self.ensure_config_belongs_to_vendor(&config_val, vendor)?;

        if let Some(raw_name) = patch.display_name {
            let trimmed = raw_name.trim();
            if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
                return Err(GatewayError::ValidationFailed);
            }
            name = trimmed.to_owned();
        }

        let next_enabled = i64::from(patch.enabled.unwrap_or(old_enabled != 0));
        let config_obj = config_val
            .as_object_mut()
            .ok_or(GatewayError::ValidationFailed)?;

        if let Some(raw_model) = patch.model {
            let trimmed = raw_model.trim();
            if trimmed.is_empty() || trimmed.len() > 256 || trimmed.chars().any(char::is_control) {
                return Err(GatewayError::ValidationFailed);
            }
            config_obj.insert("model".to_owned(), Value::String(trimmed.to_owned()));
        }

        if let Some(parsed) = patch.gateway_binding {
            self.validate_binding_belongs_to_vendor(&parsed, vendor)?;
            config_obj.insert(
                "gateway_binding".to_owned(),
                Value::String(parsed.as_config_string()),
            );
        }

        if let Some(base_url_opt) = patch.base_url {
            match base_url_opt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(url) => {
                    config_obj.insert("base_url".to_owned(), Value::String(url.to_owned()));
                }
                None => {
                    config_obj.remove("base_url");
                }
            }
        }

        if let Some(tags) = patch.tags {
            if !crate::providers::llm::valid_llm_tags(Some(&tags)) {
                return Err(GatewayError::ValidationFailed);
            }
            config_obj.insert(
                "tags".to_owned(),
                serde_json::to_value(tags).map_err(|_| GatewayError::ValidationFailed)?,
            );
        }

        if let Some(effort_opt) = patch.reasoning_effort {
            match effort_opt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(effort) => {
                    config_obj.insert(
                        "reasoning_effort".to_owned(),
                        Value::String(effort.to_owned()),
                    );
                }
                None => {
                    config_obj.remove("reasoning_effort");
                }
            }
        }

        if let Some(max_tok_opt) = patch.max_tokens {
            match max_tok_opt {
                Some(v) => {
                    config_obj.insert("max_tokens".to_owned(), Value::from(v));
                }
                None => {
                    config_obj.remove("max_tokens");
                }
            }
        }

        if let Some(ctx_opt) = patch.context_limit_tokens {
            match ctx_opt {
                Some(v) => {
                    config_obj.insert("context_limit_tokens".to_owned(), Value::from(v));
                }
                None => {
                    config_obj.remove("context_limit_tokens");
                }
            }
        }

        if let Some(temp_opt) = patch.temperature_percent {
            match temp_opt {
                Some(v) => {
                    config_obj.insert("temperature_percent".to_owned(), Value::from(v));
                }
                None => {
                    config_obj.remove("temperature_percent");
                }
            }
        }

        if let Some(timeout) = patch.timeout_ms {
            config_obj.insert("timeout_ms".to_owned(), Value::from(timeout));
        }

        let validated_config = crate::database::provider_config::validate(&adapter, &config_val)
            .map_err(|_| GatewayError::ValidationFailed)?;
        let now_epoch = unix_seconds().unwrap_or(0);

        let mut tx = pool
            .begin()
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        sqlx::query(
            "UPDATE providers \
             SET name = ?, config_json = ?, enabled = ?, revision = revision + 1, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&validated_config)
        .bind(next_enabled)
        .bind(now_epoch)
        .bind(provider_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;

        crate::database::audit::audit(
            &mut *tx,
            request_id,
            "provider",
            Some(provider_id),
            "update",
            Some(old_revision),
            Some(old_revision + 1),
            crate::database::audit::AuditOutcome::Success,
            1,
        )
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;

        self.get_pool_model(vendor, provider_key).await
    }

    pub async fn delete_pool_model(
        &self,
        vendor: GatewayVendor,
        provider_key: &str,
        request_id: &str,
    ) -> Result<bool, GatewayError> {
        let Some(pool) = &self.pool else {
            return Err(GatewayError::DatabaseUnavailable);
        };
        let (provider_id, config_json, old_revision): (i64, String, i64) = sqlx::query_as(
            "SELECT id, config_json, revision FROM providers WHERE key = ? AND type = 'llm'",
        )
        .bind(provider_key)
        .fetch_optional(pool)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?
        .ok_or(GatewayError::PoolModelNotFound)?;

        let config_val: Value =
            serde_json::from_str(&config_json).map_err(|_| GatewayError::PoolModelNotFound)?;
        self.ensure_config_belongs_to_vendor(&config_val, vendor)?;

        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        let bound_templates: Vec<String> = sqlx::query_scalar(
            "SELECT t.key \
             FROM template_provider_bindings b \
             JOIN agent_templates t ON t.id = b.template_id \
             WHERE b.provider_id = ? \
             ORDER BY t.key ASC",
        )
        .bind(provider_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;

        if !bound_templates.is_empty() {
            let _ = tx.rollback().await;
            return Err(GatewayError::PoolModelInUse(bound_templates));
        }

        let deleted = sqlx::query("DELETE FROM providers WHERE id = ?")
            .bind(provider_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?
            .rows_affected()
            == 1;

        if deleted {
            crate::database::audit::audit(
                &mut *tx,
                request_id,
                "provider",
                Some(provider_id),
                "delete",
                Some(old_revision),
                None,
                crate::database::audit::AuditOutcome::Success,
                1,
            )
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }
        tx.commit()
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;

        Ok(deleted)
    }

    pub(super) async fn get_pool_model(
        &self,
        vendor: GatewayVendor,
        provider_key: &str,
    ) -> Result<GatewayPoolModelView, GatewayError> {
        let models = self.load_pool_models(Some(vendor)).await?;
        models
            .into_iter()
            .find(|m| m.provider_key == provider_key)
            .ok_or(GatewayError::PoolModelNotFound)
    }

    pub(super) async fn load_pool_models(
        &self,
        vendor_filter: Option<GatewayVendor>,
    ) -> Result<Vec<GatewayPoolModelView>, GatewayError> {
        let Some(pool) = &self.pool else {
            return Ok(Vec::new());
        };
        type PoolModelRow = (i64, String, String, String, String, i64, i64, i64, i64, i64);
        let rows: Vec<PoolModelRow> = sqlx::query_as(
            "SELECT \
                p.id, \
                p.key, \
                p.name, \
                p.adapter, \
                p.config_json, \
                p.enabled, \
                p.revision, \
                p.created_at, \
                p.updated_at, \
                (SELECT COUNT(*) FROM template_provider_bindings b WHERE b.provider_id = p.id) AS template_usage_count \
             FROM providers p \
             WHERE p.type = 'llm' \
             ORDER BY p.id ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;

        let mut models = Vec::new();
        for (
            id,
            provider_key,
            display_name,
            adapter,
            config_json,
            enabled,
            revision,
            created_at,
            updated_at,
            template_usage_count,
        ) in rows
        {
            let Ok(val) = serde_json::from_str::<Value>(&config_json) else {
                continue;
            };
            let Some(binding_raw) = val.get("gateway_binding").and_then(Value::as_str) else {
                continue;
            };
            let Ok(binding) = GatewayBinding::parse(binding_raw) else {
                continue;
            };
            let Some((vendor, binding_mode)) = self.resolve_binding_vendor(&binding) else {
                continue;
            };
            if vendor_filter.is_some_and(|target| target != vendor) {
                continue;
            }
            let model = val
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let base_url = val
                .get("base_url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let tags = crate::providers::llm::llm_config_tags(&config_json);
            let reasoning_effort = val
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let max_tokens = val
                .get("max_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let context_limit_tokens = val
                .get("context_limit_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let temperature_percent = val
                .get("temperature_percent")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok());
            let timeout_ms = val
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or_else(crate::config::defaults::default_llm_timeout_ms);

            models.push(GatewayPoolModelView {
                id,
                provider_key,
                display_name,
                vendor,
                adapter,
                gateway_binding: binding,
                binding_mode: binding_mode.to_owned(),
                model,
                base_url,
                enabled: enabled != 0,
                revision,
                tags,
                reasoning_effort,
                max_tokens,
                context_limit_tokens,
                temperature_percent,
                timeout_ms,
                template_usage_count,
                created_at,
                updated_at,
            });
        }

        Ok(models)
    }

    pub(super) fn resolve_binding_vendor(
        &self,
        binding: &GatewayBinding,
    ) -> Option<(GatewayVendor, &'static str)> {
        match binding {
            GatewayBinding::Pool(vendor) => Some((*vendor, "pool")),
            GatewayBinding::Account(account_key) => {
                let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                guard
                    .accounts
                    .get(account_key)
                    .map(|acc| (acc.vendor, "account"))
            }
        }
    }

    pub(super) fn ensure_config_belongs_to_vendor(
        &self,
        config_val: &Value,
        expected_vendor: GatewayVendor,
    ) -> Result<(), GatewayError> {
        let current_binding_raw = config_val
            .get("gateway_binding")
            .and_then(Value::as_str)
            .ok_or(GatewayError::PoolModelNotFound)?;
        let current_binding = GatewayBinding::parse(current_binding_raw)
            .map_err(|_| GatewayError::PoolModelNotFound)?;
        if self
            .resolve_binding_vendor(&current_binding)
            .map(|(v, _)| v)
            != Some(expected_vendor)
        {
            return Err(GatewayError::PoolModelNotFound);
        }
        Ok(())
    }

    pub(super) fn validate_binding_belongs_to_vendor(
        &self,
        binding: &GatewayBinding,
        expected_vendor: GatewayVendor,
    ) -> Result<(), GatewayError> {
        match binding {
            GatewayBinding::Pool(vendor) => {
                if *vendor == expected_vendor {
                    Ok(())
                } else {
                    Err(GatewayError::InvalidBinding)
                }
            }
            GatewayBinding::Account(account_key) => {
                let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                let acc = guard
                    .accounts
                    .get(account_key)
                    .ok_or(GatewayError::AccountNotFound)?;
                if acc.vendor == expected_vendor {
                    Ok(())
                } else {
                    Err(GatewayError::InvalidBinding)
                }
            }
        }
    }
}
