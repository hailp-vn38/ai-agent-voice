use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

pub use tokio::sync::mpsc;
pub use vela_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono, VadSegmenterConfig},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderSet, VadProvider,
        llm::UnavailableLlm, tts::UnavailableTts,
    },
    session::{ActiveTurnLimiter, SessionActor, SessionPhase, SessionRuntimes},
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
    },
};

pub struct FakeAsr {
    pub final_text: String,
}

pub fn unavailable_tts_runtime() -> Arc<TtsWorkerRuntime> {
    Arc::new(TtsWorkerRuntime::new(
        Arc::new(UnavailableTts),
        WorkerRuntimeConfig::default(),
    ))
}

pub struct FakeVad;

impl VadProvider for FakeVad {
    fn open(
        &self,
    ) -> Result<Box<dyn vela_server::providers::VadSession>, vela_server::providers::VadError> {
        Err(vela_server::providers::VadError::Failed(
            "not used by Manual".into(),
        ))
    }

    fn adapter(&self) -> &'static str {
        "fake_vad"
    }
}

pub fn wait_for_worker(actor: &mut SessionActor) {
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("ASR worker did not reach terminal state");
}

impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeSession {
            final_text: self.final_text.clone(),
        }))
    }
}

pub struct FakeSession {
    pub final_text: String,
}

impl AsrSession for FakeSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![AsrEvent::Partial("xin".into())])
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new(&self.final_text))
    }

    fn cancel(&mut self) {}
}

pub struct FailingAsr;

impl AsrProvider for FailingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FailingSession))
    }
}

pub struct FailingSession;

impl AsrSession for FailingSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Err(AsrError::Failed("deterministic test failure".into()))
    }

    fn cancel(&mut self) {}
}

pub struct SlowAsr;

impl AsrProvider for SlowAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(SlowSession))
    }
}

pub struct SlowSession;

impl AsrSession for SlowSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        std::thread::sleep(Duration::from_millis(50));
        Ok(AsrResult::new("late"))
    }

    fn cancel(&mut self) {}
}

pub fn uplink_packet() -> vela_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
}

pub struct CountingAsr {
    pub pushed_frames: Arc<AtomicUsize>,
}

impl AsrProvider for CountingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(CountingSession {
            pushed_frames: Arc::clone(&self.pushed_frames),
        }))
    }
}

pub struct CountingSession {
    pub pushed_frames: Arc<AtomicUsize>,
}

impl AsrSession for CountingSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.pushed_frames.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("ignored"))
    }

    fn cancel(&mut self) {}
}

pub struct BoundaryVad;

impl VadProvider for BoundaryVad {
    fn open(
        &self,
    ) -> Result<Box<dyn vela_server::providers::VadSession>, vela_server::providers::VadError> {
        Ok(Box::new(BoundaryVadSession { frames: 0 }))
    }

    fn adapter(&self) -> &'static str {
        "boundary"
    }
}

pub struct BoundaryVadSession {
    pub frames: usize,
}

impl vela_server::providers::VadSession for BoundaryVadSession {
    fn push(
        &mut self,
        input: vela_server::providers::VadInput,
    ) -> Result<vela_server::providers::VadProbability, vela_server::providers::VadError> {
        self.frames += 1;
        Ok(vela_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: if self.frames == 1 { 1.0 } else { 0.0 },
        })
    }

    fn reset(&mut self) -> Result<(), vela_server::providers::VadError> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), vela_server::providers::VadError> {
        Ok(())
    }
}

pub struct DelayedOnsetVad;

impl VadProvider for DelayedOnsetVad {
    fn open(
        &self,
    ) -> Result<Box<dyn vela_server::providers::VadSession>, vela_server::providers::VadError> {
        Ok(Box::new(DelayedOnsetVadSession { frames: 0 }))
    }

    fn adapter(&self) -> &'static str {
        "delayed-onset"
    }
}

pub struct DelayedOnsetVadSession {
    pub frames: usize,
}

impl vela_server::providers::VadSession for DelayedOnsetVadSession {
    fn push(
        &mut self,
        input: vela_server::providers::VadInput,
    ) -> Result<vela_server::providers::VadProbability, vela_server::providers::VadError> {
        self.frames += 1;
        Ok(vela_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: (self.frames >= 2) as u8 as f32,
        })
    }

    fn reset(&mut self) -> Result<(), vela_server::providers::VadError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), vela_server::providers::VadError> {
        Ok(())
    }
}

pub struct RecordingAsr {
    pub pushed_samples: Arc<AtomicUsize>,
}

impl AsrProvider for RecordingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(RecordingAsrSession {
            pushed_samples: Arc::clone(&self.pushed_samples),
        }))
    }
}

pub struct RecordingAsrSession {
    pub pushed_samples: Arc<AtomicUsize>,
}

impl AsrSession for RecordingAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.pushed_samples
            .fetch_add(pcm.samples().len(), Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("unused"))
    }

    fn cancel(&mut self) {}
}

pub struct CountingVad {
    pub opens: Arc<AtomicUsize>,
    pub resets: Arc<AtomicUsize>,
    pub closes: Arc<AtomicUsize>,
}

impl VadProvider for CountingVad {
    fn open(
        &self,
    ) -> Result<Box<dyn vela_server::providers::VadSession>, vela_server::providers::VadError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(CountingVadSession {
            resets: Arc::clone(&self.resets),
            closes: Arc::clone(&self.closes),
        }))
    }

    fn adapter(&self) -> &'static str {
        "counting_vad"
    }
}

pub struct CountingVadSession {
    pub resets: Arc<AtomicUsize>,
    pub closes: Arc<AtomicUsize>,
}

impl vela_server::providers::VadSession for CountingVadSession {
    fn push(
        &mut self,
        input: vela_server::providers::VadInput,
    ) -> Result<vela_server::providers::VadProbability, vela_server::providers::VadError> {
        Ok(vela_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + input.pcm.len() as u64,
            probability: 0.0,
        })
    }

    fn reset(&mut self) -> Result<(), vela_server::providers::VadError> {
        self.resets.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn close(&mut self) -> Result<(), vela_server::providers::VadError> {
        self.closes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

pub fn wait_for_phase(actor: &mut SessionActor, phase: SessionPhase) {
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == phase {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("actor did not reach {phase:?}");
}

pub fn auto_actor_with_counting_vad(vad: Arc<VadWorkerRuntime>) -> SessionActor {
    let (control, _) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    SessionActor::new_with_runtimes_and_limiter(
        "auto-counting".into(),
        control,
        audio,
        4,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: Arc::new(AsrWorkerRuntime::new(
                Arc::new(FakeAsr {
                    final_text: "unused".into(),
                }),
                WorkerRuntimeConfig::default(),
            )),
            vad,
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: VadSegmenterConfig::default(),
            pre_roll_samples: 4_800,
        },
    )
    .unwrap()
}

pub fn auto_actor_for_duplicate_start(
    asr: Arc<dyn AsrProvider>,
    vad: Arc<dyn VadProvider>,
) -> SessionActor {
    let (control, _) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let config = WorkerRuntimeConfig {
        max_workers: 1,
        voice_reserved_capacity: 1,
        command_capacity: 8,
        final_timeout: Duration::from_secs(1),
        cleanup_grace: Duration::from_secs(1),
    };
    SessionActor::new_with_runtimes_and_limiter(
        "duplicate-auto".into(),
        control,
        audio,
        4,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: Arc::new(AsrWorkerRuntime::new(asr, config.clone())),
            vad: Arc::new(VadWorkerRuntime::new(vad, config)),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: VadSegmenterConfig {
                speech_threshold: 0.5,
                exit_threshold: 0.35,
                min_speech_samples: 512,
                end_silence_samples: 512,
            },
            pre_roll_samples: 0,
        },
    )
    .unwrap()
}
