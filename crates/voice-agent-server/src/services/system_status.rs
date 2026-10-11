//! Shared safe system summary for Admin and System MCP.
use crate::{
    config::EffectiveProviderBindings,
    database::Database,
    lifecycle::{Readiness, RuntimeLifecycle},
    providers::{DatabaseRuntimeSnapshot, RuntimeCatalog},
    services::provider_runtime::ProviderRuntimeManager,
};
use std::{sync::Arc, time::Instant};

pub(crate) struct SystemStatus {
    pub database: Option<Arc<Database>>,
    pub started_at: Instant,
    pub lifecycle: Arc<RuntimeLifecycle>,
    pub provider_runtime_manager: Option<Arc<ProviderRuntimeManager>>,
    pub database_runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
    pub runtimes: Arc<RuntimeCatalog>,
    pub required_providers: EffectiveProviderBindings,
    pub startup_complete: bool,
    pub managed_deployment: bool,
}

impl SystemStatus {
    pub async fn readiness(&self) -> Readiness {
        if !self.lifecycle.gate().is_open() {
            return Readiness::ShuttingDown;
        }
        if self.database.is_none() {
            // The database is configured but this process never opened one, so nothing about the
            // schema is authoritative.  Startup refused to bind in that case; reaching here means
            // a test or an embedder constructed the state directly, and it is still not ready.
            return Readiness::StartupIncomplete;
        }
        if !self.startup_complete {
            return Readiness::StartupIncomplete;
        }
        if self.required_runtime_missing() {
            return Readiness::RequiredRuntimeUnavailable;
        }
        if let Some(database) = &self.database
            && database.is_reachable().await.is_err()
        {
            return Readiness::DatabaseUnreachable;
        }
        Readiness::Ready
    }

    /// Whether a provider instance this deployment's own server defaults require is missing.
    ///
    /// This is a map lookup against the already-loaded RuntimeCatalog.  It prepares nothing, opens
    /// no model and resolves no secret — a required provider that could not be materialized never
    /// reached the listener in the first place, so this only catches state assembled some other
    /// way.
    fn required_runtime_missing(&self) -> bool {
        let providers = &self.required_providers;
        if let Some(manager) = &self.provider_runtime_manager
            && self.managed_deployment
        {
            return [
                ("vad", &providers.vad),
                ("asr", &providers.asr),
                ("llm", &providers.llm),
                ("tts", &providers.tts),
            ]
            .into_iter()
            .any(|(kind, key)| !manager.deployment_ready(kind, key));
        }
        self.runtimes.resolve(providers).is_err()
    }

    pub async fn summary(&self) -> Result<serde_json::Value, sqlx::Error> {
        let database = self.database.as_ref().ok_or(sqlx::Error::PoolClosed)?;
        let database_status = if database.is_reachable().await.is_ok() {
            "ready"
        } else {
            "unavailable"
        };
        if database_status == "unavailable" {
            return Ok(
                serde_json::json!({"status":self.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":self.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":"unavailable"},"providers":{"configured":null,"loaded":null,"stale":null,"failed":null},"sessions":{"active":self.lifecycle.drain().active()},"provider_runtime":self.provider_runtime_manager.as_ref().map(|manager|serde_json::json!({"accounting":manager.accounting(),"metrics":manager.metrics().snapshot()}))}),
            );
        }
        let tracked = self
            .provider_runtime_manager
            .as_ref()
            .map(|manager| manager.tracked_database_ids());
        let (configured, rows) = match database.provider_overview(tracked).await {
            Ok(value) => value,
            Err(cause) => return Err(cause),
        };
        let (mut loaded, mut stale, mut failed) = (0_u64, 0_u64, 0_u64);
        for (provider_id, key, revision) in &rows {
            if let Some(manager) = &self.provider_runtime_manager {
                match manager.inspect(*provider_id, *revision).desired_state {
                    crate::services::provider_runtime::RuntimeState::Ready => loaded += 1,
                    crate::services::provider_runtime::RuntimeState::Failed
                    | crate::services::provider_runtime::RuntimeState::Quarantined => failed += 1,
                    _ => stale += 1,
                }
                continue;
            }
            match self
                .database_runtime_snapshot
                .as_deref()
                .map(|snapshot| snapshot.runtime_state(key, *revision).status)
            {
                Some(crate::providers::DatabaseRuntimeStatus::Loaded) => loaded += 1,
                Some(crate::providers::DatabaseRuntimeStatus::Unavailable) => failed += 1,
                Some(crate::providers::DatabaseRuntimeStatus::NotLoaded) | None => stale += 1,
            }
        }
        if self.provider_runtime_manager.is_some() {
            stale = (configured as u64).saturating_sub(loaded + failed);
        }
        Ok(
            serde_json::json!({"status":self.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":self.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":database_status},"providers":{"configured":configured,"loaded":loaded,"stale":stale,"failed":failed},"sessions":{"active":self.lifecycle.drain().active()},"provider_runtime":self.provider_runtime_manager.as_ref().map(|manager|serde_json::json!({"accounting":manager.accounting(),"metrics":manager.metrics().snapshot()}))}),
        )
    }
}
