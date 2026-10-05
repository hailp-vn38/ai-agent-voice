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

/// Real installed ZeroTTS pack, resolved from the environment so the gate never guesses paths.
struct InstalledPack {
    root: PathBuf,
}

impl InstalledPack {
    fn resolve() -> Self {
        let root = PathBuf::from(
            std::env::var("ZEROTTS_QUALIFICATION_PACK")
                .expect("ZeroTTS qualification gate unavailable: set ZEROTTS_QUALIFICATION_PACK"),
        );
        assert!(
            root.is_dir(),
            "ZEROTTS_QUALIFICATION_PACK is not a directory"
        );
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        assert!(path.is_file(), "missing installed artifact {relative}");
        path
    }

    fn contract(&self) -> ZeroTtsContract {
        let codec = self.root.join("onnx/codec");
        ZeroTtsContract::load_engine_with_voices(
            &self.path("config.json"),
            &self.path("tokenizer.json"),
            &self.path("voices/index.json"),
            [
                "baotrang",
                "giahuy",
                "hamy",
                "huuduc",
                "kimoanh",
                "maichi",
                "quangminh",
                "tiendat",
            ]
            .into_iter()
            .map(|voice| {
                (
                    voice.to_owned(),
                    self.path(&format!("voices/{voice}/voice.npz")),
                )
            })
            .collect(),
            &self.path("onnx/text_encoder.onnx"),
            &self.path("onnx/prefix_step.onnx"),
            &self.path("onnx/local_frame_decode.onnx"),
            &codec.join("moss_audio_tokenizer_decode_full.onnx"),
            &codec.join("moss_audio_tokenizer_decode_step.onnx"),
            &codec.join("moss_audio_tokenizer_decode_shared.data"),
            &codec.join("codec_browser_onnx_meta.json"),
            &self.path("silence_frame.npy"),
            &PathBuf::from(
                std::env::var("VOICE_ONNX_RUNTIME_LIB")
                    .expect("ZeroTTS qualification gate unavailable: set VOICE_ONNX_RUNTIME_LIB"),
            ),
            1,
        )
        .expect("installed ZeroTTS pack satisfies the pinned contract")
    }
}

fn collect_pcm(
    stream: &mut voice_agent_server::providers::tts::zerotts_onnx::ZeroTtsPcmStream,
    text: &str,
    voice: &ndarray::Array3<f32>,
) -> Vec<f32> {
    let mut samples = Vec::new();
    stream
        .synthesize_with_voice(text, 256, voice, &mut |pcm| {
            samples.extend_from_slice(pcm.samples());
            Ok(())
        })
        .expect("installed ZeroTTS pack synthesizes the gate utterance");
    samples
}

#[test]
#[ignore = "requires an installed ZeroTTS pack and ONNX runtime; set ZEROTTS_QUALIFICATION_PACK and VOICE_ONNX_RUNTIME_LIB"]
fn startup_warmup_is_bounded_and_leaves_no_state_in_the_retained_runtime() {
    use voice_agent_server::providers::tts::zerotts_onnx::{
        ZeroTtsPcmStream, session_constructions,
    };

    let pack = InstalledPack::resolve();
    let contract = pack.contract();
    let maichi = contract.voice("maichi").unwrap();
    let hamy = contract.voice("hamy").unwrap();

    let before = session_constructions();
    let mut warmed = ZeroTtsPcmStream::new(&contract).unwrap();
    assert_eq!(
        session_constructions() - before,
        4,
        "one streaming replica must commit exactly four ONNX sessions"
    );

    let report = warmed.warmup(&maichi).expect("bounded warmup completes");
    assert_eq!(report.autoregressive_frames, 1);
    assert_eq!(report.codec_decodes, 1);
    assert!(report.pcm_samples > 0);
    warmed
        .reset()
        .expect("warmup state is dropped before traffic");
    assert_eq!(
        session_constructions() - before,
        4,
        "the readiness pass must not commit any additional ONNX session"
    );

    let after_warmup = collect_pcm(&mut warmed, "Xin chao ban", &maichi);
    assert!(!after_warmup.is_empty());
    assert!(
        after_warmup.len() > report.pcm_samples,
        "a full utterance must be longer than the bounded readiness pass"
    );
    assert_eq!(
        session_constructions() - before,
        4,
        "a turn must reuse the resident sessions instead of rebuilding the engine"
    );
    drop(warmed);

    // A replica that never warmed must produce byte-identical audio, which proves the readiness
    // pass left no autoregressive KV, repetition history, or codec cache behind.
    let mut reference = ZeroTtsPcmStream::new(&contract).unwrap();
    let cold = collect_pcm(&mut reference, "Xin chao ban", &maichi);
    assert_eq!(
        after_warmup, cold,
        "warmup residue changed the first real turn"
    );

    // A different voice on the same resident runtime binds its own embedding and its own turn.
    let switched = collect_pcm(&mut reference, "Xin chao ban", &hamy);
    assert_ne!(
        switched, cold,
        "a voice switch must not silently reuse the previous voice's turn state"
    );
}
