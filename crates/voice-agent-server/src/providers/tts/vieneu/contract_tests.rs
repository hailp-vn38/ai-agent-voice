use super::*;
use crate::providers::tts::vieneu::assets::NANO_VOICES;

fn model() -> ModelConfig {
    ModelConfig::parse(
        br#"{
        "n_vq":16,"audio_vocab_size":1024,"text_vocab_size":419,
        "hidden_size":768,"num_hidden_layers":12,
        "local_num_hidden_layers":1,"local_num_attention_heads":8,
        "audio_sample_rate":48000,"audio_pad_token_id":1024,
        "text_prompt_start_token_id":3,"text_prompt_end_token_id":4,
        "speech_generation_start_token_id":5,"speech_generation_end_token_id":6,
        "audio_ref_slot_token_id":7,"default_style_token_id":16,
        "use_speaker_embedding":true,"speaker_embedding_dim":192
    }"#,
    )
    .unwrap()
}

fn nano_config_json() -> Vec<u8> {
    let mut vocab = serde_json::Map::new();
    vocab.insert("<pad>".into(), serde_json::json!(0));
    vocab.insert("<bos>".into(), serde_json::json!(1));
    vocab.insert("<eos>".into(), serde_json::json!(2));
    for idx in 3..81 {
        vocab.insert(format!("p{idx}"), serde_json::json!(idx));
    }
    serde_json::to_vec(&serde_json::json!({
        "model_type": "vieneu_v3_nano",
        "version": "3.0.0-nano",
        "sample_rate": 24000,
        "latent_dim": 24,
        "group": 6,
        "flow_fps": 15.625,
        "dim": 512,
        "style_dim": 256,
        "n_style": 50,
        "steps_default": 16,
        "cfg_default": 3.0,
        "pad_id": 0,
        "bos_id": 1,
        "eos_id": 2,
        "vocab": vocab,
    }))
    .unwrap()
}

#[test]
fn accepts_pinned_model_profile() {
    assert_eq!(model().n_vq, 16);
}

#[test]
fn rejects_legacy_model_without_speaker_anchor() {
    let bytes = br#"{
        "n_vq":16,"audio_vocab_size":1024,"text_vocab_size":419,
        "hidden_size":768,"num_hidden_layers":12,
        "local_num_hidden_layers":1,"local_num_attention_heads":8,
        "audio_sample_rate":48000,"audio_pad_token_id":1024,
        "text_prompt_start_token_id":3,"text_prompt_end_token_id":4,
        "speech_generation_start_token_id":5,"speech_generation_end_token_id":6,
        "audio_ref_slot_token_id":7,"default_style_token_id":16,
        "use_speaker_embedding":false,"speaker_embedding_dim":192
    }"#;
    assert!(ModelConfig::parse(bytes).is_err());
}

#[test]
fn rejects_empty_or_unrecognized_preset_registry() {
    assert!(
        VoiceRegistry::parse(
            r#"{"default_voice":"Hải Đăng","presets":{}}"#.as_bytes(),
            &model(),
        )
        .is_err()
    );
}

#[test]
fn rejects_non_streaming_codec_metadata() {
    let json = br#"{"format_version":2,
        "codec_config":{"sample_rate":48000,"channels":2,
            "downsample_rate":3840,"num_quantizers":16},
        "files":{"decode_step":"moss_audio_tokenizer_decode_step.onnx"},
        "streaming_decode":{"transformer_offsets":[],"attention_caches":[]}
    }"#;
    assert!(validate_codec_metadata(json, &model()).is_err());
}

#[test]
fn nano_config_and_voice_registry_validate_pinned_profile() {
    let cfg = NanoModelConfig::parse(&nano_config_json()).unwrap();
    assert_eq!(cfg.sample_rate, 24_000);
    assert_eq!(cfg.latent_channels(), 144);
    assert_eq!(cfg.dim, 512);
    assert_eq!(cfg.n_style, 50);
    assert_eq!(cfg.style_dim, 256);

    let mut without_model_type =
        serde_json::from_slice::<serde_json::Value>(&nano_config_json()).unwrap();
    without_model_type
        .as_object_mut()
        .unwrap()
        .remove("model_type");
    let cfg_without_model_type =
        NanoModelConfig::parse(&serde_json::to_vec(&without_model_type).unwrap()).unwrap();
    assert_eq!(cfg_without_model_type.model_type, "vieneu_v3_nano");

    let mut bad_cfg = serde_json::from_slice::<serde_json::Value>(&nano_config_json()).unwrap();
    bad_cfg["latent_dim"] = serde_json::json!(16);
    assert!(NanoModelConfig::parse(&serde_json::to_vec(&bad_cfg).unwrap()).is_err());

    let mut presets = serde_json::Map::new();
    for voice in NANO_VOICES {
        presets.insert(
            voice.source_name.into(),
            serde_json::json!({
                "speaker_emb": vec![0.1_f32; NANO_SPEAKER_DIM],
                "style": vec![vec![0.05_f32; 256]; 50],
            }),
        );
    }
    let registry_bytes = serde_json::to_vec(&serde_json::json!({
        "default_voice": "Minh Quân",
        "presets": presets,
    }))
    .unwrap();
    let registry = NanoVoiceRegistry::parse(&registry_bytes, &cfg).unwrap();
    assert_eq!(registry.default_voice, "minh_quan");
    let minh_quan = registry.get("minh_quan").unwrap();
    assert_eq!(minh_quan.speaker_embedding.len(), 192);
    assert_eq!(minh_quan.style.len(), 50 * 256);

    assert!(
        NanoVoiceRegistry::parse(
            r#"{"default_voice":"Minh Quân","presets":{}}"#.as_bytes(),
            &cfg
        )
        .is_err()
    );

    let npz_path = std::env::temp_dir().join(format!(
        "vieneu_nano_constants_test_{}.npz",
        std::process::id()
    ));
    {
        use ndarray::{Array1, Array2};
        use ndarray_npy::WriteNpyExt;
        use std::io::Write;
        use zip::{ZipWriter, write::SimpleFileOptions};

        let file = std::fs::File::create(&npz_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let mut spk_buf = Vec::new();
        Array1::from_elem(NANO_SPEAKER_DIM, 0.25_f32)
            .write_npy(&mut spk_buf)
            .unwrap();
        zip.start_file("null_spk.npy", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&spk_buf).unwrap();

        let mut style_buf = Vec::new();
        Array2::from_elem((cfg.n_style, cfg.style_dim), -0.125_f32)
            .write_npy(&mut style_buf)
            .unwrap();
        zip.start_file("null_style.npy", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&style_buf).unwrap();
        zip.finish().unwrap();
    }
    let (null_spk, null_style) = load_nano_constants(&npz_path, &cfg).unwrap();
    let _ = std::fs::remove_file(&npz_path);
    assert_eq!(null_spk.len(), NANO_SPEAKER_DIM);
    assert_eq!(null_style.len(), cfg.n_style * cfg.style_dim);
}
