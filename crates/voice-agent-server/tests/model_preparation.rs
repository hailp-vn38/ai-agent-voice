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
