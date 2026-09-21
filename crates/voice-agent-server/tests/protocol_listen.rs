use voice_agent_server::protocol::{
    parse_client_message, ClientMessage, ListenCommand, ListenMode,
};

#[test]
fn listen_start_requires_a_typed_mode() {
    assert_eq!(
        parse_client_message(r#"{"type":"listen","state":"start","mode":"manual"}"#),
        Ok(ClientMessage::Listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }))
    );
    assert_eq!(
        parse_client_message(r#"{"type":"listen","state":"start"}"#),
        Ok(ClientMessage::Unknown)
    );
}

#[test]
fn listen_stop_does_not_require_a_mode() {
    assert_eq!(
        parse_client_message(r#"{"type":"listen","state":"stop"}"#),
        Ok(ClientMessage::Listen(ListenCommand::Stop))
    );
}

#[test]
fn listen_detect_is_parsed_without_being_inferred_as_start() {
    assert_eq!(
        parse_client_message(r#"{"type":"listen","state":"detect","text":"wake word"}"#),
        Ok(ClientMessage::Listen(ListenCommand::Detect {
            text: "wake word".into(),
        }))
    );
}
