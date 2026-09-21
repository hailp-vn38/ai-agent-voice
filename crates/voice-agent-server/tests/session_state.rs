use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    session::{SessionActor, SessionPhase},
};

#[tokio::test]
async fn raw_binary_is_only_accepted_while_listening() {
    let (control, _) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new("session".into(), control, audio, 2).unwrap();
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
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.accepted_binary_frames(), 1);
}

#[tokio::test]
async fn unsupported_listen_mode_does_not_reset_manual_capture() {
    let (control, _) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new("session".into(), control, audio, 2).unwrap();
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
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert_eq!(actor.accepted_binary_frames(), 2);
}
