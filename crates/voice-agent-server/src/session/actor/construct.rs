use super::*;

impl SessionActor {
    pub fn new(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        providers: std::sync::Arc<ProviderSet>,
    ) -> Result<Self, crate::audio::AudioError> {
        Self::new_with_history(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            providers,
            20,
        )
    }

    pub fn new_with_history(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        providers: std::sync::Arc<ProviderSet>,
        max_history_messages: usize,
    ) -> Result<Self, crate::audio::AudioError> {
        let asr_runtime = std::sync::Arc::new(AsrWorkerRuntime::new(
            providers.asr_provider(),
            WorkerRuntimeConfig::default(),
        ));
        let vad_runtime = std::sync::Arc::new(VadWorkerRuntime::new(
            providers.vad_provider(),
            WorkerRuntimeConfig::default(),
        ));
        let llm_runtime = std::sync::Arc::new(LlmRuntime::new(
            providers.llm_provider(),
            1,
            std::time::Duration::from_secs(60),
        ));
        Self::new_with_runtimes_and_limiter(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            SessionRuntimes {
                asr: asr_runtime,
                vad: vad_runtime,
                llm: llm_runtime,
                tts: std::sync::Arc::new(TtsWorkerRuntime::new(
                    providers.tts_provider(),
                    WorkerRuntimeConfig::default(),
                )),
                active_turn_limiter: std::sync::Arc::new(ActiveTurnLimiter::new(8)),
                vad_segmenter_config: VadSegmenterConfig::default(),
                pre_roll_samples: 4_800,
            },
        )
        .and_then(|actor| actor.with_delivery_providers(&providers))
    }

    /// Production passes an application-owned runtime so ASR worker capacity is global.
    pub fn new_with_history_and_runtime(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
    ) -> Result<Self, crate::audio::AudioError> {
        // Kept for the Manual-only public seam used by earlier phases.
        let vad_runtime = std::sync::Arc::new(VadWorkerRuntime::new(
            std::sync::Arc::new(crate::providers::vad::UnavailableVad),
            WorkerRuntimeConfig::default(),
        ));
        Self::new_with_runtimes(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            asr_runtime,
            vad_runtime,
        )
    }

    /// Production passes both application-owned runtimes so capacity is global across sessions.
    pub fn new_with_runtimes(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
        vad_runtime: std::sync::Arc<VadWorkerRuntime>,
    ) -> Result<Self, crate::audio::AudioError> {
        Self::new_with_runtimes_and_limiter(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            SessionRuntimes {
                asr: asr_runtime,
                vad: vad_runtime,
                llm: std::sync::Arc::new(LlmRuntime::new(
                    std::sync::Arc::new(UnavailableLlm),
                    1,
                    std::time::Duration::from_secs(60),
                )),
                tts: std::sync::Arc::new(TtsWorkerRuntime::new(
                    std::sync::Arc::new(UnavailableTts),
                    WorkerRuntimeConfig::default(),
                )),
                active_turn_limiter: std::sync::Arc::new(ActiveTurnLimiter::new(8)),
                vad_segmenter_config: VadSegmenterConfig::default(),
                pre_roll_samples: 4_800,
            },
        )
    }

    pub fn new_with_runtimes_and_limiter(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        runtimes: SessionRuntimes,
    ) -> Result<Self, crate::audio::AudioError> {
        Self::new_with_runtimes_and_limiter_and_outbound(
            session_id,
            control_tx.clone(),
            control_tx,
            audio_tx,
            std::sync::Arc::new(GenerationGate::new()),
            watch::channel(false).0,
            max_capture_frames,
            max_history_messages,
            runtimes,
        )
    }

    /// Production supplies independent normal, urgent and audio lanes.  The shorter
    /// constructors intentionally retain one control receiver for older actor-only tests.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_runtimes_and_limiter_and_outbound(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        urgent_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        generation_gate: std::sync::Arc<GenerationGate>,
        shutdown_tx: watch::Sender<bool>,
        max_capture_frames: usize,
        max_history_messages: usize,
        runtimes: SessionRuntimes,
    ) -> Result<Self, crate::audio::AudioError> {
        let retention_capacity = auto_retention_capacity(
            runtimes.vad.runtime_config().command_capacity,
            runtimes.vad_segmenter_config.min_speech_samples,
            runtimes.pre_roll_samples,
        );
        let asr_events = runtimes.asr.register_session(&session_id);
        let vad_events = runtimes.vad.register_session(&session_id);
        let llm_events = runtimes.llm.register_session(&session_id, 64);
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            asr_runtime: runtimes.asr,
            asr_events,
            asr_stream: None,
            asr_cleanup_pending: HashSet::new(),
            vad_runtime: runtimes.vad,
            vad_events,
            vad_session: None,
            vad_cycle: None,
            pending_vad_cycle: None,
            next_vad_cycle: 1,
            listening_mode: None,
            listen_arm_pending: false,
            auto_speech_active: false,
            auto_reset_pending: false,
            vad_segmenter: VadSegmenter::new(runtimes.vad_segmenter_config),
            auto_retention: AutoPcmRetention::new(retention_capacity),
            pre_roll_samples: runtimes.pre_roll_samples,
            active_turn_limiter: runtimes.active_turn_limiter,
            generation: 0,
            next_turn_id: 1,
            next_operation_id: 1,
            turn: None,
            dialogue_history: DialogueHistory::new(max_history_messages),
            llm_runtime: runtimes.llm,
            llm_events,
            llm_operation: None,
            pending_llm_delta: None,
            llm_finish_pending: false,
            generated_response: String::new(),
            tts_runtime: std::sync::Arc::clone(&runtimes.tts),
            speech_output: SpeechOutput::with_worker(
                std::sync::Arc::new(UnavailableTts),
                runtimes.tts,
                crate::config::SpeechOutputConfig::default(),
            )?,
            tts_started: false,
            pending_delivery: None,
            control_tx,
            urgent_tx,
            shutdown_tx,
            audio_tx,
            generation_gate,
            writer_events: None,
            pending_audio: None,
            client_aec_asserted: false,
            barge_in_enabled: false,
            trust_client_aec_feature: false,
        })
    }

    pub fn with_client_capabilities(
        mut self,
        client_aec_asserted: bool,
        policy: super::BargeInPolicy,
    ) -> Self {
        self.client_aec_asserted = client_aec_asserted;
        self.barge_in_enabled = policy.enabled;
        self.trust_client_aec_feature = policy.trust_client_aec_feature;
        self
    }

    pub fn with_writer_events(mut self, writer_events: mpsc::Receiver<WriterEvent>) -> Self {
        self.writer_events = Some(writer_events);
        self
    }

    /// Reports whether the current client/deployment/mode combination is eligible
    /// for the future acoustic barge-in path. Ticket 04 only establishes this
    /// policy; it deliberately does not interrupt an active turn yet.
    pub fn acoustic_barge_in_allowed(&self) -> bool {
        BargeInPolicy {
            enabled: self.barge_in_enabled,
            trust_client_aec_feature: self.trust_client_aec_feature,
        }
        .allows(self.client_aec_asserted, self.listening_mode.clone())
    }

    /// Completes construction with the injected delivery providers at the application seam.
    pub fn with_delivery_providers(
        mut self,
        providers: &ProviderSet,
    ) -> Result<Self, crate::audio::AudioError> {
        let tts_runtime = std::sync::Arc::clone(&self.tts_runtime);
        self.speech_output = SpeechOutput::with_worker(
            providers.tts_provider(),
            tts_runtime,
            crate::config::SpeechOutputConfig::default(),
        )?;
        Ok(self)
    }

    pub fn with_delivery_providers_config(
        mut self,
        providers: &ProviderSet,
        config: crate::config::SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let tts_runtime = std::sync::Arc::clone(&self.tts_runtime);
        self.speech_output =
            SpeechOutput::with_worker(providers.tts_provider(), tts_runtime, config)?;
        Ok(self)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }
    pub fn accepted_binary_frames(&self) -> u64 {
        self.accepted_binary_frames
    }
    pub fn dialogue_history(&self) -> &[String] {
        self.dialogue_history.messages()
    }
}
