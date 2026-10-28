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
