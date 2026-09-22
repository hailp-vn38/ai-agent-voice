use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use voice_agent_server::models::{
    ModelAcquirer, ModelError, ModelPreparation, ModelPreparationConfig,
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
