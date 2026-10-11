use super::*;

fn installed_contract() -> ZeroTtsContract {
    let root = PathBuf::from(std::env::var_os("ZEROTTS_QUALIFICATION_PACK").unwrap());
    let assets = crate::providers::tts::zerotts::assets::resolve_assets_from(&root).unwrap();
    ZeroTtsContract::load_engine_with_voices(
        &assets.config,
        &assets.tokenizer,
        &assets.voices_index,
        assets.voices,
        &assets.text_encoder,
        &assets.prefix_step,
        &assets.local_frame_decode,
        &assets.codec_decode_full,
        &assets.codec_decode_step,
        &assets.codec_shared_data,
        &assets.codec_metadata,
        &assets.silence_frame,
        &PathBuf::from(std::env::var_os("VOICE_ONNX_RUNTIME_LIB").unwrap()),
        2,
    )
    .unwrap()
}

fn serial_stream(contract: &ZeroTtsContract) -> ZeroTtsPcmStream {
    let graphs = contract.graphs.as_ref().unwrap();
    ZeroTtsPcmStream {
        replica: ZeroTtsReplica {
            contract: contract.clone(),
            text_encoder: load_graph(
                &graphs.text_encoder,
                graphs.threads,
                TEXT_ENCODER_INPUTS,
                TEXT_ENCODER_OUTPUTS,
            )
            .unwrap(),
            prefix_step: load_graph(
                &graphs.prefix_step,
                graphs.threads,
                PREFIX_STEP_INPUTS,
                PREFIX_STEP_OUTPUTS,
            )
            .unwrap(),
            local_frame_decode: load_graph(
                &graphs.local_frame_decode,
                graphs.threads,
                LOCAL_FRAME_INPUTS,
                LOCAL_FRAME_OUTPUTS,
            )
            .unwrap(),
        },
        codec: ZeroTtsCodecReplica::new(contract).unwrap(),
        first_segment: true,
    }
}

fn pcm(stream: &mut ZeroTtsPcmStream, voice: &Array3<f32>) -> Vec<f32> {
    let mut samples = Vec::new();
    stream
        .synthesize_with_voice("Xin chao ban", 256, voice, &mut |chunk| {
            assert_eq!(chunk.sample_rate_hz(), 48_000);
            assert!(!chunk.samples().is_empty());
            assert!(chunk.samples().iter().all(|sample| sample.is_finite()));
            samples.extend_from_slice(chunk.samples());
            Ok(())
        })
        .unwrap();
    assert!(!samples.is_empty());
    samples
}

#[test]
#[ignore = "requires ZEROTTS_QUALIFICATION_PACK and VOICE_ONNX_RUNTIME_LIB"]
fn parallel_initialization_preserves_serial_audio_and_retained_sessions() {
    let contract = installed_contract();
    let before = session_constructions();
    let mut serial = serial_stream(&contract);
    assert_eq!(session_constructions() - before, 4);
    let mut parallel = ZeroTtsPcmStream::new(&contract).unwrap();
    assert_eq!(session_constructions() - before, 8);
    for stream in [&mut serial, &mut parallel] {
        let report = stream.warmup(&contract.voice("maichi").unwrap()).unwrap();
        assert_eq!(report.autoregressive_frames, 1);
        assert_eq!(report.codec_decodes, 1);
        assert!(report.pcm_samples > 0);
        stream.reset().unwrap();
    }
    for voice in ["maichi", "hamy"] {
        let voice = contract.voice(voice).unwrap();
        let reference = pcm(&mut serial, &voice);
        let actual = pcm(&mut parallel, &voice);
        assert_eq!(reference.len(), actual.len());
        assert!(
            reference
                .iter()
                .zip(&actual)
                .all(|(reference, actual)| reference.to_bits() == actual.to_bits()),
            "parallel loading must preserve byte-identical PCM"
        );
        serial.reset().unwrap();
        parallel.reset().unwrap();
        assert_eq!(session_constructions() - before, 8);
    }
}

#[test]
#[ignore = "requires ZEROTTS_QUALIFICATION_PACK and VOICE_ONNX_RUNTIME_LIB"]
fn invalid_local_graph_waits_for_other_sessions_and_never_opens_codec() {
    let mut contract = installed_contract();
    let graphs = contract.graphs.as_mut().unwrap();
    graphs.local_frame_decode = graphs
        .local_frame_decode
        .with_file_name("missing-initialization-graph.onnx");
    assert!(!graphs.local_frame_decode.exists());
    let before = session_constructions();
    assert!(matches!(
        ZeroTtsPcmStream::new(&contract),
        Err(TtsError::IncompatibleContract(_))
    ));
    assert_eq!(session_constructions() - before, 2);
}
