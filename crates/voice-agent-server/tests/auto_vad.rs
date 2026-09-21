use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderCapacityLimits,
        ProviderSet, VadError, VadEvent, VadProvider, VadSession,
    },
    session::{SessionActor, SessionPhase},
};

#[derive(Default)]
struct AutoProbe {
    asr_pushes: Mutex<Vec<usize>>,
    resets: Mutex<usize>,
}

struct ScriptedVad {
    probe: Arc<AutoProbe>,
}

impl VadProvider for ScriptedVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(ScriptedVadSession {
            probe: Arc::clone(&self.probe),
            pushes: 0,
        }))
    }

    fn adapter(&self) -> &'static str {
        "scripted"
    }
}

struct ScriptedVadSession {
    probe: Arc<AutoProbe>,
    pushes: usize,
}

impl VadSession for ScriptedVadSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError> {
        self.pushes += 1;
        Ok(match self.pushes {
            2 => vec![VadEvent::SpeechStart],
            4 => vec![VadEvent::SpeechEnd],
            _ => Vec::new(),
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        *self.probe.resets.lock().unwrap() += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

struct RecordingAsr {
    probe: Arc<AutoProbe>,
}

impl AsrProvider for RecordingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(RecordingAsrSession {
            probe: Arc::clone(&self.probe),
        }))
    }
}

struct RecordingAsrSession {
    probe: Arc<AutoProbe>,
}

impl AsrSession for RecordingAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.probe
            .asr_pushes
            .lock()
            .unwrap()
            .push(pcm.samples().len());
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("xin chào"))
    }

    fn cancel(&mut self) {}
}

fn uplink_packet() -> voice_agent_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
}

#[test]
fn auto_opens_asr_at_speech_start_feeds_pre_roll_then_rearms_after_reset() {
    let probe = Arc::new(AutoProbe::default());
    let providers = Arc::new(ProviderSet::with_vad(
        Arc::new(ScriptedVad {
            probe: Arc::clone(&probe),
        }),
        Arc::new(RecordingAsr {
            probe: Arc::clone(&probe),
        }),
    ));
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new("session".into(), control, audio, 8, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    for _ in 0..4 {
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
    }

    assert_eq!(actor.phase(), SessionPhase::Listening);
    assert_eq!(*probe.asr_pushes.lock().unwrap(), vec![960, 960, 960]);
    assert_eq!(*probe.resets.lock().unwrap(), 1);
    let payload: serde_json::Value =
        serde_json::from_str(messages.try_recv().unwrap().as_text().unwrap()).unwrap();
    assert_eq!(payload["type"], "stt");
    assert_eq!(payload["text"], "xin chào");
    assert!(messages.try_recv().is_err());
}

#[test]
fn provider_capacity_rejects_a_second_pinned_asr_stream_until_the_first_is_released() {
    let probe = Arc::new(AutoProbe::default());
    let providers = ProviderSet::with_vad_capacity(
        Arc::new(ScriptedVad {
            probe: Arc::clone(&probe),
        }),
        Arc::new(RecordingAsr { probe }),
        ProviderCapacityLimits {
            max_asr_streams: 1,
            max_vad_sessions: 1,
        },
    );

    let first = providers.open_asr().unwrap();
    assert!(providers.open_asr().is_err());
    drop(first);
    assert!(providers.open_asr().is_ok());
}
