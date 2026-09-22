use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use voice_agent_server::providers::tts::zerotts_onnx::{ZeroTtsContract, normalize_text};

static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Deserialize)]
struct ParityFixture {
    max_frames: usize,
    first_frame_positions: Vec<i64>,
    first_seen_code_counts: Vec<usize>,
    first_frames: Vec<Vec<i32>>,
    eoa_frame_index: usize,
    eoa_frame: Vec<i32>,
}

#[test]
fn checked_in_parity_fixture_requires_a_bounded_eoa_checkpoint() {
    let fixture: ParityFixture =
        serde_json::from_str(include_str!("fixtures/zerotts_core_parity.json"))
            .expect("checked-in parity fixture is valid JSON");
    assert!(fixture.max_frames > fixture.eoa_frame_index);
    assert_eq!(
        fixture.first_frames.len(),
        fixture.first_frame_positions.len()
    );
    assert_eq!(
        fixture.first_frames.len(),
        fixture.first_seen_code_counts.len()
    );
    assert!(
        fixture
            .first_seen_code_counts
            .windows(2)
            .all(|window| window[0] < window[1])
    );
    assert_eq!(fixture.eoa_frame.len(), 16);
}

fn temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("zerotts-core-{nonce}-{sequence}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_contract(root: &std::path::Path, special_eot: u32) -> (PathBuf, PathBuf) {
    let config = root.join("config.json");
    let tokenizer = root.join("tokenizer.json");
    fs::write(&config, r#"{
      "text_format":"bpe", "vocab_size":16, "num_codebooks":16,
      "codebook_size":1024, "d_model":768, "n_heads":12, "n_layers":9,
      "n_voice_queries":10, "sample_rate":48000, "codec_frame_rate":12.5,
      "special_tokens":{"<pad>":0,"<bos>":1,"<eot>":2,"<soa>":3,"<slot>":4,"<eoa>":5,"<en>":6,"<vi>":7}
    }"#).unwrap();
    fs::write(&tokenizer, format!(r#"{{
      "version":"1.0", "truncation":null, "padding":null,
      "added_tokens":[
        {{"id":0,"content":"<pad>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},{{"id":1,"content":"<bos>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":{special_eot},"content":"<eot>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},{{"id":3,"content":"<soa>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":4,"content":"<slot>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},{{"id":5,"content":"<eoa>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":6,"content":"<en>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},{{"id":7,"content":"<vi>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}}
      ],
      "normalizer":null,"pre_tokenizer":{{"type":"Whitespace"}},"post_processor":null,"decoder":null,
      "model":{{"type":"WordLevel","vocab":{{"<pad>":0,"<bos>":1,"<eot>":{special_eot},"<soa>":3,"<slot>":4,"<eoa>":5,"<en>":6,"<vi>":7,"<unk>":8,"Xin":9,"chào":10}},"unk_token":"<unk>"}}
    }}"#)).unwrap();
    (config, tokenizer)
}

#[test]
fn tokenizer_contract_normalizes_then_wraps_pinned_special_tokens() {
    let root = temp_dir();
    let (config, tokenizer) = write_contract(&root, 2);
    let contract = ZeroTtsContract::load(&config, &tokenizer).unwrap();

    assert_eq!(normalize_text("Xin\tcha\u{0300}o\n"), "Xin chào ");
    assert_eq!(contract.encode("Xin   chào").unwrap(), vec![1, 9, 10, 2]);
    assert_eq!(contract.frame_position(0), 11);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tokenizer_contract_rejects_special_token_drift_before_inference() {
    let root = temp_dir();
    let (config, tokenizer) = write_contract(&root, 12);

    let error = match ZeroTtsContract::load(&config, &tokenizer) {
        Ok(_) => panic!("tokenizer drift was accepted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("<eot>"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tokenizer_contract_rejects_wrong_voice_or_graph_dimensions_before_inference() {
    let root = temp_dir();
    let (config, tokenizer) = write_contract(&root, 2);
    let incompatible_config = fs::read_to_string(&config)
        .unwrap()
        .replace("\"n_voice_queries\":10", "\"n_voice_queries\":0");
    fs::write(&config, incompatible_config).unwrap();

    let error = match ZeroTtsContract::load(&config, &tokenizer) {
        Ok(_) => panic!("incompatible voice dimensions were accepted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("pinned ZeroTTS"));
    fs::remove_dir_all(root).unwrap();
}
