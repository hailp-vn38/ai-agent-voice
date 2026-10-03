//! Materializes database desired-provider rows during startup.  This module deliberately has no
//! WebSocket dependency: session admission can only resolve the immutable catalog it produces.

use std::{collections::HashMap, sync::Arc, time::Duration};

use serde_json::Value;

use crate::{
    config::{
        AppConfig, AsrInstanceConfig, LlmInstanceConfig, TtsInstanceConfig, VadInstanceConfig,
    },
    database::{
        DesiredProvider, ProviderLoadPlan, ProviderLoadRequirement, provider_config,
        secrets::{SecretRef, SecretResolver},
    },
    models::prepare_immutable as prepare,
    providers::{
        LoadedVad, ProviderCatalog, RuntimeCatalog, compiled_provider_registry,
        loader::LoadedProviders, loader::vad_timing,
    },
    workers::{
        AsrWorkerRuntime, LlmRuntime, ProviderRuntimeAdmission, TtsWorkerRuntime, VadWorkerRuntime,
        WorkerRuntimeConfig,
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
    Quarantined,
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

/// A required database provider could not become Loaded Runtime, so startup must fail before the
/// listener binds.  Only the provider key and the coarse failure class survive for telemetry.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("required provider `{provider_key}` is unavailable")]
pub struct RequiredProviderUnavailable {
    pub provider_key: String,
    pub failure: DatabaseRuntimeFailure,
}

/// Applies the startup Provider Load Plan to the enabled desired rows.
///
/// Required rows must materialize or startup fails.  Optional rows are still attempted so a
/// non-default Template can become a switch candidate, and their failure only excludes that
/// candidate.  Unbound rows are validated but never resolve a secret or build a runtime.
pub fn materialize_database_providers(
    config: &AppConfig,
    rows: Vec<DesiredProvider>,
    plan: &ProviderLoadPlan,
    secrets: &dyn SecretResolver,
) -> Result<DatabaseMaterialization, RequiredProviderUnavailable> {
    let mut loaded = empty_loaded();
    let mut states = HashMap::with_capacity(rows.len());
    for row in rows {
        let requirement = plan.requirement(&row.key);
        let state = load_one(config, &row, requirement, secrets, &mut loaded);
        if requirement == ProviderLoadRequirement::Required
            && let Some(failure) = state.failure
        {
            return Err(RequiredProviderUnavailable {
                provider_key: row.key.clone(),
                failure,
            });
        }
        states.insert(row.key.clone(), state);
    }
    Ok(DatabaseMaterialization { loaded, states })
}

fn load_one(
    config: &AppConfig,
    row: &DesiredProvider,
    requirement: ProviderLoadRequirement,
    secrets: &dyn SecretResolver,
    loaded: &mut LoadedProviders,
) -> DatabaseRuntimeState {
    // An unbound provider is still configuration-validated, so Admin inspection is honest, but it
    // must not cost a secret resolution or a model build for a session that cannot use it.
    let (status, outcome) = match requirement {
        ProviderLoadRequirement::Unbound => (
            DatabaseRuntimeStatus::NotLoaded,
            desired_value(row).map(|_| ()),
        ),
        ProviderLoadRequirement::Required | ProviderLoadRequirement::Optional => (
            DatabaseRuntimeStatus::Loaded,
            materialize_one(config, row, secrets, loaded, None, None),
        ),
    };
    match outcome {
        Ok(()) => DatabaseRuntimeState {
            provider_id: row.id,
            desired_revision: row.revision,
            status,
            failure: None,
        },
        Err(failure) => DatabaseRuntimeState {
            provider_id: row.id,
            desired_revision: row.revision,
            status: DatabaseRuntimeStatus::Unavailable,
            failure: Some(failure),
        },
    }
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

/// Builds exactly one owned desired version using the same factories as startup.
/// The caller owns bounded blocking execution, memory reservation and publication.
/// Admission uses installed artifacts only: missing artifacts must be prepared explicitly.
pub fn materialize_provider(
    config: &AppConfig,
    row: &DesiredProvider,
    secrets: &dyn SecretResolver,
) -> Result<RuntimeCatalog, DatabaseRuntimeFailure> {
    let capacity = match row.kind.as_str() {
        "vad" => config.workers.vad.max_workers,
        "asr" => config.workers.asr.max_workers,
        "llm" => config.limits.llm_concurrency,
        "tts" => config.workers.tts.max_workers,
        _ => return Err(DatabaseRuntimeFailure::Configuration),
    };
    if capacity == 0 {
        return Err(DatabaseRuntimeFailure::Configuration);
    }
    materialize_provider_with_admission(
        config,
        row,
        secrets,
        ProviderRuntimeAdmission::new(capacity, 1),
    )
}

pub fn materialize_provider_with_admission(
    config: &AppConfig,
    row: &DesiredProvider,
    secrets: &dyn SecretResolver,
    quota: ProviderRuntimeAdmission,
) -> Result<RuntimeCatalog, DatabaseRuntimeFailure> {
    for worker in [
        vad_worker_config(config),
        asr_worker_config(config),
        tts_worker_config(config),
    ] {
        worker
            .validate()
            .map_err(|_| DatabaseRuntimeFailure::Configuration)?;
    }
    let mut config = config.clone();
    config.deployment.models.offline = true;
    let mut loaded = empty_loaded();
    materialize_one(&config, row, secrets, &mut loaded, Some(quota), None)?;
    Ok(loaded.runtimes)
}

pub(crate) fn materialize_provider_from_artifacts(
    config: &AppConfig,
    row: &DesiredProvider,
    secrets: &dyn SecretResolver,
    quota: ProviderRuntimeAdmission,
    model: Option<&crate::models::ResolvedModel>,
) -> Result<RuntimeCatalog, DatabaseRuntimeFailure> {
    let mut config = config.clone();
    config.deployment.models.offline = true;
    let mut loaded = empty_loaded();
    materialize_one(&config, row, secrets, &mut loaded, Some(quota), model)?;
    Ok(loaded.runtimes)
}

fn selected_model(
    config: &AppConfig,
    identity: &str,
    adapter: &str,
    prepared: Option<&crate::models::ResolvedModel>,
) -> Result<crate::models::ResolvedModel, crate::models::ModelError> {
    if let Some(model) = prepared {
        if model.identity() != identity || model.adapter() != adapter {
            return Err(crate::models::ModelError::UnknownModel(identity.into()));
        }
        return Ok(model.clone());
    }
    prepare(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        config.deployment.models.offline,
        identity,
        adapter,
        &config.deployment,
    )
}

fn materialize_one(
    config: &AppConfig,
    row: &DesiredProvider,
    secrets: &dyn SecretResolver,
    loaded: &mut LoadedProviders,
    quota: Option<ProviderRuntimeAdmission>,
    prepared_model: Option<&crate::models::ResolvedModel>,
) -> Result<(), DatabaseRuntimeFailure> {
    let mut value = super::factory_registry::effective_local_config(
        &row.adapter,
        desired_value(row)?,
        &config.runtime,
    )
    .map_err(|_| DatabaseRuntimeFailure::Configuration)?;
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
            let instance: VadInstanceConfig = if row.id == 0 {
                config
                    .providers
                    .vad
                    .instances
                    .get(&row.key)
                    .cloned()
                    .ok_or(())?
            } else {
                typed_instance(&mut value, &row.adapter, None)?
            };
            let factory = registry.vad_factory(instance.adapter()).map_err(|_| ())?;
            let model = selected_model(
                config,
                factory.model_identity(&instance).map_err(|_| ())?,
                factory.adapter(),
                prepared_model,
            )
            .map_err(|_| ())?;
            let provider = factory
                .build(&instance, &config.runtime, &model)
                .map_err(|_| ())?;
            let (segmenter, pre_roll_samples) = vad_timing(instance.silero_onnx());
            loaded.runtimes.vad.insert(
                row.key.clone(),
                LoadedVad {
                    runtime: Arc::new(
                        VadWorkerRuntime::try_new_with_admission(
                            Arc::clone(&provider),
                            vad_worker_config(config),
                            quota.unwrap_or_else(|| {
                                ProviderRuntimeAdmission::new(config.workers.vad.max_workers, 1)
                            }),
                        )
                        .map_err(|_| DatabaseRuntimeFailure::Runtime)?,
                    ),
                    segmenter,
                    pre_roll_samples,
                },
            );
            loaded.providers.vad.insert(row.key.clone(), provider);
        }
        "asr" => {
            reject_secret(row, secret.as_ref())?;
            let instance: AsrInstanceConfig = typed_instance(&mut value, &row.adapter, None)?;
            let factory = registry.asr_factory(instance.adapter()).map_err(|_| ())?;
            let model = selected_model(
                config,
                factory.model_identity(&instance).map_err(|_| ())?,
                factory.adapter(),
                prepared_model,
            )
            .map_err(|_| ())?;
            let samples = usize::try_from(config.audio.max_utterance_ms)
                .map_err(|_| ())?
                .checked_mul(16)
                .ok_or(())?;
            let provider = factory
                .build(&instance, &config.runtime, &model, samples)
                .map_err(|_| ())?;
            loaded.runtimes.asr.insert(
                row.key.clone(),
                Arc::new(
                    AsrWorkerRuntime::try_new_with_admission(
                        Arc::clone(&provider),
                        asr_worker_config(config),
                        quota.unwrap_or_else(|| {
                            ProviderRuntimeAdmission::new(config.workers.asr.max_workers, 1)
                        }),
                    )
                    .map_err(|_| ())?,
                ),
            );
            loaded.providers.asr.insert(row.key.clone(), provider);
        }
        "llm" => {
            let instance: LlmInstanceConfig = if row.id == 0 {
                config
                    .providers
                    .llm
                    .instances
                    .get(&row.key)
                    .cloned()
                    .ok_or(())?
            } else {
                typed_instance(
                    &mut value,
                    &row.adapter,
                    secret.as_ref().map(|value| value.expose()),
                )?
            };
            let timeout = Duration::from_millis(instance.openai().timeout_ms);
            let provider = registry
                .llm_factory(instance.adapter())
                .map_err(|_| ())?
                .build(&instance)
                .map_err(|_| ())?;
            loaded.runtimes.llm.insert(
                row.key.clone(),
                Arc::new(LlmRuntime::new_with_admission(
                    Arc::clone(&provider),
                    quota.unwrap_or_else(|| {
                        ProviderRuntimeAdmission::new(config.limits.llm_concurrency, 1)
                    }),
                    timeout,
                )),
            );
            loaded.providers.llm.insert(row.key.clone(), provider);
        }
        "tts" => {
            let instance = if row.id == 0 {
                config
                    .providers
                    .tts
                    .instances
                    .get(&row.key)
                    .cloned()
                    .ok_or(())?
            } else {
                typed_tts_instance(&mut value, &row.adapter, secret.as_ref())?
            };
            let factory = registry.tts_factory(instance.adapter()).map_err(|_| ())?;
            let model = factory
                .model_identity(&instance)
                .map_err(|_| ())?
                .map(|identity| selected_model(config, identity, factory.adapter(), prepared_model))
                .transpose()
                .map_err(|_| ())?;
            let provider = factory
                .build(&instance, &config.runtime, model.as_ref())
                .map_err(|_| ())?;
            loaded.runtimes.tts.insert(
                row.key.clone(),
                Arc::new(
                    TtsWorkerRuntime::try_new_with_admission(
                        Arc::clone(&provider),
                        tts_worker_config(config),
                        quota.unwrap_or_else(|| {
                            ProviderRuntimeAdmission::new(config.workers.tts.max_workers, 1)
                        }),
                    )
                    .map_err(|error| {
                        if matches!(error, crate::workers::TtsWorkerError::Quarantined) {
                            DatabaseRuntimeFailure::Quarantined
                        } else {
                            DatabaseRuntimeFailure::Runtime
                        }
                    })?,
                ),
            );
            loaded.providers.tts.insert(row.key.clone(), provider);
        }
        _ => return Err(DatabaseRuntimeFailure::Configuration),
    }
    Ok(())
}

fn desired_value(row: &DesiredProvider) -> Result<Value, DatabaseRuntimeFailure> {
    if !crate::providers::compiled_provider_adapter_registry()
        .get(&row.adapter)
        .is_some_and(|descriptor| descriptor.provider_type.as_str() == row.kind)
    {
        return Err(DatabaseRuntimeFailure::Configuration);
    }
    if row.id == 0 {
        if row.config_json.len() > provider_config::MAX_PROVIDER_CONFIG_BYTES {
            return Err(DatabaseRuntimeFailure::Configuration);
        }
        return serde_json::from_str(&row.config_json)
            .map_err(|_| DatabaseRuntimeFailure::Configuration);
    }
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

fn typed_tts_instance(
    value: &mut Value,
    adapter: &str,
    secret: Option<&crate::database::secrets::SecretValue>,
) -> Result<TtsInstanceConfig, ()> {
    if adapter != "chillaudio_ws" {
        return reject_secret_value(secret).and_then(|()| typed_instance(value, adapter, None));
    }
    let object = value.as_object_mut().ok_or(())?;
    object.insert("adapter".into(), Value::String(adapter.into()));
    if let Some(secret) = secret {
        object.insert("token".into(), Value::String(secret.expose().into()));
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

fn reject_secret_value(secret: Option<&crate::database::secrets::SecretValue>) -> Result<(), ()> {
    if secret.is_some() { Err(()) } else { Ok(()) }
}

fn vad_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.vad.max_workers,
        voice_reserved_capacity: 1,
        command_capacity: config.workers.vad.command_queue_capacity,
        final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
    }
}
fn asr_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.asr.max_workers,
        voice_reserved_capacity: 1,
        command_capacity: config.workers.asr.command_queue_capacity,
        final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
    }
}
fn tts_worker_config(config: &AppConfig) -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: config.workers.tts.max_workers,
        voice_reserved_capacity: 1,
        command_capacity: config.workers.tts.command_queue_capacity,
        final_timeout: Duration::from_millis(config.tts.timeout_ms),
        cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::secrets::{EnvSecretResolver, SecretValue};

    #[test]
    fn chillaudio_tts_materialization_maps_the_secret_reference_to_its_runtime_token() {
        let mut value = serde_json::json!({
            "ws_url": "wss://tts.example.test/socket",
            "voice": "BV421_vivn_streaming"
        });
        let secret = SecretValue::new("runtime-token".into());
        let instance = typed_tts_instance(&mut value, "chillaudio_ws", Some(&secret)).unwrap();
        let TtsInstanceConfig::ChillAudioWs(config) = instance else {
            panic!("expected ChillAudio runtime configuration");
        };
        assert_eq!(config.token.expose(), "runtime-token");
    }

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

    fn plan(requirement: ProviderLoadRequirement) -> ProviderLoadPlan {
        let key = "db-llm".to_owned();
        match requirement {
            ProviderLoadRequirement::Required => ProviderLoadPlan::new([key], []),
            ProviderLoadRequirement::Optional => ProviderLoadPlan::new([], [key]),
            ProviderLoadRequirement::Unbound => ProviderLoadPlan::default(),
        }
    }

    fn valid_config() -> &'static str {
        r#"{"base_url":"https://example.test/v1","model":"x"}"#
    }

    #[test]
    fn optional_and_unbound_failures_stay_unavailable_without_loading_a_runtime() {
        let invalid = materialize_database_providers(
            &config(),
            vec![row(r#"{"model":"x","unexpected":true}"#, None)],
            &plan(ProviderLoadRequirement::Optional),
            &EnvSecretResolver,
        )
        .expect("an optional provider never blocks startup");
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
            vec![row(valid_config(), Some("VOICE_AGENT_TEST_MISSING_SECRET"))],
            &plan(ProviderLoadRequirement::Optional),
            &EnvSecretResolver,
        )
        .expect("an optional provider never blocks startup");
        assert_eq!(
            unavailable_secret.runtime_state("db-llm", 3).status,
            DatabaseRuntimeStatus::Unavailable
        );
        assert_eq!(
            unavailable_secret.runtime_state("db-llm", 3).failure,
            Some(DatabaseRuntimeFailure::Secret)
        );
        assert!(
            !unavailable_secret
                .loaded
                .runtimes
                .llm
                .contains_key("db-llm")
        );
    }

    #[test]
    fn a_required_failure_reports_the_coarse_reason_and_blocks_startup() {
        let Err(error) = materialize_database_providers(
            &config(),
            vec![row(valid_config(), Some("VOICE_AGENT_TEST_MISSING_SECRET"))],
            &plan(ProviderLoadRequirement::Required),
            &EnvSecretResolver,
        ) else {
            panic!("a required provider must block startup");
        };
        assert_eq!(error.provider_key, "db-llm");
        assert_eq!(error.failure, DatabaseRuntimeFailure::Secret);
    }

    #[test]
    fn an_unbound_provider_never_resolves_a_secret_or_builds_a_runtime() {
        let materialization = materialize_database_providers(
            &config(),
            vec![row(valid_config(), Some("VOICE_AGENT_TEST_MISSING_SECRET"))],
            &plan(ProviderLoadRequirement::Unbound),
            &EnvSecretResolver,
        )
        .expect("unbound configuration never blocks boot");
        assert_eq!(
            materialization.runtime_state("db-llm", 3).status,
            DatabaseRuntimeStatus::NotLoaded
        );
        assert!(
            !materialization.loaded.runtimes.llm.contains_key("db-llm"),
            "an unbound provider must not build a runtime"
        );
    }

    #[test]
    fn an_unbound_provider_with_invalid_config_is_unavailable_without_blocking_boot() {
        let materialization = materialize_database_providers(
            &config(),
            vec![row(r#"{"model":"x","unexpected":true}"#, None)],
            &plan(ProviderLoadRequirement::Unbound),
            &EnvSecretResolver,
        )
        .expect("unbound configuration never blocks boot");
        assert_eq!(
            materialization.runtime_state("db-llm", 3).status,
            DatabaseRuntimeStatus::Unavailable
        );
        assert_eq!(
            materialization.runtime_state("db-llm", 3).failure,
            Some(DatabaseRuntimeFailure::Configuration)
        );
    }
}
