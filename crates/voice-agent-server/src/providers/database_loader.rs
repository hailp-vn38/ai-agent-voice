//! Materializes database desired-provider rows during startup.  This module deliberately has no
//! WebSocket dependency: session admission can only resolve the immutable catalog it produces.

use std::{collections::HashMap, sync::Arc, time::Duration};

use serde_json::Value;

use crate::{
    config::{
        AppConfig, AsrInstanceConfig, LlmInstanceConfig, TtsInstanceConfig, VadInstanceConfig,
    },
    database::{
        DesiredProvider, provider_config,
        secrets::{SecretRef, SecretResolver},
    },
    models::prepare,
    providers::{
        ProviderCatalog, RuntimeCatalog, compiled_provider_registry, loader::LoadedProviders,
    },
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseRuntimeStatus {
    NotLoaded,
    Unavailable,
    Loaded,
}

/// Coarse, redactable reason retained for Provider Load Plan policy and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseRuntimeFailure {
    Configuration,
    Secret,
    Runtime,
}

impl From<()> for DatabaseRuntimeFailure {
    fn from(_: ()) -> Self {
        Self::Runtime
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseRuntimeState {
    pub provider_id: i64,
    pub desired_revision: i64,
    pub status: DatabaseRuntimeStatus,
    pub failure: Option<DatabaseRuntimeFailure>,
}

/// Startup snapshot of DB-backed runtime outcomes.  A state is never changed after creation;
/// Admin mutations compare their desired revision against this snapshot instead of hot-reloading.
pub struct DatabaseMaterialization {
    pub loaded: LoadedProviders,
    states: HashMap<String, DatabaseRuntimeState>,
}

#[derive(Clone, Default)]
pub struct DatabaseRuntimeSnapshot {
    states: HashMap<String, DatabaseRuntimeState>,
}

impl DatabaseMaterialization {
    pub(crate) fn take_loaded(&mut self) -> LoadedProviders {
        std::mem::replace(&mut self.loaded, empty_loaded())
    }

    pub fn snapshot(&self) -> DatabaseRuntimeSnapshot {
        DatabaseRuntimeSnapshot {
            states: self.states.clone(),
        }
    }
    pub fn runtime_state(&self, key: &str, desired_revision: i64) -> DatabaseRuntimeState {
        self.states
            .get(key)
            .cloned()
            .unwrap_or(DatabaseRuntimeState {
                provider_id: 0,
                desired_revision,
                status: DatabaseRuntimeStatus::NotLoaded,
                failure: None,
            })
    }
}

impl DatabaseRuntimeSnapshot {
    pub fn from_states(states: impl IntoIterator<Item = (String, DatabaseRuntimeState)>) -> Self {
        Self {
            states: states.into_iter().collect(),
        }
    }

    pub fn runtime_state(&self, key: &str, desired_revision: i64) -> DatabaseRuntimeState {
        self.states
            .get(key)
            .cloned()
            .unwrap_or(DatabaseRuntimeState {
                provider_id: 0,
                desired_revision,
                status: DatabaseRuntimeStatus::NotLoaded,
                failure: None,
            })
    }
}

/// Validates and attempts every enabled desired row.  Per-row failure is intentionally reported
/// as `Unavailable`: Ticket 07's Provider Load Plan decides whether that outcome blocks startup
/// (required) or excludes an optional candidate.  This function never falls back or loads later.
pub fn materialize_database_providers(
    config: &AppConfig,
    rows: Vec<DesiredProvider>,
    secrets: &dyn SecretResolver,
) -> DatabaseMaterialization {
    let mut loaded = empty_loaded();
    let mut states = HashMap::with_capacity(rows.len());
    for row in rows {
        let state = DatabaseRuntimeState {
            provider_id: row.id,
            desired_revision: row.revision,
            status: DatabaseRuntimeStatus::Loaded,
            failure: None,
        };
        let state = match materialize_one(config, &row, secrets, &mut loaded) {
            Ok(()) => state,
            Err(failure) => DatabaseRuntimeState {
                status: DatabaseRuntimeStatus::Unavailable,
                failure: Some(failure),
                ..state
            },
        };
        states.insert(row.key.clone(), state);
    }
    DatabaseMaterialization { loaded, states }
}

impl DatabaseMaterialization {
    pub(crate) fn mark_unavailable(&mut self, key: &str, failure: DatabaseRuntimeFailure) {
        if let Some(state) = self.states.get_mut(key) {
            state.status = DatabaseRuntimeStatus::Unavailable;
            state.failure = Some(failure);
        }
    }
}

fn empty_loaded() -> LoadedProviders {
    LoadedProviders {
        providers: ProviderCatalog {
            vad: HashMap::new(),
            asr: HashMap::new(),
            llm: HashMap::new(),
            tts: HashMap::new(),
            vision: HashMap::new(),
        },
        runtimes: RuntimeCatalog {
            vad: HashMap::new(),
            asr: HashMap::new(),
            llm: HashMap::new(),
            tts: HashMap::new(),
            vision: HashMap::new(),
        },
    }
}

fn materialize_one(
    config: &AppConfig,
    row: &DesiredProvider,
    secrets: &dyn SecretResolver,
    loaded: &mut LoadedProviders,
) -> Result<(), DatabaseRuntimeFailure> {
    let mut value = desired_value(row)?;
    let secret = match &row.secret_ref {
        Some(reference) => Some(
            secrets
                .resolve(
                    &SecretRef::parse(reference.clone())
                        .map_err(|_| DatabaseRuntimeFailure::Secret)?,
                )
                .map_err(|_| DatabaseRuntimeFailure::Secret)?,
        ),
        None => None,
    };
    let registry = compiled_provider_registry();
    match row.kind.as_str() {
        "vad" => {
            reject_secret(row, secret.as_ref())?;
            let instance: VadInstanceConfig = typed_instance(&mut value, &row.adapter, None)?;
            let factory = registry.vad_factory(instance.adapter()).map_err(|_| ())?;
            let model = prepare(
                &config.deployment.model_manifest,
                &config.deployment.models.root,
                config.deployment.models.offline,
                factory.model_identity(&instance).map_err(|_| ())?,
                factory.adapter(),
                &config.deployment,
            )
            .map_err(|_| ())?;
            let provider = factory
                .build(&instance, &config.runtime, &model)
                .map_err(|_| ())?;
            loaded.runtimes.vad.insert(
                row.key.clone(),
                Arc::new(VadWorkerRuntime::new(
                    Arc::clone(&provider),
                    vad_worker_config(config),
                )),
            );
            loaded.providers.vad.insert(row.key.clone(), provider);
        }
        "asr" => {
            reject_secret(row, secret.as_ref())?;
            let instance: AsrInstanceConfig = typed_instance(&mut value, &row.adapter, None)?;
            let factory = registry.asr_factory(instance.adapter()).map_err(|_| ())?;
            let model = prepare(
                &config.deployment.model_manifest,
                &config.deployment.models.root,
                config.deployment.models.offline,
                factory.model_identity(&instance).map_err(|_| ())?,
                factory.adapter(),
                &config.deployment,
            )
            .map_err(|_| ())?;
            let samples = usize::try_from(config.audio.max_utterance_ms)
                .map_err(|_| ())?
                .checked_mul(16)
                .ok_or(())?;
            let provider = factory.build(&instance, &model, samples).map_err(|_| ())?;
            loaded.runtimes.asr.insert(
                row.key.clone(),
                Arc::new(AsrWorkerRuntime::new(
                    Arc::clone(&provider),
                    asr_worker_config(config),
                )),
            );
            loaded.providers.asr.insert(row.key.clone(), provider);
        }
        "llm" => {
            let instance: LlmInstanceConfig = typed_instance(
                &mut value,
                &row.adapter,
                secret.as_ref().map(|value| value.expose()),
            )?;
            let timeout = Duration::from_millis(instance.openai().timeout_ms);
            let provider = registry
                .llm_factory(instance.adapter())
                .map_err(|_| ())?
                .build(&instance)
                .map_err(|_| ())?;
            loaded.runtimes.llm.insert(
                row.key.clone(),
                Arc::new(LlmRuntime::new(
                    Arc::clone(&provider),
                    config.limits.llm_concurrency,
                    timeout,
                )),
            );
            loaded.providers.llm.insert(row.key.clone(), provider);
        }
        "tts" => {
            reject_secret(row, secret.as_ref())?;
            let instance: TtsInstanceConfig = typed_instance(&mut value, &row.adapter, None)?;
            let factory = registry.tts_factory(instance.adapter()).map_err(|_| ())?;
            let model = factory
                .model_identity(&instance)
                .map_err(|_| ())?
                .map(|identity| {
                    prepare(
                        &config.deployment.model_manifest,
                        &config.deployment.models.root,
                        config.deployment.models.offline,
                        identity,
                        factory.adapter(),
                        &config.deployment,
                    )
                })
                .transpose()
                .map_err(|_| ())?;
            let provider = factory
                .build(&instance, &config.runtime, model.as_ref())
                .map_err(|_| ())?;
            loaded.runtimes.tts.insert(
                row.key.clone(),
                Arc::new(TtsWorkerRuntime::new(
                    Arc::clone(&provider),
                    tts_worker_config(config),
                )),
            );
            loaded.providers.tts.insert(row.key.clone(), provider);
        }
        _ => return Err(DatabaseRuntimeFailure::Configuration),
    }
    Ok(())
}

fn desired_value(row: &DesiredProvider) -> Result<Value, DatabaseRuntimeFailure> {
    let raw = provider_config::validate_raw(&row.adapter, &row.config_json)
        .map_err(|_| DatabaseRuntimeFailure::Configuration)?;
    serde_json::from_str(&raw).map_err(|_| DatabaseRuntimeFailure::Configuration)
}

fn typed_instance<T: serde::de::DeserializeOwned>(
    value: &mut Value,
    adapter: &str,
    secret: Option<&str>,
) -> Result<T, ()> {
    let object = value.as_object_mut().ok_or(())?;
    // `max_tokens` is an Admin bound for future provider policy; the current runtime factory does
    // not consume it, so it must not turn a valid desired row into an adapter-specific bag.
    object.remove("max_tokens");
    object.insert("adapter".into(), Value::String(adapter.into()));
    if let Some(secret) = secret {
        object.insert("api_key".into(), Value::String(secret.into()));
    }
    serde_json::from_value(value.clone()).map_err(|_| ())
}

fn reject_secret(
    row: &DesiredProvider,
    secret: Option<&crate::database::secrets::SecretValue>,
) -> Result<(), ()> {
    if row.secret_ref.is_some() || secret.is_some() {
        Err(())
    } else {
        Ok(())
    }
}

fn vad_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.vad.max_workers,
        command_capacity: config.workers.vad.command_queue_capacity,
        final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
    }
}
fn asr_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.asr.max_workers,
        command_capacity: config.workers.asr.command_queue_capacity,
        final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
    }
}
fn tts_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.tts.max_workers,
        command_capacity: config.workers.tts.command_queue_capacity,
        final_timeout: Duration::from_millis(config.tts.timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::secrets::EnvSecretResolver;

    fn config() -> AppConfig {
        toml::from_str(
            r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"

            [provider_defaults]
            vad = "vad"
            asr = "asr"
            llm = "llm"
            tts = "tts"
            "#,
        )
        .unwrap()
    }

    fn row(config_json: &str, secret_ref: Option<&str>) -> DesiredProvider {
        DesiredProvider {
            id: 7,
            key: "db-llm".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            config_json: config_json.into(),
            secret_ref: secret_ref.map(str::to_owned),
            revision: 3,
        }
    }

    #[test]
    fn invalid_config_and_unavailable_secret_are_classified_without_runtime() {
        let invalid = materialize_database_providers(
            &config(),
            vec![row(r#"{"model":"x","unexpected":true}"#, None)],
            &EnvSecretResolver,
        );
        assert_eq!(
            invalid.runtime_state("db-llm", 3).status,
            DatabaseRuntimeStatus::Unavailable
        );
        assert_eq!(
            invalid.runtime_state("db-llm", 3).failure,
            Some(DatabaseRuntimeFailure::Configuration)
        );

        let unavailable_secret = materialize_database_providers(
            &config(),
            vec![row(
                r#"{"base_url":"https://example.test/v1","model":"x"}"#,
                Some("VOICE_AGENT_TEST_MISSING_SECRET"),
            )],
            &EnvSecretResolver,
        );
        assert_eq!(
            unavailable_secret.runtime_state("db-llm", 3).status,
            DatabaseRuntimeStatus::Unavailable
        );
        assert_eq!(
            unavailable_secret.runtime_state("db-llm", 3).failure,
            Some(DatabaseRuntimeFailure::Secret)
        );
        assert!(
            unavailable_secret
                .loaded
                .runtimes
                .llm
                .get("db-llm")
                .is_none()
        );
    }
}
