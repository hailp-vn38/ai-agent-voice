use super::*;
use crate::providers::tts::vieneu::assets::{
    DEFAULT_VOICE, NANO_DEFAULT_VOICE, NANO_VOICES, TURBO_VOICES,
};

#[test]
fn default_selection_is_in_local_voice_catalog() {
    let config = VieneuNativeConfig::default();
    assert!(config.valid_selection());
    assert_eq!(config.model, TURBO_MODEL_ID);
    assert_eq!(config.parsed_model(), Some(VieneuModel::V3TurboFp32));
    assert_eq!(config.voice, DEFAULT_VOICE);
    assert_eq!(config.language, "vi-VN");
}

#[test]
fn nano_model_defaults_to_nano_default_voice_when_omitted() {
    let config: VieneuNativeConfig =
        serde_json::from_str(r#"{"model":"vieneu_v3_nano_fp32"}"#).unwrap();
    assert!(config.valid_selection());
    assert_eq!(config.model, NANO_MODEL_ID);
    assert_eq!(config.parsed_model(), Some(VieneuModel::V3NanoFp32));
    assert_eq!(config.voice, NANO_DEFAULT_VOICE);
    assert_eq!(config.language, "vi-VN");
}

#[test]
fn rejects_cross_model_voices_and_accepts_shared_voices() {
    let turbo_with_nano_only: VieneuNativeConfig =
        serde_json::from_str(r#"{"model":"vieneu_v3_turbo_fp32","voice":"minh_quan"}"#).unwrap();
    assert!(!turbo_with_nano_only.valid_selection());

    let nano_with_turbo_only: VieneuNativeConfig =
        serde_json::from_str(r#"{"model":"vieneu_v3_nano_fp32","voice":"hai_dang"}"#).unwrap();
    assert!(!nano_with_turbo_only.valid_selection());

    for shared in ["adam", "my_duyen", "duc_tri", "mai_anh", "truc_ly"] {
        let turbo: VieneuNativeConfig = serde_json::from_value(serde_json::json!({
            "model": TURBO_MODEL_ID,
            "voice": shared,
        }))
        .unwrap();
        assert!(
            turbo.valid_selection(),
            "turbo must accept shared voice {shared}"
        );

        let nano: VieneuNativeConfig = serde_json::from_value(serde_json::json!({
            "model": NANO_MODEL_ID,
            "voice": shared,
        }))
        .unwrap();
        assert!(
            nano.valid_selection(),
            "nano must accept shared voice {shared}"
        );
    }

    for voice in TURBO_VOICES {
        let cfg = VieneuNativeConfig {
            model: TURBO_MODEL_ID.into(),
            voice: voice.id.into(),
            ..Default::default()
        };
        assert!(cfg.valid_selection());
    }
    for voice in NANO_VOICES {
        let cfg = VieneuNativeConfig {
            model: NANO_MODEL_ID.into(),
            voice: voice.id.into(),
            ..Default::default()
        };
        assert!(cfg.valid_selection());
    }
}

#[test]
fn rejects_unknown_options_and_credentials() {
    for invalid in [
        r#"{"api_key":"secret"}"#,
        r#"{"model_path":"/tmp/other.onnx"}"#,
        r#"{"precision":"int8"}"#,
        r#"{"base_url":"http://localhost"}"#,
    ] {
        assert!(serde_json::from_str::<VieneuNativeConfig>(invalid).is_err());
    }
}

#[test]
fn rejects_invalid_voice_language_threads_or_model() {
    let mut value = VieneuNativeConfig {
        voice: "not-in-catalog".into(),
        ..Default::default()
    };
    assert!(!value.valid_selection());
    value.voice = DEFAULT_VOICE.into();
    value.language = "en-US".into();
    assert!(!value.valid_selection());
    value.language = "vi-VN".into();
    value.num_threads = 0;
    assert!(!value.valid_selection());
    value.num_threads = 1;
    value.model = "other-model".into();
    assert!(!value.valid_selection());
}

#[test]
fn speed_percent_defaults_to_natural_rate() {
    assert_eq!(VieneuNativeConfig::default().speed_percent, 100);
    let config: VieneuNativeConfig =
        serde_json::from_str(r#"{"model":"vieneu_v3_nano_fp32"}"#).unwrap();
    assert_eq!(config.speed_percent, 100);
    assert!((config.speed_factor() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn nano_accepts_bounded_speed_percent() {
    for (speed, valid) in [
        (49, false),
        (50, true),
        (100, true),
        (150, true),
        (200, true),
        (201, false),
    ] {
        let config: VieneuNativeConfig = serde_json::from_value(serde_json::json!({
            "model": NANO_MODEL_ID,
            "speed_percent": speed,
        }))
        .unwrap();
        assert_eq!(
            config.valid_selection(),
            valid,
            "nano speed_percent {speed}"
        );
    }
}

#[test]
fn turbo_rejects_non_natural_speed_percent() {
    for (speed, valid) in [(100, true), (90, false), (150, false)] {
        let config: VieneuNativeConfig = serde_json::from_value(serde_json::json!({
            "model": TURBO_MODEL_ID,
            "speed_percent": speed,
        }))
        .unwrap();
        assert_eq!(
            config.valid_selection(),
            valid,
            "turbo speed_percent {speed}"
        );
    }
}

#[test]
fn speed_percent_round_trips_through_serialization() {
    let config = VieneuNativeConfig {
        model: NANO_MODEL_ID.into(),
        voice: NANO_DEFAULT_VOICE.into(),
        speed_percent: 125,
        ..Default::default()
    };
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(value["speed_percent"], 125);
    let parsed: VieneuNativeConfig = serde_json::from_value(value).unwrap();
    assert_eq!(parsed.speed_percent, 125);
    assert!((parsed.speed_factor() - 1.25).abs() < 1e-6);
}

#[test]
fn hides_deployment_owned_fields_when_serializing() {
    let value = serde_json::to_value(VieneuNativeConfig::default()).unwrap();
    assert_eq!(value["model"], TURBO_MODEL_ID);
    assert!(value.get("num_threads").is_none());
    assert_eq!(value["voice"], DEFAULT_VOICE);
}
