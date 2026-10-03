use std::env;

use anyhow::{Context, Result, bail};
use opus2::{Channels, Decoder};
use voice_agent_server::{
    config::AppConfig,
    models::{ModelError, verify_installed},
    providers::{compiled_provider_registry, vad::verify_onnx_runtime},
};

const FIXTURES: [(&str, &[u8], bool); 5] = [
    (
        "silence A",
        include_bytes!("../../tests/fixtures/phase5-uplink-01-silence.opus"),
        false,
    ),
    (
        "speech A",
        include_bytes!("../../tests/fixtures/phase5-uplink-02-speech-a.opus"),
        true,
    ),
    (
        "silence B",
        include_bytes!("../../tests/fixtures/phase5-uplink-03-silence.opus"),
        false,
    ),
    (
        "speech B",
        include_bytes!("../../tests/fixtures/phase5-uplink-04-speech-b.opus"),
        true,
    ),
    (
        "silence C",
        include_bytes!("../../tests/fixtures/phase5-uplink-05-silence.opus"),
        false,
    ),
];

fn main() {
    match run() {
        Ok(()) => println!("phase5-preflight: pass"),
        Err(error) if unavailable(&error) => {
            eprintln!("phase5-preflight: unavailable: {error:#}");
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("phase5-preflight: fail: {error:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<()> {
    let config_path = env::var("VOICE_AGENT_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let mut config =
        AppConfig::load(&config_path).with_context(|| format!("load {config_path}"))?;
    // This qualification command must never turn a missing artifact into a network request.
    config.deployment.models.offline = true;
    verify_onnx_runtime(&config.runtime.onnx.library)
        .context("load deployment-selected ONNX Runtime")?;
    let vad_instance = &config.providers.vad.instances[&config.effective_agent().providers.vad];
    let asr_instance = &config.providers.asr.instances[&config.effective_agent().providers.asr];
    let tts_instance = &config.providers.tts.instances[&config.effective_agent().providers.tts];
    let vad_model = verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        &vad_instance.silero_onnx().model,
        "silero_onnx",
        &config.deployment,
    )?;
    let asr_model = verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        asr_instance.model(),
        asr_instance.adapter(),
        &config.deployment,
    )?;
    let tts_model = verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        match tts_instance {
            voice_agent_server::config::TtsInstanceConfig::ZeroTtsOnnx(options) => &options.model,
            voice_agent_server::config::TtsInstanceConfig::KokoroViOnnx(options) => &options.model,
            voice_agent_server::config::TtsInstanceConfig::ChillAudioWs(_) => anyhow::bail!(
                "Phase 5 offline preflight requires a local-model effective TTS instance"
            ),
        },
        tts_instance.adapter(),
        &config.deployment,
    )?;
    let registry = compiled_provider_registry();
    registry.vad_factory(vad_instance.adapter())?.build(
        vad_instance,
        &config.runtime,
        &vad_model,
    )?;
    registry.asr_factory(asr_instance.adapter())?.build(
        asr_instance,
        &asr_model,
        usize::try_from(config.audio.max_utterance_ms)
            .context("convert configured ASR capture bound")?
            * 16,
    )?;
    registry.tts_factory(tts_instance.adapter())?.build(
        tts_instance,
        &config.runtime,
        Some(&tts_model),
    )?;
    verify_fixture()?;
    Ok(())
}

fn unavailable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<ModelError>()
            .is_some_and(|model| matches!(model, ModelError::MissingArtifact(_)))
    }) || error
        .to_string()
        .contains("ONNX Runtime dynamic library is missing")
}

fn verify_fixture() -> Result<()> {
    for (name, packet, speech) in FIXTURES {
        let mut decoder =
            Decoder::new(16_000, Channels::Mono).context("create uplink Opus decoder")?;
        let mut samples = [0_i16; 960];
        let decoded = decoder
            .decode(packet, &mut samples, false)
            .with_context(|| format!("decode {name}"))?;
        if decoded != samples.len() {
            bail!("{name} decoded to {decoded} samples, expected 960");
        }
        let peak = samples
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap_or(0);
        if speech && peak <= 1_000 {
            bail!("{name} does not contain the required synthetic speech");
        }
        if !speech && peak >= 100 {
            bail!("{name} is not canonical silence");
        }
    }
    Ok(())
}
