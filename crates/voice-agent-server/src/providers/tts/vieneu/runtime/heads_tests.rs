use super::*;

#[test]
#[ignore = "requires prepared native VieNeu assets and ONNX Runtime"]
fn embeddings_and_streaming_codec_match_pinned_numpy_ort_oracle() {
    use crate::providers::{
        tts::{
            vieneu::{assets, contract::VieneuContract},
            zerotts::runtime::codec::ZeroTtsCodecReplica,
        },
        vad::initialize_ort,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = assets::resolve_assets_from(&root.join("models/TTS/vieneu")).unwrap();
    let contract = VieneuContract::load(&assets).unwrap();
    let heads = Heads::load(&assets.heads, &contract.config).unwrap();
    let voice = contract.voices.get("hai_dang").unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vieneu.json"
    )))
    .unwrap();
    let close = |actual: &[f32], expected: &serde_json::Value| {
        for (value, expected) in actual.iter().zip(expected.as_array().unwrap()) {
            assert!((f64::from(*value) - expected.as_f64().unwrap()).abs() < 0.00002);
        }
    };
    let anchor = heads.anchor(&voice.speaker_embedding).unwrap();
    close(&anchor.as_slice().unwrap()[..16], &oracle["anchor_head"]);
    let embedded = heads
        .embed(
            contract.config.audio_ref_slot_token_id,
            &voice.reference_codes[0],
            &anchor,
        )
        .unwrap();
    close(&embedded[..16], &oracle["reference_embedding_head"]);
    initialize_ort(&root.join("runtime/onnxruntime/libonnxruntime.so")).unwrap();
    let mut codec = ZeroTtsCodecReplica::from_assets(
        &assets.codec_metadata,
        &assets.codec_step,
        &assets.codec_data,
        contract.config.n_vq,
        2,
    )
    .unwrap();
    let frames: Vec<Vec<i32>> = serde_json::from_value(oracle["code_frames"].clone()).unwrap();
    for (frames, expected) in [&frames[..4], &frames[4..]]
        .into_iter()
        .zip(oracle["codec_chunks"].as_array().unwrap())
    {
        let pcm = codec.decode_step(frames).unwrap();
        assert_eq!(
            pcm.samples().len() as u64,
            expected["length"].as_u64().unwrap()
        );
        close(&pcm.samples()[..32], &expected["head"]);
        close(
            &pcm.samples()[pcm.samples().len() - 32..],
            &expected["tail"],
        );
    }
    codec.reset().unwrap();
    let pcm = codec.decode_step(&frames[..4]).unwrap();
    close(&pcm.samples()[..32], &oracle["codec_chunks"][0]["head"]);
}
