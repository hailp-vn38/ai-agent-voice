use super::{
    AsrFeedMode, TtsBenchmarkMode, run_asr_provider, run_llm_provider, run_tts_benchmark,
    run_vad_provider,
};
use crate::{
    audio::PcmF32Mono,
    providers::llm::{LlmEventStream, LlmRequest, ToolCall},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmEvent, LlmProvider, TtsError,
        TtsSynthesisRequest, TtsWorker, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
    },
};
use std::sync::atomic::AtomicBool;

struct Worker {
    syntheses: usize,
    resets: usize,
}
impl TtsWorker for Worker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &AtomicBool,
        callback: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.syntheses += 1;
        callback(PcmF32Mono::new(vec![0.25; 2880], 48000))
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        self.resets += 1;
        Ok(())
    }
}

#[test]
fn delivery_excludes_warmup_and_reports_canonical_packets() {
    let mut worker = Worker {
        syntheses: 0,
        resets: 0,
    };
    let report = run_tts_benchmark(
        &mut worker,
        &crate::providers::TtsBinding::readiness(),
        TtsBenchmarkMode::Delivery,
        1,
        2,
    )
    .unwrap();
    assert_eq!(worker.syntheses, 3);
    assert_eq!(worker.resets, 3);
    assert_eq!(report.samples.len(), 2);
    assert!(
        report
            .samples
            .iter()
            .all(|s| s.opus_bytes.is_some_and(|bytes| bytes > 0))
    );
    assert!(report.delivery_total_ms.is_some());
}

#[test]
fn invalid_pcm_is_rejected() {
    struct Invalid;
    impl TtsWorker for Invalid {
        fn synthesize(
            &mut self,
            _: &TtsSynthesisRequest,
            _: &AtomicBool,
            callback: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            callback(PcmF32Mono::new(vec![f32::NAN], 48000))
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            Ok(())
        }
    }
    let mut worker = Invalid;
    assert!(
        run_tts_benchmark(
            &mut worker,
            &crate::providers::TtsBinding::readiness(),
            TtsBenchmarkMode::Provider,
            0,
            1,
        )
        .is_err()
    );
}

struct Asr;
struct AsrSessionFake;
impl AsrProvider for Asr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(AsrSessionFake))
    }
}
impl AsrSession for AsrSessionFake {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        assert_eq!(pcm.samples().len(), 960);
        Ok(vec![AsrEvent::Partial("partial".into())])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("final"))
    }
    fn cancel(&mut self) {}
}

#[test]
fn asr_runner_uses_canonical_frames_and_rejects_a_partial_frame() {
    let report = run_asr_provider(&Asr, &vec![0.0; 1_920], AsrFeedMode::Burst).unwrap();
    assert_eq!(report.partial_count, 2);
    assert_eq!(report.final_text_chars, 5);
    assert!(run_asr_provider(&Asr, &vec![0.0; 961], AsrFeedMode::Burst).is_err());
}

struct Vad;
struct VadSessionFake;
impl VadProvider for Vad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(VadSessionFake))
    }
    fn adapter(&self) -> &'static str {
        "fake"
    }
}
impl VadSession for VadSessionFake {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        assert_eq!(input.pcm.len(), 512);
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.5,
        })
    }
    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

#[test]
fn vad_runner_reports_every_canonical_frame_and_rejects_a_partial_frame() {
    let report = run_vad_provider(&Vad, &vec![0.0; 1_024]).unwrap();
    assert_eq!(report.frame_latencies_us.len(), 2);
    assert!(report.frames_per_second > 0.0);
    assert!(run_vad_provider(&Vad, &vec![0.0; 513]).is_err());
}

#[test]
fn vad_runner_rejects_a_provider_that_mutates_the_input_timeline() {
    struct InvalidTimeline;
    struct InvalidTimelineSession;
    impl VadProvider for InvalidTimeline {
        fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
            Ok(Box::new(InvalidTimelineSession))
        }
        fn adapter(&self) -> &'static str {
            "invalid-timeline"
        }
    }
    impl VadSession for InvalidTimelineSession {
        fn push(&mut self, _: VadInput) -> Result<VadProbability, VadError> {
            Ok(VadProbability {
                start_sample: 1,
                end_sample: 513,
                probability: 0.5,
            })
        }
        fn reset(&mut self) -> Result<(), VadError> {
            Ok(())
        }
    }
    assert!(run_vad_provider(&InvalidTimeline, &vec![0.0; 512]).is_err());
}

struct Llm;
#[async_trait::async_trait]
impl LlmProvider for Llm {
    fn adapter(&self) -> &'static str {
        "fake"
    }
    async fn stream(&self, _: LlmRequest) -> Result<LlmEventStream, crate::providers::LlmError> {
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta("   ".into())),
            Ok(LlmEvent::ToolCall(ToolCall {
                id: "call-1".into(),
                name: "noop".into(),
                arguments: serde_json::json!({}),
            })),
            Ok(LlmEvent::TextDelta("xin chào".into())),
            Ok(LlmEvent::Finished),
        ])))
    }
}

#[tokio::test]
async fn llm_runner_uses_first_non_empty_text_delta_for_ttft() {
    let report = run_llm_provider(&Llm, LlmRequest::from("hello"))
        .await
        .unwrap();
    assert!(report.ttft_ms.is_some());
    assert_eq!(report.text_delta_count, 2);
    assert_eq!(report.output_chars, 11);
    assert_eq!(report.tool_call_count, 1);
}

#[test]
fn kokoro_24khz_delivery_measures_real_duration_and_canonical_packets() {
    struct KokoroWorker;
    impl TtsWorker for KokoroWorker {
        fn synthesize(
            &mut self,
            _: &TtsSynthesisRequest,
            _: &AtomicBool,
            callback: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            callback(PcmF32Mono::new(vec![0.25; 1440], 24000))
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            Ok(())
        }
    }
    let report = run_tts_benchmark(
        &mut KokoroWorker,
        &crate::providers::TtsBinding::readiness(),
        TtsBenchmarkMode::Delivery,
        1,
        2,
    )
    .unwrap();
    assert!(
        report
            .samples
            .iter()
            .all(|sample| sample.provider_audio_duration_ms == 60.0)
    );
    assert!(
        report
            .samples
            .iter()
            .all(|sample| sample.packet_count.is_some_and(|count| count > 0))
    );
}
