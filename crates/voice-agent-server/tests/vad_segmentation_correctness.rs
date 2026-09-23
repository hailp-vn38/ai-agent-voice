use std::{sync::Arc, time::Duration};

use voice_agent_server::{
    audio::{PcmF32Mono, VadBoundary, VadSegmenter, VadSegmenterConfig},
    providers::{VadError, VadInput, VadProbability, VadProvider, VadSession},
    workers::{
        VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerRuntime, WorkerIdentity,
        WorkerRuntimeConfig,
    },
};

fn probability(value: f32, start_sample: u64) -> VadProbability {
    VadProbability {
        probability: value,
        start_sample,
        end_sample: start_sample + 512,
    }
}

#[test]
fn segmenter_emits_confirmed_candidate_onset_and_trailing_silence_boundaries() {
    let mut segmenter = VadSegmenter::new(VadSegmenterConfig {
        speech_threshold: 0.5,
        exit_threshold: 0.35,
        min_speech_samples: 1_024,
        end_silence_samples: 1_024,
    });

    assert_eq!(segmenter.observe(probability(0.9, 0)).unwrap(), None);
    assert_eq!(
        segmenter.observe(probability(0.8, 512)).unwrap(),
        Some(VadBoundary::SpeechStart { start_sample: 0 })
    );
    assert_eq!(segmenter.observe(probability(0.43, 1_024)).unwrap(), None);
    assert_eq!(segmenter.observe(probability(0.2, 1_536)).unwrap(), None);
    assert_eq!(
        segmenter.observe(probability(0.1, 2_048)).unwrap(),
        Some(VadBoundary::SpeechEnd { end_sample: 1_536 })
    );
}

#[test]
fn segmenter_rejects_a_gap_before_endpointing() {
    let mut segmenter = VadSegmenter::new(VadSegmenterConfig::default());

    assert_eq!(segmenter.observe(probability(0.9, 0)).unwrap(), None);
    assert!(segmenter.observe(probability(0.9, 1_024)).is_err());
}

#[test]
fn reset_allows_a_fresh_auto_utterance_timeline() {
    let mut segmenter = VadSegmenter::new(VadSegmenterConfig {
        speech_threshold: 0.5,
        exit_threshold: 0.35,
        min_speech_samples: 512,
        end_silence_samples: 512,
    });

    assert!(matches!(
        segmenter.observe(probability(0.9, 0)),
        Ok(Some(VadBoundary::SpeechStart { start_sample: 0 }))
    ));
    segmenter.reset();
    assert!(matches!(
        segmenter.observe(probability(0.9, 0)),
        Ok(Some(VadBoundary::SpeechStart { start_sample: 0 }))
    ));
}

struct InvalidRangeVad;

impl VadProvider for InvalidRangeVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(InvalidRangeSession))
    }

    fn adapter(&self) -> &'static str {
        "invalid-range"
    }
}

struct InvalidRangeSession;

impl VadSession for InvalidRangeSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            probability: 0.9,
            start_sample: input.start_sample + 1,
            end_sample: input.start_sample + 513,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

#[test]
fn worker_fails_when_provider_mutates_the_canonical_sample_range() {
    let runtime = VadWorkerRuntime::new(
        Arc::new(InvalidRangeVad),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let lease = runtime.open(WorkerIdentity::new("session", 1, 1)).unwrap();
    let _ = runtime.recv_timeout(Duration::from_secs(1));
    runtime
        .send(
            lease,
            VadCommand::Push {
                cycle: VadCaptureCycleId::new(1),
                pcm: PcmF32Mono::new(vec![0.0; 960], 16_000),
            },
        )
        .unwrap();

    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::Failed { .. })
    ));
}
