use std::sync::Arc;

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::ProviderSet,
    session::{SessionActor, SessionPhase},
};

#[tokio::test]
async fn raw_binary_is_only_accepted_while_listening() {
    let (control, _) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new(
        "session".into(),
        control,
        audio,
        2,
        Arc::new(ProviderSet::unavailable()),
    )
    .unwrap();
    assert!(!actor.on_binary(vec![1]));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert_eq!(actor.phase(), SessionPhase::Listening);
    let packet = DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap();
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.accepted_binary_frames(), 1);
}

#[tokio::test]
async fn auto_replaces_manual_capture_and_never_falls_back_when_vad_is_unavailable() {
    let (control, mut messages) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new(
        "session".into(),
        control,
        audio,
        2,
        Arc::new(ProviderSet::unavailable()),
    )
    .unwrap();
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    let packet = DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap();
    assert!(actor.on_binary(packet.as_bytes().to_vec()));

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    assert_eq!(actor.phase(), SessionPhase::Listening);
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Closed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Closed);
    assert!(matches!(
        messages.try_recv(),
        Ok(voice_agent_server::session::OutboundMessage::Close(1011))
    ));
    assert!(!actor.on_binary(packet.as_bytes().to_vec()));
}
