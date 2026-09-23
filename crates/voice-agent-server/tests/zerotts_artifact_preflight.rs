use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use voice_agent_server::{
    config::{DeploymentConfig, ModelAcknowledgement, ZeroTtsOnnxConfig},
    models::{ModelAcquirer, ModelError, ModelPreparation, ModelPreparationConfig, prepare},
    providers::compiled_provider_registry,
    providers::tts::{WarmupPcm, validate_warmup_pcm},
};

const ROLES: &[&str] = &[
    "config",
    "tokenizer",
    "null_voice",
    "voices_index",
    "voice",
    "text_encoder",
    "prefix_step",
    "local_frame_decode",
    "codec_decode_full",
    "codec_decode_step",
    "codec_shared_data",
    "codec_metadata",
    "codec_license",
];

#[test]
fn default_manifest_pins_the_complete_zerotts_default_pack() {
    let manifest = include_str!("../../../models/manifest.toml");

    assert!(manifest.contains("identity = \"zerotts_default\""));
    assert!(manifest.contains("revision = \"c2bfbd67dc648cac455077333f7cf5c18a2e3bb4\""));
    assert!(manifest.contains("license = \"MIT; bundled-codec=Apache-2.0\""));
    for role in ROLES {
        assert!(manifest.contains(&format!("role = \"{role}\"")));
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("voice-agent-{label}-{nonce}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct FixtureAcquirer;

impl ModelAcquirer for FixtureAcquirer {
    fn acquire(&self, _: &str, _: &Path) -> Result<(), ModelError> {
        panic!("offline fixture must never acquire artifacts")
    }
}

fn prepared_pack(
    root: &Path,
    omitted_role: Option<&str>,
) -> voice_agent_server::models::ResolvedModel {
    let mut manifest = String::from(
        r#"
[[model]]
identity = "zerotts_default"
adapter = "zerotts_onnx"
source = "https://huggingface.co/zeroweight-ai/ZeroTTS"
revision = "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4"
license = "MIT; bundled-codec=Apache-2.0"
"#,
    );
    for role in ROLES {
        if Some(*role) == omitted_role {
            continue;
        }
        let bytes = format!("fixture-{role}").into_bytes();
        let relative = format!("zerotts/{role}");
        let path = root.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &bytes).unwrap();
        let hash = sha256(&bytes);
        manifest.push_str(&format!(
            r#"
[[model.artifacts]]
role = "{role}"
remote = "https://example.invalid/{role}"
install_path = "{relative}"
source_sha256 = "{hash}"
sha256 = "{hash}"
transform = "identity"
"#,
        ));
    }
    let manifest_path = root.join("manifest.toml");
    fs::write(&manifest_path, manifest).unwrap();
    ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path,
            root: root.into(),
            offline: true,
        },
        FixtureAcquirer,
    )
    .prepare("zerotts_default", "zerotts_onnx")
    .unwrap()
}

#[test]
fn factory_rejects_a_prepared_zerotts_pack_missing_a_required_role() {
    let root = temp_dir("zerotts-role");
    let model = prepared_pack(&root, Some("codec_shared_data"));
    let factory = compiled_provider_registry()
        .tts_factory("zerotts_onnx")
        .unwrap();
    let error = match factory.build(
        &ZeroTtsOnnxConfig {
            model: "zerotts_default".into(),
            num_threads: 2,
            voice: "maichi".into(),
            delivery_mode: Default::default(),
        },
        &voice_agent_server::config::RuntimeConfig::default(),
        &model,
    ) {
        Ok(_) => panic!("factory accepted a missing required ZeroTTS artifact"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("codec_shared_data"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deployment_requires_the_exact_zerotts_composite_license_acknowledgement() {
    let root = temp_dir("zerotts-license");
    let model = prepared_pack(&root, None);
    let manifest_path = root.join("manifest.toml");
    let deployment = DeploymentConfig {
        model_acknowledgements: vec![ModelAcknowledgement {
            model: "zerotts_default".into(),
            revision: "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4".into(),
            license: "MIT".into(),
        }],
        ..DeploymentConfig::default()
    };

    let error = prepare(
        &manifest_path,
        &root,
        true,
        model.identity(),
        model.adapter(),
        &deployment,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("MIT; bundled-codec=Apache-2.0"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn warmup_pcm_requires_terminal_finite_non_empty_48khz_mono_audio() {
    for pcm in [
        WarmupPcm::new(24_000, 1, vec![0.0]),
        WarmupPcm::new(48_000, 2, vec![0.0]),
        WarmupPcm::new(48_000, 1, Vec::new()),
        WarmupPcm::new(48_000, 1, vec![f32::NAN]),
    ] {
        assert!(validate_warmup_pcm(pcm).is_err());
    }

    validate_warmup_pcm(WarmupPcm::new(48_000, 1, vec![0.0, 0.25, -0.25])).unwrap();
}
