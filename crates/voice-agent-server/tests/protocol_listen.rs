use voice_agent_server::protocol::{
    ClientMessage, ListenCommand, ListenMode, parse_client_message,
};
use voice_agent_server::session::BargeInPolicy;

#[test]
fn barge_in_policy_requires_opt_in_and_a_non_manual_mode() {
    let enabled = BargeInPolicy {
        enabled: true,
        trust_client_aec_feature: true,
    };
    assert!(enabled.allows(true, Some(ListenMode::Auto)));
    assert!(enabled.allows(true, Some(ListenMode::Realtime)));
    assert!(!enabled.allows(false, Some(ListenMode::Auto)));
    assert!(!enabled.allows(true, Some(ListenMode::Manual)));
    assert!(!enabled.allows(true, None));
    assert!(
        !BargeInPolicy {
            enabled: false,
            trust_client_aec_feature: true,
        }
        .allows(true, Some(ListenMode::Auto))
    );
    assert!(
        !BargeInPolicy {
            enabled: true,
            trust_client_aec_feature: false,
        }
        .allows(true, Some(ListenMode::Realtime))
    );
}

#[test]
fn hello_accepts_optional_aec_assertion_and_ignores_unknown_features() {
    let missing =
        parse_client_message(r#"{"type":"hello","features":{"future_capture":true}}"#).unwrap();
    let false_value =
        parse_client_message(r#"{"type":"hello","features":{"aec":false,"future_capture":true}}"#)
            .unwrap();
    let true_value =
        parse_client_message(r#"{"type":"hello","features":{"aec":true,"future_capture":true}}"#)
            .unwrap();

    for (message, expected) in [(missing, false), (false_value, false), (true_value, true)] {
        let ClientMessage::Hello(hello) = message else {
            panic!("expected hello");
        };
        assert_eq!(hello.features.aec, expected);
    }
}

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
