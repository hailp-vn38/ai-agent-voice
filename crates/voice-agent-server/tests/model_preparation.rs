use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use voice_agent_server::config::{DeploymentConfig, ModelAcknowledgement};
use voice_agent_server::models::{
    ModelAcquirer, ModelError, ModelPreparation, ModelPreparationConfig, verify_installed,
};

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

struct FixtureAcquirer(Vec<u8>);

impl ModelAcquirer for FixtureAcquirer {
    fn acquire(&self, _: &str, destination: &std::path::Path) -> Result<(), ModelError> {
        fs::write(destination, &self.0).unwrap();
        Ok(())
    }
}

struct ExpectedSourceAcquirer;

impl ModelAcquirer for ExpectedSourceAcquirer {
    fn acquire(&self, remote: &str, destination: &std::path::Path) -> Result<(), ModelError> {
        assert_eq!(remote, "https://artifacts.example.invalid/voice.bin");
        fs::write(destination, b"voicepack")?;
        Ok(())
    }
}

#[test]
fn prepared_artifact_uses_deployment_source_and_preserves_manifest_checksum() {
    let root = temp_dir("prepared-source");
    let manifest = manifest(&root, "voice.bin", b"voicepack", b"voicepack", "identity");
    let prepared_source = "prepared://deployment/voice.bin";
    let raw = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        raw.replace("https://example.invalid/model", prepared_source),
    )
    .unwrap();
    let sources = [(
        prepared_source.into(),
        "https://artifacts.example.invalid/voice.bin".into(),
    )]
    .into_iter()
    .collect();
    let result = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        ExpectedSourceAcquirer,
    )
    .with_sources(&sources)
    .prepare("test-vad", "silero_onnx")
    .unwrap();
    assert_eq!(
        fs::read(result.artifact("vad").unwrap()).unwrap(),
        b"voicepack"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checksum_failure_keeps_existing_file_and_removes_temporary_files() {
    let root = temp_dir("checksum-failure");
    let manifest = manifest(
        &root,
        "vad/model.onnx",
        b"expected",
        b"expected",
        "identity",
    );
    let installed = root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"existing").unwrap();
    let result = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(b"incorrect".to_vec()),
    )
    .prepare("test-vad", "silero_onnx");
    assert!(matches!(result, Err(ModelError::HashMismatch { .. })));
    assert_eq!(fs::read(&installed).unwrap(), b"existing");
    assert!(!root.join("vad/model.onnx.part").exists());
    assert!(!root.join("vad/model.onnx.transform").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_source_mapping_is_needed_only_when_artifact_is_missing() {
    let root = temp_dir("prepared-source-required");
    let manifest = manifest(&root, "voice.bin", b"voicepack", b"voicepack", "identity");
    let raw = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        raw.replace(
            "https://example.invalid/model",
            "prepared://deployment/voice.bin",
        ),
    )
    .unwrap();
    let preparation = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        ExpectedSourceAcquirer,
    );
    assert!(matches!(
        preparation.prepare("test-vad", "silero_onnx"),
        Err(ModelError::PreparedSourceRequired(_))
    ));
    fs::write(root.join("voice.bin"), b"voicepack").unwrap();
    preparation.prepare("test-vad", "silero_onnx").unwrap();
    fs::remove_dir_all(root).unwrap();
}

fn manifest(
    root: &std::path::Path,
    install_path: &str,
    source: &[u8],
    output: &[u8],
    transform: &str,
) -> PathBuf {
    let path = root.join("manifest.toml");
    fs::write(
        &path,
        format!(
            r#"
[[model]]
identity = "test-vad"
adapter = "silero_onnx"
source = "test"
revision = "v1"
license = "MIT"

[[model.artifacts]]
role = "vad"
remote = "https://example.invalid/model"
install_path = "{install_path}"
source_sha256 = "{}"
sha256 = "{}"
transform = "{transform}"
"#,
            sha256(source),
            sha256(output)
        ),
    )
    .unwrap();
    path
}

fn acknowledged_deployment() -> DeploymentConfig {
    DeploymentConfig {
        model_acknowledgements: vec![ModelAcknowledgement {
            model: "test-vad".into(),
            revision: "v1".into(),
            license: "MIT".into(),
        }],
        ..DeploymentConfig::default()
    }
}

#[test]
fn offline_preflight_accepts_an_acknowledged_verified_artifact_without_acquisition() {
    let root = temp_dir("offline-preflight-pass");
    let manifest = manifest(&root, "vad/model.onnx", b"model", b"model", "identity");
    let installed = root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"model").unwrap();

    let verified = verify_installed(
        &manifest,
        &root,
        "test-vad",
        "silero_onnx",
        &acknowledged_deployment(),
    )
    .unwrap();

    assert_eq!(verified.artifact("vad"), Some(installed.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_preflight_reports_a_corrupt_artifact_without_acquisition() {
    let root = temp_dir("offline-preflight-corrupt");
    let manifest = manifest(&root, "vad/model.onnx", b"model", b"model", "identity");
    let installed = root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"corrupt").unwrap();

    let result = verify_installed(
        &manifest,
        &root,
        "test-vad",
        "silero_onnx",
        &acknowledged_deployment(),
    );

    assert!(matches!(result, Err(ModelError::HashMismatch { .. })));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_preflight_does_not_create_a_missing_model_root() {
    let root = temp_dir("offline-preflight-read-only");
    let manifest = manifest(
        &root,
        "models/vad/model.onnx",
        b"model",
        b"model",
        "identity",
    );
    let model_root = root.join("not-installed");

    let result = verify_installed(
        &manifest,
        &model_root,
        "test-vad",
        "silero_onnx",
        &acknowledged_deployment(),
    );

    assert!(matches!(result, Err(ModelError::MissingArtifact(_))));
    assert!(!model_root.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reuses_a_verified_installed_artifact_by_role_without_network() {
    let root = temp_dir("model-reuse");
    let installed = root.join("vad/silero_vad.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"verified model").unwrap();
    let manifest = root.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            r#"
[[model]]
identity = "test-vad"
adapter = "silero_onnx"
source = "test"
revision = "v1"
license = "MIT"

[[model.artifacts]]
role = "vad"
remote = "https://example.invalid/silero_vad.onnx"
install_path = "vad/silero_vad.onnx"
source_sha256 = "{}"
sha256 = "{}"
transform = "identity"
"#,
            sha256(b"verified model"),
            sha256(b"verified model")
        ),
    )
    .unwrap();

    let prepared = ModelPreparation::new(ModelPreparationConfig {
        manifest_path: manifest,
        root: root.clone(),
        offline: true,
    })
    .prepare("test-vad", "silero_onnx")
    .unwrap();

    assert_eq!(prepared.artifact("vad").unwrap(), installed.as_path());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replaces_a_corrupt_artifact_and_discards_an_interrupted_part() {
    let root = temp_dir("model-replace");
    let installed = root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"corrupt").unwrap();
    fs::write(root.join("vad/model.onnx.part"), b"interrupted").unwrap();
    let manifest = manifest(&root, "vad/model.onnx", b"source", b"source", "identity");

    let prepared = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(b"source".to_vec()),
    )
    .prepare("test-vad", "silero_onnx")
    .unwrap();

    assert_eq!(
        fs::read(prepared.artifact("vad").unwrap()).unwrap(),
        b"source"
    );
    assert!(!root.join("vad/model.onnx.part").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verifies_transformed_output_before_atomic_install() {
    let root = temp_dir("model-transform");
    let manifest = manifest(
        &root,
        "vad/model.onnx",
        b"HEADmodel",
        b"model",
        "strip_prefix:4",
    );

    let prepared = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(b"HEADmodel".to_vec()),
    )
    .prepare("test-vad", "silero_onnx")
    .unwrap();

    assert_eq!(
        fs::read(prepared.artifact("vad").unwrap()).unwrap(),
        b"model"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transforms_a_sentencepiece_model_into_provider_tokens() {
    let root = temp_dir("model-sentencepiece");
    let source = b"\x0a\x03\x0a\x01A\x0a\x03\x0a\x01B";
    let manifest = manifest(
        &root,
        "asr/tokens.txt",
        source,
        b"A 0\nB 1\n",
        "sentencepiece_tokens_v1",
    );

    let prepared = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(source.to_vec()),
    )
    .prepare("test-vad", "silero_onnx")
    .unwrap();

    assert_eq!(
        fs::read(prepared.artifact("vad").unwrap()).unwrap(),
        b"A 0\nB 1\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_missing_artifact_fails_without_invoking_acquisition() {
    let root = temp_dir("model-offline");
    let manifest = manifest(&root, "vad/model.onnx", b"source", b"source", "identity");

    let result = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: true,
        },
        FixtureAcquirer(b"should-not-be-written".to_vec()),
    )
    .prepare("test-vad", "silero_onnx");

    assert!(matches!(result, Err(ModelError::Offline(_))));
    assert!(!root.join("vad/model.onnx").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_absolute_and_traversal_install_paths_before_acquisition() {
    for install_path in ["../escape.onnx", "/tmp/escape.onnx"] {
        let root = temp_dir("model-path");
        let manifest = manifest(&root, install_path, b"source", b"source", "identity");
        let result = ModelPreparation::with_acquirer(
            ModelPreparationConfig {
                manifest_path: manifest,
                root: root.clone(),
                offline: false,
            },
            FixtureAcquirer(b"source".to_vec()),
        )
        .prepare("test-vad", "silero_onnx");
        assert!(matches!(result, Err(ModelError::UnsafePath(_))));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn rejects_an_install_path_that_escapes_the_root_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("model-symlink");
    let outside = temp_dir("model-outside");
    symlink(&outside, root.join("vad")).unwrap();
    let manifest = manifest(&root, "vad/model.onnx", b"source", b"source", "identity");
    let result = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: manifest,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(b"source".to_vec()),
    )
    .prepare("test-vad", "silero_onnx");

    assert!(matches!(result, Err(ModelError::UnsafePath(_))));
    assert!(!outside.join("model.onnx").exists());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn immutable_artifact_tree_preserves_old_resources_after_mutable_alias_changes() {
    use voice_agent_server::models::prepare_immutable;
    let root = temp_dir("immutable-model");
    let installed = root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"old-model").unwrap();
    let manifest_path = manifest(
        &root,
        "vad/model.onnx",
        b"old-model",
        b"old-model",
        "identity",
    );
    let deployment = DeploymentConfig {
        model_acknowledgements: vec![ModelAcknowledgement {
            model: "test-vad".into(),
            revision: "v1".into(),
            license: "MIT".into(),
        }],
        ..Default::default()
    };
    let old = prepare_immutable(
        &manifest_path,
        &root,
        true,
        "test-vad",
        "silero_onnx",
        &deployment,
    )
    .unwrap();
    fs::write(&installed, b"new-model").unwrap();
    manifest(
        &root,
        "vad/model.onnx",
        b"new-model",
        b"new-model",
        "identity",
    );
    let new = prepare_immutable(
        &manifest_path,
        &root,
        true,
        "test-vad",
        "silero_onnx",
        &deployment,
    )
    .unwrap();
    assert_ne!(old.artifact("vad"), new.artifact("vad"));
    assert_eq!(
        fs::read(old.artifact("vad").unwrap()).unwrap(),
        b"old-model"
    );
    assert_eq!(
        fs::read(new.artifact("vad").unwrap()).unwrap(),
        b"new-model"
    );
    fs::remove_dir_all(root).unwrap();
}

fn kokoro_tensor_archive(metadata: &[u8]) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("voice/data.pkl", metadata.to_vec()),
        ("voice/byteorder", b"little".to_vec()),
        ("voice/data/0", vec![0; 510 * 256 * 4]),
    ] {
        archive.start_file(name, options).unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn kokoro_voicepack_is_transformed_deterministically_before_atomic_install() {
    let root = temp_dir("kokoro-transform");
    let source = kokoro_tensor_archive(include_bytes!("../src/models/kokoro_tensor_v1.pkl"));
    // Independent specification: magic, LE shape 510 x 1 x 256, then LE float32 storage.
    let mut output = b"KOVI_VOICEPACK_V1".to_vec();
    output.extend_from_slice(&510u32.to_le_bytes());
    output.extend_from_slice(&1u32.to_le_bytes());
    output.extend_from_slice(&256u32.to_le_bytes());
    output.resize(output.len() + 510 * 256 * 4, 0);
    let manifest_path = manifest(&root, "voice.bin", &source, &output, "kokoro_voicepack_v1");
    let preparation = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(source),
    );
    let model = preparation.prepare("test-vad", "silero_onnx").unwrap();
    assert_eq!(fs::read(model.artifact("vad").unwrap()).unwrap(), output);
    // Reuse must bypass both network and conversion after verification.
    let verified = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: root.join("manifest.toml"),
            root: root.clone(),
            offline: true,
        },
        FixtureAcquirer(b"must not be used".to_vec()),
    )
    .prepare("test-vad", "silero_onnx")
    .unwrap();
    assert_eq!(fs::read(verified.artifact("vad").unwrap()).unwrap(), output);
    fs::remove_file(model.artifact("vad").unwrap()).unwrap();
    let again = preparation.prepare("test-vad", "silero_onnx").unwrap();
    assert_eq!(fs::read(again.artifact("vad").unwrap()).unwrap(), output);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn kokoro_transform_rejects_unrecognized_pickle_without_installing_or_executing_it() {
    let root = temp_dir("kokoro-invalid-transform");
    let source = kokoro_tensor_archive(b"unrecognized pickle payload");
    let path = manifest(
        &root,
        "voice.bin",
        &source,
        b"expected",
        "kokoro_voicepack_v1",
    );
    fs::write(root.join("voice.bin"), b"previous artifact").unwrap();
    let result = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: path,
            root: root.clone(),
            offline: false,
        },
        FixtureAcquirer(source),
    )
    .prepare("test-vad", "silero_onnx");
    assert!(matches!(result, Err(ModelError::UnsupportedTransform(_))));
    assert_eq!(
        fs::read(root.join("voice.bin")).unwrap(),
        b"previous artifact"
    );
    assert!(!root.join("voice.bin.part").exists());
    assert!(!root.join("voice.bin.transform").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires pinned voices and installed base artifacts; set KOKORO_PREPARATION_FIXTURE_DIR and PROVIDER_QUALIFICATION_CONFIG"]
fn pinned_kokoro_sources_prepare_every_voice_with_verified_output_checksums() {
    struct LocalVoices(PathBuf);
    impl ModelAcquirer for LocalVoices {
        fn acquire(&self, remote: &str, destination: &std::path::Path) -> Result<(), ModelError> {
            let filename = remote.rsplit('/').next().unwrap();
            fs::copy(self.0.join("voicepacks").join(filename), destination)?;
            Ok(())
        }
    }
    let source_root = PathBuf::from(
        std::env::var("KOKORO_PREPARATION_FIXTURE_DIR").expect("pinned source fixture directory"),
    );
    let root = temp_dir("real-kokoro-preparation");
    let manifest: toml::Value =
        toml::from_str(include_str!("../../../models/manifest.toml")).unwrap();
    let model = manifest["model"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["identity"].as_str() == Some("kokoro_vi_contextbox"))
        .unwrap()
        .clone();
    let artifacts = model["artifacts"].as_array().unwrap().clone();
    assert_eq!(
        artifacts
            .iter()
            .filter(|artifact| artifact["role"].as_str().unwrap().starts_with("voicepack_"))
            .count(),
        14
    );
    let config_path = std::env::var("PROVIDER_QUALIFICATION_CONFIG").expect("deployment config");
    let mut config =
        voice_agent_server::config::AppConfig::parse_and_resolve(&config_path).unwrap();
    let base = std::path::Path::new(&config_path).parent().unwrap();
    for value in [
        &mut config.deployment.models.root,
        &mut config.runtime.onnx.library,
        &mut config.runtime.kokoro_vi.g2p_executable,
    ] {
        if value.is_relative() {
            *value = base.join(&*value);
        }
    }
    for artifact in &artifacts {
        if artifact["role"].as_str().unwrap().starts_with("voicepack_") {
            continue;
        }
        let relative = artifact["install_path"].as_str().unwrap();
        let destination = root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::hard_link(config.deployment.models.root.join(relative), destination).unwrap();
    }
    let manifest_path = root.join("manifest.toml");
    let table = toml::Value::Table(
        [("model".into(), toml::Value::Array(vec![model]))]
            .into_iter()
            .collect(),
    );
    fs::write(&manifest_path, toml::to_string(&table).unwrap()).unwrap();
    let preparation = ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path,
            root: root.clone(),
            offline: false,
        },
        LocalVoices(source_root),
    );
    let resolved = preparation
        .prepare("kokoro_vi_contextbox", "kokoro_vi_onnx")
        .unwrap();
    let factory = voice_agent_server::providers::compiled_provider_registry()
        .tts_factory("kokoro_vi_onnx")
        .unwrap();
    for artifact in artifacts {
        let role = artifact["role"].as_str().unwrap();
        let Some(voice) = role.strip_prefix("voicepack_") else {
            continue;
        };
        let selection = voice_agent_server::config::TtsInstanceConfig::KokoroViOnnx(
            voice_agent_server::config::KokoroViOnnxConfig {
                voice: voice.into(),
                ..Default::default()
            },
        );
        assert!(
            factory
                .build(&selection, &config.runtime, Some(&resolved))
                .is_ok(),
            "{voice}"
        );
        let role = artifact["role"].as_str().unwrap();
        let output = fs::read(resolved.artifact(role).unwrap()).unwrap();
        assert_eq!(
            sha256(&output),
            artifact["sha256"].as_str().unwrap(),
            "{role}"
        );
        assert!(output.starts_with(b"KOVI_VOICEPACK_V1"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_provider_preparation_acquires_each_shared_voice_once() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    struct CountAcquisitions(Arc<AtomicUsize>);
    impl ModelAcquirer for CountAcquisitions {
        fn acquire(&self, _: &str, destination: &std::path::Path) -> Result<(), ModelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::yield_now();
            fs::write(destination, b"verified voice")?;
            Ok(())
        }
    }
    let root = temp_dir("concurrent-voice-preparation");
    let path = manifest(
        &root,
        "voices/maichi.bin",
        b"verified voice",
        b"verified voice",
        "identity",
    );
    let raw = fs::read_to_string(&path)
        .unwrap()
        .replace("role = \"vad\"", "role = \"voice_maichi\"");
    let hash = sha256(b"verified voice");
    fs::write(&path, format!("{raw}\n[[model.artifacts]]\nrole = \"voice_baotrang\"\nremote = \"https://example.invalid/baotrang\"\ninstall_path = \"voices/baotrang.bin\"\nsource_sha256 = \"{hash}\"\nsha256 = \"{hash}\"\n")).unwrap();
    let acquired = Arc::new(AtomicUsize::new(0));
    let preparation = Arc::new(ModelPreparation::with_acquirer(
        ModelPreparationConfig {
            manifest_path: path,
            root: root.clone(),
            offline: false,
        },
        CountAcquisitions(Arc::clone(&acquired)),
    ));
    let start = Arc::new(Barrier::new(5));
    let providers: Vec<_> = (0..4)
        .map(|_| {
            let start = Arc::clone(&start);
            let preparation = Arc::clone(&preparation);
            std::thread::spawn(move || {
                start.wait();
                preparation.prepare("test-vad", "silero_onnx").unwrap()
            })
        })
        .collect();
    start.wait();
    for provider in providers {
        let model = provider.join().unwrap();
        for role in ["voice_maichi", "voice_baotrang"] {
            assert_eq!(
                fs::read(model.artifact(role).unwrap()).unwrap(),
                b"verified voice"
            );
        }
    }
    assert_eq!(acquired.load(Ordering::SeqCst), 2);
    assert!(!root.join("voices/maichi.bin.part").exists());
    assert!(!root.join("voices/baotrang.bin.transform").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_immutable_model_is_prepared_once_and_reused_by_later_materializations() {
    use sha2::{Digest, Sha256};
    use voice_agent_server::{
        config::{DeploymentConfig, ModelAcknowledgement, ModelStoreConfig},
        models::{PreparedModelCatalog, model_fingerprint},
    };

    let root = temp_dir("prepared-model-catalog");
    let bytes = b"catalog-artifact".to_vec();
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::create_dir_all(root.join("catalog")).unwrap();
    std::fs::write(root.join("catalog/artifact.bin"), &bytes).unwrap();
    let manifest = root.join("manifest.toml");
    std::fs::write(
        &manifest,
        format!(
            r#"
[[model]]
identity = "catalog_default"
adapter = "catalog_adapter"
source = "https://example.invalid/catalog"
revision = "rev-1"
license = "MIT"

[[model.artifacts]]
role = "graph"
remote = "https://example.invalid/graph"
install_path = "catalog/artifact.bin"
source_sha256 = "{hash}"
sha256 = "{hash}"
transform = "identity"
"#
        ),
    )
    .unwrap();
    let deployment = DeploymentConfig {
        model_manifest: manifest.clone(),
        models: ModelStoreConfig {
            root: root.clone(),
            offline: true,
            ..ModelStoreConfig::default()
        },
        model_acknowledgements: vec![ModelAcknowledgement {
            model: "catalog_default".into(),
            revision: "rev-1".into(),
            license: "MIT".into(),
        }],
        ..DeploymentConfig::default()
    };
    let fingerprint = model_fingerprint(&manifest, "catalog_default", "catalog_adapter").unwrap();

    let catalog = PreparedModelCatalog::new();
    let first = catalog
        .resolve(
            &manifest,
            &root,
            true,
            "catalog_default",
            "catalog_adapter",
            &deployment,
        )
        .unwrap();
    assert!(first.prepared);
    assert_eq!(first.model.fingerprint(), fingerprint);

    let second = catalog
        .resolve(
            &manifest,
            &root,
            true,
            "catalog_default",
            "catalog_adapter",
            &deployment,
        )
        .unwrap();
    assert!(
        !second.prepared,
        "a second materialization must not re-prepare"
    );
    assert_eq!(second.model.fingerprint(), first.model.fingerprint());
    assert!(
        std::sync::Arc::ptr_eq(&first.model, &second.model),
        "a reused preparation must be the same trusted result, not a re-verified copy"
    );

    // Editing the manifest changes immutable content, so the next resolution must prepare again.
    std::fs::write(
        &manifest,
        std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("revision = \"rev-1\"", "revision = \"rev-2\""),
    )
    .unwrap();
    let changed = catalog
        .resolve(
            &manifest,
            &root,
            true,
            "catalog_default",
            "catalog_adapter",
            &DeploymentConfig {
                model_acknowledgements: vec![ModelAcknowledgement {
                    model: "catalog_default".into(),
                    revision: "rev-2".into(),
                    license: "MIT".into(),
                }],
                ..deployment.clone()
            },
        )
        .unwrap();
    assert!(changed.prepared, "changed manifest content must re-prepare");
    assert_ne!(changed.model.fingerprint(), first.model.fingerprint());

    // A removed pinned tree is never served from the cache.
    std::fs::remove_file(
        root.join(".installed")
            .join(changed.model.fingerprint())
            .join("catalog/artifact.bin"),
    )
    .unwrap();
    let repaired = catalog
        .resolve(
            &manifest,
            &root,
            true,
            "catalog_default",
            "catalog_adapter",
            &DeploymentConfig {
                model_acknowledgements: vec![ModelAcknowledgement {
                    model: "catalog_default".into(),
                    revision: "rev-2".into(),
                    license: "MIT".into(),
                }],
                ..deployment.clone()
            },
        )
        .unwrap();
    assert!(
        repaired.prepared,
        "a missing pinned artifact must re-prepare"
    );

    std::fs::remove_dir_all(root).unwrap();
}
