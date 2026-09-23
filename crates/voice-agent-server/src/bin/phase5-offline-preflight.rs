use std::env;

use anyhow::{Context, Result, bail};
use opus2::{Channels, Decoder};
use voice_agent_server::{
    config::AppConfig,
    models::{ModelError, verify_installed},
    providers::vad::verify_onnx_runtime,
};

const FIXTURES: [(&str, &[u8], bool); 5] = [
    (
        "silence A",
        include_bytes!(
            "../../../voice-reference-client/tests/fixtures/phase5-uplink-01-silence.opus"
        ),
        false,
    ),
    (
        "speech A",
        include_bytes!(
            "../../../voice-reference-client/tests/fixtures/phase5-uplink-02-speech-a.opus"
        ),
        true,
    ),
    (
        "silence B",
        include_bytes!(
            "../../../voice-reference-client/tests/fixtures/phase5-uplink-03-silence.opus"
        ),
        false,
    ),
    (
        "speech B",
        include_bytes!(
            "../../../voice-reference-client/tests/fixtures/phase5-uplink-04-speech-b.opus"
        ),
        true,
    ),
    (
        "silence C",
        include_bytes!(
            "../../../voice-reference-client/tests/fixtures/phase5-uplink-05-silence.opus"
        ),
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
    verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        &config
            .providers
            .vad
            .silero_onnx
            .as_ref()
            .context("silero_onnx options are required")?
            .model,
        "silero_onnx",
        &config.deployment,
    )?;
    verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        &config
            .providers
            .asr
            .zipformer_sherpa
            .as_ref()
            .context("zipformer_sherpa options are required")?
            .model,
        "zipformer_sherpa",
        &config.deployment,
    )?;
    verify_installed(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        &config
            .providers
            .tts
            .zerotts_onnx
            .as_ref()
            .context("zerotts_onnx options are required")?
            .model,
        "zerotts_onnx",
        &config.deployment,
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
