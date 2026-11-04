use super::harness::*;

#[tokio::test]
async fn streaming_llm_delivers_first_audio_before_eof_and_commits_only_after_drained() {
    let (control_tx, mut control_rx) = mpsc::channel(16);
    let (audio_tx, mut audio_rx) = mpsc::channel(16);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(StreamingLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    for _ in 0..80 {
        actor.pump_workers();
        if audio_rx.try_recv().is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(
        actor.phase(),
        SessionPhase::Speaking,
        "first playable audio enters Speaking while LLM still streams"
    );
    assert_eq!(
        actor.dialogue_history(),
        ["xin chao"],
        "assistant is not delivered yet"
    );
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok()).collect::<Vec<_>>();
    assert!(controls.iter().any(|message| {
        message
            .as_text()
            .is_some_and(|text| text.contains(r#""state":"start""#))
    }));
    assert!(!controls.iter().any(|message| {
        message
            .as_text()
            .is_some_and(|text| text.contains(r#""state":"stop""#))
    }));

    for _ in 0..300 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(
        actor.dialogue_history(),
        ["xin chao", "Cau dau tien. Cau thu hai."]
    );
}

#[tokio::test]
async fn complete_vietnamese_sentences_announce_once_before_their_audio() {
    let (control_tx, mut control_rx) = mpsc::channel(16);
    let (audio_tx, mut audio_rx) = mpsc::channel(16);
    let release_second = Arc::new(Notify::new());
    let tts_inputs = Arc::new(Mutex::new(Vec::new()));
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(ControlledSentenceLlm {
            release_second: Arc::clone(&release_second),
        }),
        Arc::new(RecordingTts(Arc::clone(&tts_inputs))),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    for _ in 0..200 {
        actor.pump_workers();
        if !audio_rx.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        !audio_rx.is_empty(),
        "first sentence must produce audio before LLM EOF"
    );
    assert_eq!(actor.phase(), SessionPhase::Speaking);
    let first_controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .map(|message| {
            serde_json::from_str::<serde_json::Value>(message.as_text().unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(first_controls.len(), 3);
    assert_eq!(first_controls[0]["type"], "stt");
    assert_eq!(first_controls[1]["type"], "llm");
    assert_eq!(first_controls[1]["text"], "Xin chào!");
    assert_eq!(first_controls[2]["state"], "start");
    assert_eq!(*tts_inputs.lock().unwrap(), ["Xin chào."]);
    assert!(matches!(
        audio_rx.try_recv(),
        Ok(OutboundMessage::Binary { .. })
    ));

    release_second.notify_one();
    for _ in 0..300 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    let later_controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .map(|message| {
            serde_json::from_str::<serde_json::Value>(message.as_text().unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(later_controls.len(), 2);
    assert_eq!(later_controls[0]["type"], "llm");
    assert_eq!(
        later_controls[0]["text"],
        "Mình có thể giúp gì cho bạn hôm nay?"
    );
    assert_eq!(later_controls[1]["state"], "stop");
    assert_eq!(
        *tts_inputs.lock().unwrap(),
        ["Xin chào.", "Mình có thể giúp gì cho bạn hôm nay."]
    );
    assert!(matches!(
        audio_rx.try_recv(),
        Ok(OutboundMessage::Binary { .. })
    ));
    assert!(audio_rx.try_recv().is_err());
}

#[tokio::test]
async fn full_segment_capacity_pauses_llm_until_all_segments_are_spoken() {
    let (control_tx, mut control_rx) = mpsc::channel(32);
    let (audio_tx, _audio_rx) = mpsc::channel(32);
    let release_overflow = Arc::new(Notify::new());
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(BackpressuredLlm {
            release_overflow: Arc::clone(&release_overflow),
        }),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    let mut controls_before_overflow = Vec::new();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    for _ in 0..100 {
        actor.pump_workers();
        while let Ok(message) = control_rx.try_recv() {
            let started = message
                .as_text()
                .is_some_and(|text| text.contains(r#""state":"start""#));
            controls_before_overflow.push(message);
            if started {
                release_overflow.notify_one();
                break;
            }
        }
        if !controls_before_overflow.iter().any(|message| {
            message
                .as_text()
                .is_some_and(|text| text.contains(r#""state":"start""#))
        }) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        } else {
            break;
        }
    }
    for _ in 0..2_000 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(
        actor.dialogue_history(),
        [
            "xin chao".to_owned(),
            format!(
                "{}{}{}",
                "Mot cau dau tien du de tach thanh speech segment ngay. ",
                "Mot cau thu hai du de tach thanh speech segment ngay. ",
                "Mot cau dai du de tach thanh speech segment ngay. ".repeat(10)
            )
        ]
    );
    let controls = controls_before_overflow
        .into_iter()
        .chain(std::iter::from_fn(|| control_rx.try_recv().ok()))
        .filter_map(|message| message.as_text().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""state":"start""#))
            .count(),
        1
    );
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""state":"stop""#))
            .count(),
        1
    );
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""type":"llm""#))
            .count(),
        12
    );
}

#[tokio::test]
async fn auto_greeting_then_rust_response_drains_on_the_same_session() {
    let (control_tx, mut control_rx) = mpsc::channel(64);
    let (audio_tx, _audio_rx) = mpsc::channel(64);
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(GreetingThenRustLlm(Arc::clone(&calls))),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));

    let mut controls = Vec::new();
    for (prompt, expected_turns) in [("xin chào", 1), ("rust là gì", 2)] {
        actor.on_client_message(ClientMessage::listen(ListenCommand::Detect {
            text: prompt.to_owned(),
        }));
        for _ in 0..2_000 {
            actor.pump_workers();
            controls.extend(std::iter::from_fn(|| control_rx.try_recv().ok()));
            if actor.phase() == SessionPhase::Listening
                && controls
                    .iter()
                    .filter(|message| {
                        message
                            .as_text()
                            .is_some_and(|text| text.contains(r#""state":"stop""#))
                    })
                    .count()
                    == expected_turns
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(actor.phase(), SessionPhase::Listening);
        assert_eq!(
            controls
                .iter()
                .filter(|message| message
                    .as_text()
                    .is_some_and(|text| text.contains(r#""state":"stop""#)))
                .count(),
            expected_turns,
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(actor.dialogue_history().len(), 4);
    assert!(actor.dialogue_history()[3].contains("Rust còn hỗ trợ lập trình đồng thời an toàn."));
    assert_eq!(
        controls
            .iter()
            .filter(|message| message
                .as_text()
                .is_some_and(|text| text.contains(r#""type":"llm""#)))
            .count(),
        13,
    );
}

#[tokio::test]
async fn wrapped_long_llm_text_waits_for_speech_segment_capacity() {
    let (control_tx, mut control_rx) = mpsc::channel(64);
    let (audio_tx, _audio_rx) = mpsc::channel(64);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(WrappedLongLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Detect {
        text: "rust là gì".into(),
    }));
    for _ in 0..2_000 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Listening {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .filter_map(|message| message.as_text().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""state":"stop""#))
            .count(),
        1
    );
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""type":"llm""#))
            .count(),
        9
    );
    assert_eq!(actor.dialogue_history().len(), 2);
    assert_eq!(
        actor.dialogue_history()[1],
        format!("[{}]", "Rust là ngôn ngữ lập trình hệ thống. ".repeat(9))
    );
}

#[tokio::test]
async fn llm_failure_before_audio_emits_no_playback_controls_or_assistant_history() {
    let (control_tx, mut control_rx) = mpsc::channel(16);
    let (audio_tx, mut audio_rx) = mpsc::channel(16);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FailingLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.dialogue_history(), ["xin chao"]);
    assert!(audio_rx.try_recv().is_err());
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .filter_map(|message| message.as_text().map(str::to_owned))
        .collect::<Vec<_>>();
    assert!(!controls.iter().any(|text| text.contains(r#""type":"tts""#)));
}
