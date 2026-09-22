use voice_agent_server::protocol::{
    ClientMessage, ListenCommand, ListenMode, parse_client_message,
};

#[test]
fn listen_start_requires_a_typed_mode() {
    assert_eq!(
        parse_client_message(r#"{"type":"listen","state":"start","mode":"manual"}"#),
        Ok(ClientMessage::listen(ListenCommand::Start {
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
        Ok(ClientMessage::listen(ListenCommand::Stop))
    );
}

#[test]
fn listen_detect_is_parsed_without_being_inferred_as_start() {
    assert_eq!(
        parse_client_message(
            r#"{"session_id":"session","type":"listen","state":"detect","text":"wake word"}"#,
        ),
        Ok(ClientMessage::Listen {
            session_id: Some("session".into()),
            command: ListenCommand::Detect {
                text: "wake word".into(),
            },
        })
    );
}

#[test]
fn session_id_is_optional_but_present_non_string_is_unknown() {
    assert_eq!(
        parse_client_message(r#"{"type":"abort"}"#),
        Ok(ClientMessage::Abort { session_id: None })
    );
    assert_eq!(
        parse_client_message(r#"{"session_id":"","type":"abort"}"#),
        Ok(ClientMessage::Abort {
            session_id: Some(String::new()),
        })
    );
    assert_eq!(
        parse_client_message(r#"{"session_id":42,"type":"abort"}"#),
        Ok(ClientMessage::Unknown)
    );
}
