use std::sync::{Arc, Mutex};

use super::{OwnedRuntimeResource, RuntimeResource, tts_binding};
use crate::{
    database::DesiredProvider,
    providers::{LoadedVad, RuntimeCatalog, VadError, VadProvider, VadSession},
    workers::{ProviderRuntimeAdmission, VadWorkerRuntime, WorkerRuntimeConfig},
};

struct TestVad;

impl VadProvider for TestVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Err(VadError::Failed(
            "test provider does not open native sessions".into(),
        ))
    }

    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }
}

#[test]
fn shared_silero_resource_materializes_snapshot_endpoint_policy() {
    let mut catalog = RuntimeCatalog::default();
    catalog.vad.insert(
        "owner".into(),
        LoadedVad {
            runtime: Arc::new(VadWorkerRuntime::new(
                Arc::new(TestVad),
                WorkerRuntimeConfig::default(),
            )),
            segmenter: Default::default(),
            pre_roll_samples: 0,
        },
    );
    let resource = OwnedRuntimeResource {
        readiness: Default::default(),
        physical_admission: ProviderRuntimeAdmission::new(8, 1),
        resource_key: None,
        health_flags: Vec::new(),
        catalog: Mutex::new(Some(catalog)),
        capabilities: None,
        timings: Default::default(),
    };
    let snapshot = DesiredProvider {
        id: 0,
        key: "logical-vad".into(),
        kind: "vad".into(),
        adapter: "silero_onnx".into(),
        revision: 1,
        config_json: r#"{"speech_threshold":0.7,"exit_threshold":0.4,"min_speech_ms":250,"end_silence_ms":800,"pre_roll_ms":100}"#.into(),
        secret_ref: None,
    };

    let view = resource
        .runtimes_for(&snapshot, ProviderRuntimeAdmission::new(8, 1))
        .expect("valid logical VAD view");
    let loaded = &view.vad["logical-vad"];
    assert_eq!(loaded.segmenter.speech_threshold, 0.7);
    assert_eq!(loaded.segmenter.exit_threshold, 0.4);
    assert_eq!(loaded.segmenter.min_speech_samples, 4_000);
    assert_eq!(loaded.segmenter.end_silence_samples, 12_800);
    assert_eq!(loaded.pre_roll_samples, 1_600);
}

#[test]
fn a_shared_zerotts_runtime_rebinds_voice_without_changing_physical_state() {
    // Template A and Template B share one resident ZeroTTS runtime, so the only thing that may
    // differ between their logical views is the voice binding itself.
    let snapshot = |voice: &str| DesiredProvider {
        id: 1,
        key: format!("tts-{voice}"),
        kind: "tts".into(),
        adapter: "zerotts_onnx".into(),
        revision: 1,
        config_json: format!(
            r#"{{"voice":"{voice}","language":"vi-VN","delivery_mode":"stream"}}"#
        ),
        secret_ref: None,
    };

    let maichi = tts_binding(&snapshot("maichi")).expect("maichi is a pinned ZeroTTS voice");
    let hamy = tts_binding(&snapshot("hamy")).expect("hamy is a pinned ZeroTTS voice");
    assert_eq!(maichi.voice, "maichi");
    assert_eq!(hamy.voice, "hamy");
    assert_eq!(maichi.language, "vi-VN");

    let unsupported = snapshot("not-a-zerotts-voice");
    assert!(
        tts_binding(&unsupported).is_none(),
        "a logical view must never fall back to a registry default voice"
    );
}
