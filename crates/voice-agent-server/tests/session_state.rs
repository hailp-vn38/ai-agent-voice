use tokio::sync::mpsc;
use voice_agent_server::{
    protocol::{ClientMessage, ListenState},
    session::{SessionActor, SessionPhase},
};

#[tokio::test]
async fn raw_binary_is_only_accepted_while_listening() {
    let (control, _) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let mut actor = SessionActor::new("session".into(), control, audio);
    assert!(!actor.on_binary(vec![1]));
    actor.on_client_message(ClientMessage::Listen(ListenState::Start));
    assert_eq!(actor.phase(), SessionPhase::Listening);
    assert!(actor.on_binary(vec![1]));
    actor.on_client_message(ClientMessage::Listen(ListenState::Stop));
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.accepted_binary_frames(), 1);
}
