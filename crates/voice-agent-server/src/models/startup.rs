//! Prepare artifacts on disk before any provider runtime or its initialization deadline exists.

use super::{ModelError, prepare, prepare_immutable_inner};
use crate::{
    config::{AppConfig, TtsInstanceConfig},
    database::{DesiredProvider, ProviderLoadPlan, ProviderLoadRequirement, provider_config},
};
use std::collections::BTreeMap;

#[derive(Default)]
struct Requirement {
    required: bool,
    immutable: bool,
}

type ModelPlan = BTreeMap<(String, String), Requirement>;

pub(crate) fn prepare_startup(
    config: &AppConfig,
    rows: &[DesiredProvider],
    load_plan: &ProviderLoadPlan,
) -> Result<(), ModelError> {
    let plan = model_plan(config, rows, load_plan)?;
    // Required models first; optional preparation never allocates inference workers.
    for required in [true, false] {
        for ((adapter, identity), requirement) in &plan {
            if requirement.required != required {
                continue;
            }
            tracing::info!(model = %identity, adapter = %adapter, required, "preparing startup model artifacts");
            let result = if requirement.immutable {
                prepare_immutable_inner(
                    &config.deployment.model_manifest,
                    &config.deployment.models.root,
                    config.deployment.models.offline,
                    identity,
                    adapter,
                    &config.deployment,
                    true,
                )
            } else {
                prepare(
                    &config.deployment.model_manifest,
                    &config.deployment.models.root,
                    config.deployment.models.offline,
                    identity,
                    adapter,
                    &config.deployment,
                )
            };
            if let Err(error) = result {
                if required {
                    tracing::error!(model = %identity, adapter = %adapter, %error, "required model preparation failed");
                    return Err(error);
                }
                tracing::warn!(model = %identity, adapter = %adapter, %error, "optional model artifacts unavailable");
            }
        }
    }
    Ok(())
}

fn model_plan(
    config: &AppConfig,
    rows: &[DesiredProvider],
    load_plan: &ProviderLoadPlan,
) -> Result<ModelPlan, ModelError> {
    let mut plan = ModelPlan::new();
    let effective = &config.effective_agent.providers;
    let defaults = &config.provider_defaults;
    let managed = config.provider_runtime.is_some();
    for (key, instance) in &config.providers.vad.instances {
        add_model(
            &mut plan,
            instance.adapter(),
            &instance.silero_onnx().model,
            key == &effective.vad || key == &defaults.vad,
            managed,
        );
    }
    for (key, instance) in &config.providers.asr.instances {
        add_model(
            &mut plan,
            instance.adapter(),
            instance.model(),
            key == &effective.asr || key == &defaults.asr,
            managed,
        );
    }
    for (key, instance) in &config.providers.tts.instances {
        let model = match instance {
            TtsInstanceConfig::ZeroTtsOnnx(config) => &config.model,
            TtsInstanceConfig::KokoroViOnnx(config) => &config.model,
            TtsInstanceConfig::ChillAudioWs(_) => continue,
        };
        add_model(
            &mut plan,
            instance.adapter(),
            model,
            key == &effective.tts || key == &defaults.tts || instance.preload(),
            managed,
        );
    }
    for row in rows {
        let requirement = load_plan.requirement(&row.key);
        if requirement == ProviderLoadRequirement::Unbound
            || matches!(row.adapter.as_str(), "openai" | "chillaudio_ws")
        {
            continue;
        }
        let model = crate::providers::admin_provider_adapter_matches_kind(&row.kind, &row.adapter)
            .then(|| provider_config::validate_raw(&row.adapter, &row.config_json).ok())
            .flatten()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| {
                value
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });
        let Some(model) = model else {
            if requirement == ProviderLoadRequirement::Required {
                return Err(ModelError::ProviderConfiguration(row.key.clone()));
            }
            tracing::warn!(provider_instance = %row.key, "optional provider has invalid model configuration");
            continue;
        };
        add_model(
            &mut plan,
            &row.adapter,
            &model,
            requirement == ProviderLoadRequirement::Required,
            true,
        );
    }
    Ok(plan)
}

fn add_model(plan: &mut ModelPlan, adapter: &str, model: &str, required: bool, immutable: bool) {
    let entry = plan
        .entry((adapter.to_owned(), model.to_owned()))
        .or_default();
    entry.required |= required;
    entry.immutable |= immutable;
}

#[cfg(test)]
mod tests;
