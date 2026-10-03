use super::*;
use crate::config::ModelAcknowledgement;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn config() -> AppConfig {
    toml::from_str(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "primary"
asr = "remote"
llm = "remote"
tts = "remote"
[providers.vad.instances.primary]
adapter = "silero_onnx"
model = "test-vad"
[providers.vad.instances.same_model]
adapter = "silero_onnx"
model = "test-vad"
[providers.vad.instances.optional]
adapter = "silero_onnx"
model = "optional-vad"
"#,
    )
    .unwrap()
}

fn row(key: &str, model: &str) -> DesiredProvider {
    DesiredProvider {
        id: 1,
        key: key.into(),
        kind: "vad".into(),
        adapter: "silero_onnx".into(),
        config_json: serde_json::json!({"model":model,"num_threads":1}).to_string(),
        secret_ref: None,
        revision: 1,
    }
}

#[test]
fn startup_plan_merges_shared_models_and_skips_unbound_database_providers() {
    let rows = vec![
        row("db_required", "test-vad"),
        row("db_optional", "another-vad"),
        row("unbound", "unused-model"),
    ];
    let load_plan = ProviderLoadPlan::new(["db_required".into()], ["db_optional".into()]);
    let plan = model_plan(&config(), &rows, &load_plan).unwrap();
    assert_eq!(plan.len(), 3);
    let shared = &plan[&("silero_onnx".into(), "test-vad".into())];
    assert!(shared.required);
    assert!(shared.immutable);
    assert!(!plan[&("silero_onnx".into(), "another-vad".into())].required);
    assert!(!plan.contains_key(&("silero_onnx".into(), "unused-model".into())));
}

#[test]
fn required_database_configuration_is_validated_before_acquisition() {
    let mut invalid = row("broken", "test-vad");
    invalid.config_json = r#"{"model":"test-vad","num_threads":1,"token":"secret"}"#.into();
    let required = ProviderLoadPlan::new(["broken".into()], []);
    assert!(matches!(
        model_plan(&config(), &[invalid.clone()], &required),
        Err(ModelError::ProviderConfiguration(_))
    ));
    let optional = ProviderLoadPlan::new([], ["broken".into()]);
    assert!(model_plan(&config(), &[invalid], &optional).is_ok());
}

fn fixture() -> (AppConfig, PathBuf) {
    let root = std::env::temp_dir().join(format!("voice-startup-models-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let mut config = config();
    config.deployment.models.root = root.join("models");
    config.deployment.models.offline = true;
    config.deployment.model_manifest = root.join("manifest.toml");
    config.runtime.onnx.library = root.join("uninstalled-onnx-library");
    config.deployment.model_acknowledgements = vec![ModelAcknowledgement {
        model: "test-vad".into(),
        revision: "test".into(),
        license: "MIT".into(),
    }];
    let checksum: String = Sha256::digest(b"fixture-model")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    fs::write(
        &config.deployment.model_manifest,
        format!(
            r#"
[[model]]
identity = "test-vad"
adapter = "silero_onnx"
source = "fixture"
revision = "test"
license = "MIT"
[[model.artifacts]]
role = "vad"
remote = "https://example.invalid/model.onnx"
install_path = "vad/model.onnx"
source_sha256 = "{checksum}"
sha256 = "{checksum}"
"#,
        ),
    )
    .unwrap();
    (config, root)
}

#[test]
fn installed_required_model_is_prepared_without_loading_a_native_runtime() {
    let (config, root) = fixture();
    let installed = config.deployment.models.root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"fixture-model").unwrap();
    let plan = ProviderLoadPlan::from_server_defaults(&config.provider_defaults);
    prepare_startup(&config, &[], &plan).unwrap();
    assert!(!config.runtime.onnx.library.exists());
    assert_eq!(fs::read(installed).unwrap(), b"fixture-model");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_required_model_fails_preparation_in_offline_mode() {
    let (config, root) = fixture();
    let plan = ProviderLoadPlan::from_server_defaults(&config.provider_defaults);
    assert!(matches!(
        prepare_startup(&config, &[], &plan),
        Err(ModelError::Offline(_))
    ));
    assert!(!config.runtime.onnx.library.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fresh_startup_downloads_into_missing_model_directory_and_reuses_it_without_network() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    let (mut config, root) = fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let raw = fs::read_to_string(&config.deployment.model_manifest).unwrap();
    fs::write(
        &config.deployment.model_manifest,
        raw.replace("https://example.invalid/model.onnx", &url),
    )
    .unwrap();
    config.deployment.models.offline = false;
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() <= 16 * 1024);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nfixture-model").unwrap();
    });
    let plan = ProviderLoadPlan::from_server_defaults(&config.provider_defaults);
    assert!(!config.deployment.models.root.exists());
    prepare_startup(&config, &[], &plan).unwrap();
    server.join().unwrap();
    let installed = config.deployment.models.root.join("vad/model.onnx");
    assert_eq!(fs::read(installed).unwrap(), b"fixture-model");
    // The source listener is now closed, so any second acquisition would fail.
    prepare_startup(&config, &[], &plan).unwrap();
    assert!(!config.runtime.onnx.library.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_repairs_corrupt_immutable_copy_but_hot_preparation_refuses_to_replace_it() {
    let (config, root) = fixture();
    let installed = config.deployment.models.root.join("vad/model.onnx");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, b"fixture-model").unwrap();
    let rows = [row("db_required", "test-vad")];
    let plan = ProviderLoadPlan::new(["db_required".into()], []);
    prepare_startup(&config, &rows, &plan).unwrap();
    let pinned = crate::models::prepare_immutable(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        true,
        "test-vad",
        "silero_onnx",
        &config.deployment,
    )
    .unwrap();
    let pinned_path = pinned.artifact("vad").unwrap();
    fs::write(pinned_path, b"corrupt").unwrap();
    assert!(matches!(
        crate::models::prepare_immutable(
            &config.deployment.model_manifest,
            &config.deployment.models.root,
            true,
            "test-vad",
            "silero_onnx",
            &config.deployment,
        ),
        Err(ModelError::HashMismatch { .. })
    ));
    prepare_startup(&config, &rows, &plan).unwrap();
    assert_eq!(fs::read(pinned_path).unwrap(), b"fixture-model");
    fs::remove_dir_all(root).unwrap();
}
