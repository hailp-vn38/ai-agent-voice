use crate::{
    config::AppConfig,
    models::prepare,
    providers::{ProviderLoadError, ProviderSet, compiled_provider_registry},
};

/// Startup orchestrates typed selection, Model Preparation, and fixed factory build.
pub(crate) fn load_local(config: &AppConfig) -> Result<ProviderSet, ProviderLoadError> {
    config
        .validate()
        .map_err(|error| ProviderLoadError::Configuration(error.to_string()))?;
    let registry = compiled_provider_registry();
    let vad_factory = registry.vad_factory(&config.providers.vad.adapter)?;
    let asr_factory = registry.asr_factory(&config.providers.asr.adapter)?;
    let vad_identity = vad_factory.model_identity(&config.providers.vad)?;
    let asr_identity = asr_factory.model_identity(&config.providers.asr)?;
    let vad_model = prepare(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        config.deployment.models.offline,
        vad_identity,
        vad_factory.adapter(),
        &config.deployment,
    )?;
    let asr_model = prepare(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        config.deployment.models.offline,
        asr_identity,
        asr_factory.adapter(),
        &config.deployment,
    )?;
    let vad = vad_factory.build(&config.providers.vad, &config.runtime, &vad_model)?;
    let asr = asr_factory.build(&config.providers.asr, &asr_model)?;
    Ok(ProviderSet::with_vad(vad, asr))
}
