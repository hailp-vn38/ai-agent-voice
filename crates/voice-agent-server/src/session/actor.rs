use crate::session::SessionPhase;
use crate::{
    audio::{CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::ProviderSet,
};
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
    accepted_binary_frames: u64,
    uplink_decoder: UplinkOpusDecoder,
    manual_capture: ManualCapture,
    asr_stream: Option<Box<dyn crate::providers::AsrSession>>,
    vad_session: Option<Box<dyn crate::providers::VadSession>>,
    listening_mode: Option<ListenMode>,
    pre_roll: VecDeque<PcmF32Mono>,
    dialogue_history: DialogueHistory,
    providers: std::sync::Arc<ProviderSet>,
    control_tx: mpsc::Sender<OutboundMessage>,
    _audio_tx: mpsc::Sender<OutboundMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundMessage {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Binary(_) | Self::Close(_) => None,
        }
    }
}

#[derive(Debug)]
struct DialogueHistory {
    messages: Vec<String>,
    max_messages: usize,
}

impl DialogueHistory {
    fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
        }
    }

    fn commit_user(&mut self, text: String) {
        if self.messages.len() == self.max_messages {
            self.messages.remove(0);
        }
        self.messages.push(text);
    }
}

/// Reader tasks only enqueue these events; they never mutate Voice Session state.
#[derive(Debug)]
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(Vec<u8>),
}

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
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            asr_stream: None,
            vad_session: None,
            listening_mode: None,
            pre_roll: VecDeque::with_capacity(5),
            dialogue_history: DialogueHistory::new(max_history_messages),
            providers,
            control_tx,
            _audio_tx: audio_tx,
        })
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
        &self.dialogue_history.messages
    }

    pub fn send_control(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self.control_tx.try_send(OutboundMessage::Text(text))
    }

    pub async fn run(mut self, mut ingress: mpsc::Receiver<SessionEvent>) {
        while let Some(event) = ingress.recv().await {
            match event {
                SessionEvent::ClientMessage(message) => self.on_client_message(message),
                SessionEvent::ClientAudio(payload) => {
                    self.on_binary(payload);
                }
            }
        }
        self.phase = SessionPhase::Closed;
    }

    pub fn on_client_message(&mut self, message: ClientMessage) {
        match message {
            ClientMessage::Listen(ListenCommand::Start {
                mode: ListenMode::Manual,
            }) => {
                self.cancel_capture();
                self.manual_capture.restart();
                match self.providers.open_asr() {
                    Ok(stream) => {
                        self.asr_stream = Some(stream);
                        self.listening_mode = Some(ListenMode::Manual);
                        self.phase = SessionPhase::Listening;
                    }
                    Err(_) => self.phase = SessionPhase::Ready,
                }
            }
            ClientMessage::Listen(ListenCommand::Start {
                mode: ListenMode::Auto,
            }) => {
                if let Ok(vad) = self.providers.open_vad() {
                    self.cancel_capture();
                    self.vad_session = Some(vad);
                    self.listening_mode = Some(ListenMode::Auto);
                    self.pre_roll.clear();
                    self.phase = SessionPhase::Listening;
                }
            }
            ClientMessage::Listen(ListenCommand::Start { .. })
            | ClientMessage::Listen(ListenCommand::Detect { .. }) => {}
            ClientMessage::Listen(ListenCommand::Stop) => {
                if self.phase == SessionPhase::Listening
                    && self.listening_mode == Some(ListenMode::Manual)
                {
                    let outcome = self.manual_capture.stop();
                    if let CaptureOutcome::Utterance(_) = outcome {
                        self.phase = SessionPhase::Processing;
                        self.finish_manual();
                    } else {
                        self.cancel_asr();
                    }
                    self.listening_mode = None;
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Abort => {
                if self.phase == SessionPhase::Listening {
                    self.cancel_capture();
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    fn finish_manual(&mut self) {
        let Some(mut stream) = self.asr_stream.take() else {
            return;
        };
        let Ok(final_result) = stream.finish() else {
            return;
        };
        let text = final_result.text().trim();
        if text.is_empty() {
            return;
        }
        let text = text.to_owned();
        self.dialogue_history.commit_user(text.clone());
        let payload = serde_json::json!({
            "session_id": self.session_id,
            "type": "stt",
            "text": text,
        });
        let Ok(payload) = serde_json::to_string(&payload) else {
            return;
        };
        let _ = self.send_control(payload);
    }

    fn cancel_asr(&mut self) {
        if let Some(mut stream) = self.asr_stream.take() {
            stream.cancel();
        }
    }

    fn cancel_capture(&mut self) {
        self.manual_capture.abort();
        self.cancel_asr();
        if let Some(mut vad) = self.vad_session.take() {
            let _ = vad.close();
        }
        self.listening_mode = None;
        self.pre_roll.clear();
    }

    fn push_pre_roll(&mut self, pcm: PcmF32Mono) {
        if self.pre_roll.len() == 5 {
            self.pre_roll.pop_front();
        }
        self.pre_roll.push_back(pcm);
    }

    fn start_auto_asr(&mut self) -> bool {
        let Ok(mut stream) = self.providers.open_asr() else {
            return false;
        };
        for frame in &self.pre_roll {
            if stream.push_pcm(frame).is_err() {
                stream.cancel();
                return false;
            }
        }
        self.asr_stream = Some(stream);
        true
    }

    fn finish_auto(&mut self) {
        self.phase = SessionPhase::Processing;
        self.finish_manual();
        let reset_ok = self
            .vad_session
            .as_mut()
            .is_some_and(|vad| vad.reset().is_ok());
        if reset_ok {
            self.pre_roll.clear();
            self.phase = SessionPhase::Listening;
        } else {
            self.cancel_capture();
            let _ = self.control_tx.try_send(OutboundMessage::Close(1011));
            self.phase = SessionPhase::Closed;
        }
    }

    pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    let pcm = PcmF32Mono::from_uplink(&frame);
                    if self.listening_mode == Some(ListenMode::Auto) {
                        self.push_pre_roll(pcm.clone());
                        let events = match self.vad_session.as_mut() {
                            Some(vad) => vad.push_pcm(&pcm),
                            None => Err(crate::providers::VadError::Failed(
                                "VAD session missing".into(),
                            )),
                        };
                        let Ok(events) = events else {
                            self.cancel_capture();
                            let _ = self.control_tx.try_send(OutboundMessage::Close(1011));
                            self.phase = SessionPhase::Closed;
                            return false;
                        };
                        let mut started_this_frame = false;
                        for event in events {
                            match event {
                                crate::providers::VadEvent::SpeechStart => {
                                    if self.asr_stream.is_none() && !self.start_auto_asr() {
                                        self.phase = SessionPhase::Processing;
                                    } else {
                                        started_this_frame = true;
                                    }
                                }
                                crate::providers::VadEvent::SpeechEnd => {
                                    if self.asr_stream.is_some() {
                                        self.finish_auto();
                                    }
                                }
                            }
                        }
                        if !started_this_frame
                            && self.asr_stream.is_some()
                            && self.phase == SessionPhase::Listening
                        {
                            if let Some(stream) = self.asr_stream.as_mut() {
                                if stream.push_pcm(&pcm).is_err() {
                                    self.cancel_asr();
                                    self.phase = SessionPhase::Processing;
                                }
                            }
                        }
                        self.accepted_binary_frames += 1;
                        return self.phase != SessionPhase::Closed;
                    }
                    if !self.manual_capture.push(frame.clone()) {
                        self.cancel_asr();
                        return false;
                    }
                    let Some(stream) = self.asr_stream.as_mut() else {
                        self.phase = SessionPhase::Ready;
                        return false;
                    };
                    if stream.push_pcm(&pcm).is_err() {
                        self.cancel_asr();
                        self.manual_capture.abort();
                        self.phase = SessionPhase::Ready;
                        return false;
                    }
                    self.accepted_binary_frames += 1;
                    true
                }
                DecodeOutcome::Dropped(reason) => {
                    tracing::debug!(
                        event = "audio_frame_dropped",
                        session_id = %self.session_id,
                        ?reason,
                        "audio frame dropped"
                    );
                    false
                }
            }
        } else {
            false
        }
    }
}
