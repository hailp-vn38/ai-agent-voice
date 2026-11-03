use super::harness::*;

#[tokio::test]
async fn router_writes_tts_start_then_canonical_audio_then_one_stop() {
    let (base, task) = start_router().await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Binary(uplink_packet().as_bytes().to_vec().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut observed = Vec::new();
    while observed.len() < 5 {
        observed.push(
            timeout(Duration::from_secs(1), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    task.abort();

    assert!(
        matches!(&observed[0], Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).unwrap()["type"] == "stt")
    );
    assert!(matches!(&observed[1], Message::Text(text) if {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        value["type"] == "llm" && value["text"] == "Xin chao ban."
    }));
    assert!(matches!(&observed[2], Message::Text(text) if {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(value.get("source").is_none());
        assert!(value.get("request_id").is_none());
        assert!(value.get("attempt_id").is_none());
        value["type"] == "tts" && value["state"] == "start"
    }));
    let Message::Binary(packet) = &observed[3] else {
        panic!("expected paced audio")
    };
    let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
    let mut pcm = [0_i16; 1_440];
    assert_eq!(decoder.decode(packet, &mut pcm, false).unwrap(), 1_440);
    assert!(matches!(&observed[4], Message::Text(text) if {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        value["type"] == "tts" && value["state"] == "stop"
    }));
}

#[tokio::test]
async fn public_websocket_sends_each_complete_llm_sentence_once() {
    let release_second = Arc::new(Notify::new());
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(ControlledSentenceLlm {
            release_second: Arc::clone(&release_second),
        }),
        Arc::new(FakeTts),
    ));
    let (base, task) = start_router_with(providers).await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Binary(uplink_packet().as_bytes().to_vec().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut llm_texts = Vec::new();
    let mut saw_start = false;
    loop {
        let frame = timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "llm" {
                    llm_texts.push(value["text"].as_str().unwrap().to_owned());
                } else if value["type"] == "tts" && value["state"] == "start" {
                    assert_eq!(llm_texts, ["Xin chào!"]);
                    saw_start = true;
                }
            }
            Message::Binary(_) => {
                assert!(saw_start);
                assert_eq!(llm_texts, ["Xin chào!"]);
                break;
            }
            other => panic!("unexpected websocket frame: {other:?}"),
        }
    }

    release_second.notify_one();
    loop {
        let frame = timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "llm" {
                llm_texts.push(value["text"].as_str().unwrap().to_owned());
            } else if value["type"] == "tts" && value["state"] == "stop" {
                break;
            }
        }
    }
    assert_eq!(
        llm_texts,
        ["Xin chào!", "Mình có thể giúp gì cho bạn hôm nay?"]
    );
    assert!(
        timeout(Duration::from_millis(50), socket.next())
            .await
            .is_err()
    );
    task.abort();
}

#[tokio::test]
async fn auto_listening_accepts_digital_human_detect_as_a_text_turn() {
    let (base, task) = start_router().await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "auto"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "detect", "text": "browser text"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut saw_stt = false;
    let mut saw_start = false;
    let mut saw_audio = false;
    loop {
        let message = timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match message {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                match (value["type"].as_str(), value["state"].as_str()) {
                    (Some("stt"), _) => saw_stt = true,
                    (Some("llm"), _) => assert_eq!(value["text"], "Xin chao ban."),
                    (Some("tts"), Some("start")) => saw_start = true,
                    (Some("tts"), Some("stop")) => break,
                    _ => panic!("unexpected control: {value}"),
                }
            }
            Message::Binary(_) => {
                assert!(saw_start, "audio preceded tts:start");
                saw_audio = true;
            }
            other => panic!("unexpected websocket frame: {other:?}"),
        }
    }
    assert!(saw_stt);
    assert!(saw_start);
    assert!(saw_audio);

    task.abort();
}

#[tokio::test]
async fn public_tts_failure_before_first_packet_emits_no_playback_controls() {
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(ControlledFailureTts {
            calls: Arc::clone(&calls),
            after_audio: None,
        }),
    ));
    let (base, task) = start_router_with(providers).await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"auto"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"detect","text":"speak"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, socket.next()).await {
        match frame {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_ne!(
                    value["type"], "tts",
                    "provider failure fabricated playback: {value}"
                );
            }
            Message::Binary(_) => panic!("failed provider emitted audio"),
            _ => {}
        }
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the request must reach TTS"
    );
    task.abort();
}

#[tokio::test]
async fn public_tts_failure_after_first_packet_stops_once_and_fences_late_audio() {
    let (release, held) = std::sync::mpsc::channel();
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(ControlledFailureTts {
            calls: Arc::new(AtomicUsize::new(0)),
            after_audio: Some(Mutex::new(held)),
        }),
    ));
    let (base, task) = start_router_with(providers).await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"auto"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"detect","text":"speak"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut started = false;
    loop {
        let frame = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "tts" {
                    assert_eq!(value["state"], "start");
                    assert!(!started, "duplicate start");
                    started = true;
                }
            }
            Message::Binary(packet) => {
                assert!(started);
                let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
                assert_eq!(
                    decoder.decode(&packet, &mut [0_i16; 1_440], false).unwrap(),
                    1_440
                );
                break;
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
    release.send(()).unwrap();
    loop {
        let frame = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(
                value["type"] == "tts" && value["state"] == "stop",
                "unexpected control {value}"
            );
            break;
        }
    }
    assert!(
        timeout(Duration::from_millis(150), socket.next())
            .await
            .is_err(),
        "stale payload crossed the terminal stop"
    );
    task.abort();
}
