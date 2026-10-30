use super::*;
use crate::providers::tts::vieneu::config::{NANO_MODEL_ID, TURBO_MODEL_ID};
use std::collections::HashSet;

#[test]
fn discovered_voices_exactly_match_asset_catalog_for_both_models() {
    let default_discovered = INSPECTOR.inspect(&serde_json::json!({})).unwrap();
    let turbo_discovered = INSPECTOR
        .inspect(&serde_json::json!({
            "model": TURBO_MODEL_ID,
        }))
        .unwrap();
    assert_eq!(default_discovered.voices.len(), assets::TURBO_VOICES.len());
    assert_eq!(turbo_discovered.voices.len(), assets::TURBO_VOICES.len());
    let turbo_ids: HashSet<_> = turbo_discovered
        .voices
        .iter()
        .map(|voice| voice.id)
        .collect();
    assert_eq!(turbo_ids.len(), assets::TURBO_VOICES.len());
    for voice in assets::TURBO_VOICES {
        assert!(turbo_ids.contains(voice.id));
    }

    let nano_discovered = INSPECTOR
        .inspect(&serde_json::json!({
            "model": NANO_MODEL_ID,
        }))
        .unwrap();
    let nano_ids: HashSet<_> = nano_discovered
        .voices
        .iter()
        .map(|voice| voice.id)
        .collect();
    assert_eq!(nano_ids.len(), assets::NANO_VOICES.len());
    for voice in assets::NANO_VOICES {
        assert!(nano_ids.contains(voice.id));
    }
}

#[test]
fn refuses_other_model_identity_and_unknown_fields() {
    assert!(
        INSPECTOR
            .inspect(&serde_json::json!({
                "model": "vieneu_v4",
            }))
            .is_err()
    );
    assert!(
        INSPECTOR
            .inspect(&serde_json::json!({
                "model": TURBO_MODEL_ID,
                "api_key": "secret",
            }))
            .is_err()
    );
}

#[test]
fn exposes_bounded_optional_speed_percent_field() {
    let field = DESCRIPTOR
        .config_schema
        .fields
        .iter()
        .find(|field| field.key == "speed_percent")
        .expect("speed_percent field");
    assert!(matches!(field.field_type, ConfigFieldType::Integer));
    assert!(!field.required);
    assert_eq!(field.minimum, Some(50));
    assert_eq!(field.maximum, Some(200));
}

#[test]
fn runtime_capabilities_isolate_turbo_and_nano_voices_and_sample_rates() {
    let turbo = runtime_capabilities(r#"{"model":"vieneu_v3_turbo_fp32","voice":"hai_dang"}"#)
        .expect("turbo capabilities");
    let turbo_voices = turbo["voices"].as_array().unwrap();
    assert_eq!(turbo_voices.len(), assets::TURBO_VOICES.len());
    assert!(
        turbo_voices
            .iter()
            .all(|v| v["model"] == "vieneu_v3_turbo_fp32")
    );
    assert_eq!(
        turbo["provider_output_sample_rates"],
        serde_json::json!([48_000])
    );

    let nano = runtime_capabilities(r#"{"model":"vieneu_v3_nano_fp32","voice":"minh_quan"}"#)
        .expect("nano capabilities");
    let nano_voices = nano["voices"].as_array().unwrap();
    assert_eq!(nano_voices.len(), assets::NANO_VOICES.len());
    assert!(
        nano_voices
            .iter()
            .all(|v| v["model"] == "vieneu_v3_nano_fp32")
    );
    assert_eq!(
        nano["provider_output_sample_rates"],
        serde_json::json!([24_000])
    );
}
