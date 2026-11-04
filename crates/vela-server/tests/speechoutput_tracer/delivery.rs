use super::harness::*;

#[tokio::test]
async fn non_empty_asr_final_delivers_started_canonical_opus_then_one_stop() {
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (audio_tx, mut audio_rx) = mpsc::channel(8);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(FakeTts),
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
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);

    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .map(|message| message.as_text().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 4);
    let controls = controls
        .iter()
        .map(|control| serde_json::from_str::<serde_json::Value>(control).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(controls[0]["type"], "stt");
    assert_eq!(controls[1]["type"], "llm");
    assert_eq!(controls[1]["text"], "Xin chao ban.");
    assert_eq!(controls[2]["type"], "tts");
    assert_eq!(controls[2]["state"], "start");
    assert_eq!(controls[3]["type"], "tts");
    assert_eq!(controls[3]["state"], "stop");

    let packet = match audio_rx.try_recv().unwrap() {
        OutboundMessage::Binary { packet, .. } => packet,
        unexpected => panic!("expected downlink audio, got {unexpected:?}"),
    };
    assert!(audio_rx.try_recv().is_err());
    let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
    let mut pcm = [0_i16; 1_440];
    assert_eq!(decoder.decode(&packet, &mut pcm, false).unwrap(), 1_440);
}

#[tokio::test]
async fn full_outbound_audio_queue_retries_every_packet_without_a_gap() {
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (audio_tx, mut audio_rx) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(LongTts),
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
        if audio_rx.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(audio_rx.len(), 1, "first packet must be queued");
    assert_eq!(actor.phase(), SessionPhase::Speaking);

    let mut packets = Vec::new();
    for _ in 0..3 {
        packets.push(audio_rx.recv().await.expect("queued packet"));
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(1)).await;
            actor.pump_workers();
            if audio_rx.len() == 1 || actor.phase() == SessionPhase::Ready {
                break;
            }
        }
    }
    assert!(
        packets
            .iter()
            .all(|message| matches!(message, OutboundMessage::Binary { .. }))
    );
    assert_eq!(
        packets.len(),
        3,
        "no canonical packet may be silently dropped"
    );

    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Listening {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert!(
        std::iter::from_fn(|| control_rx.try_recv().ok()).any(|message| {
            message
                .as_text()
                .is_some_and(|text| text.contains(r#""state":"stop""#))
        })
    );
}

#[tokio::test]
async fn explicit_abort_admits_urgent_stop_and_invalidates_queued_turn_payload_under_pressure() {
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (urgent_tx, mut urgent_rx) = mpsc::channel(1);
    let (audio_tx, mut audio_rx) = mpsc::channel(1);
    let gate = Arc::new(vela_server::session::GenerationGate::new());
    let vad: Arc<dyn VadProvider> = Arc::new(FakeVad);
    let asr: Arc<dyn AsrProvider> = Arc::new(FakeAsr);
    let llm: Arc<dyn LlmProvider> = Arc::new(FakeLlm);
    let tts: Arc<dyn TtsProvider> = Arc::new(LongTts);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::clone(&vad),
        Arc::clone(&asr),
        Arc::clone(&llm),
        Arc::clone(&tts),
    ));
    let runtimes = vela_server::session::SessionRuntimes {
        job_submit: None,
        asr: Arc::new(vela_server::workers::AsrWorkerRuntime::new(
            asr,
            vela_server::workers::WorkerRuntimeConfig::default(),
        )),
        vad: Arc::new(vela_server::workers::VadWorkerRuntime::new(
            vad,
            vela_server::workers::WorkerRuntimeConfig::default(),
        )),
        llm: Arc::new(vela_server::workers::LlmRuntime::new(
            llm,
            1,
            Duration::from_secs(60),
        )),
        tts: Arc::new(vela_server::workers::TtsWorkerRuntime::new(
            tts,
            vela_server::workers::WorkerRuntimeConfig::default(),
        )),
        active_turn_limiter: Arc::new(vela_server::session::ActiveTurnLimiter::new(1)),
        vad_segmenter_config: vela_server::audio::VadSegmenterConfig::default(),
        pre_roll_samples: 4_800,
    };
    let mut actor = SessionActor::new_with_runtimes_and_limiter_and_outbound(
        "session".into(),
        control_tx.clone(),
        urgent_tx,
        audio_tx,
        Arc::clone(&gate),
        tokio::sync::watch::channel(false).0,
        2,
        20,
        runtimes,
    )
    .unwrap()
    .with_delivery_providers(&providers)
    .unwrap();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    for _ in 0..200 {
        actor.pump_workers();
        if audio_rx.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(audio_rx.len(), 1, "the started turn must have queued audio");

    while control_rx.try_recv().is_ok() {}
    for index in 0..8 {
        control_tx
            .try_send(OutboundMessage::TurnText {
                generation: 1,
                turn_id: TurnId::new(1).unwrap(),
                text: format!(r#"{{"type":"llm","text":"queued-{index}"}}"#),
            })
            .unwrap();
    }
    actor.on_client_message(ClientMessage::Abort { session_id: None });
    actor.on_client_message(ClientMessage::Abort { session_id: None });

    let stop = urgent_rx
        .try_recv()
        .expect("abort must bypass normal queue pressure");
    assert!(
        stop.as_text()
            .is_some_and(|text| text.contains(r#""state":"stop""#))
    );
    assert!(
        !gate.admits(1),
        "the writer must reject queued JSON and audio from the aborted turn"
    );
    assert!(matches!(
        audio_rx.try_recv(),
        Ok(OutboundMessage::Binary { generation: 1, .. })
    ));
    assert!(urgent_rx.try_recv().is_err(), "double abort emits one stop");
    assert_eq!(actor.phase(), SessionPhase::Ready);
}

#[tokio::test]
async fn public_abort_with_bounded_lanes_sends_one_stop_then_writer_stays_quiet() {
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(LongTts),
    ));
    let (base, task) = start_router_with_limits(
        providers,
        LimitsConfig {
            outbound_control_queue: 1,
            urgent_control_queue: 1,
            outbound_audio_queue: 1,
            ..LimitsConfig::default()
        },
    )
    .await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"manual"})
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
            serde_json::json!({"type":"listen","state":"stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut saw_start = false;
    loop {
        let message = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        saw_start |= matches!(&message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).unwrap()["type"] == "tts" && serde_json::from_str::<serde_json::Value>(text).unwrap()["state"] == "start");
        if matches!(message, Message::Binary(_)) {
            break;
        }
    }
    assert!(saw_start, "tts:start must precede the observed audio");

    socket
        .send(Message::Text(
            serde_json::json!({"type":"abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    loop {
        let message = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if matches!(&message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).unwrap()["type"] == "tts" && serde_json::from_str::<serde_json::Value>(text).unwrap()["state"] == "stop")
        {
            break;
        }
    }
    assert!(
        timeout(Duration::from_millis(200), socket.next())
            .await
            .is_err(),
        "writer must not deliver stale JSON or Opus after interruption stop"
    );
    task.abort();
}

#[tokio::test]
async fn listen_arm_during_delivery_does_not_interrupt_the_active_turn() {
    let (control_tx, mut control_rx) = mpsc::channel(32);
    let (audio_tx, _audio_rx) = mpsc::channel(32);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    // The first arm arrives while ASR finalization is still Processing; the
    // duplicate is a normal client retry. Neither may cancel that turn.
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));

    let mut controls = Vec::new();
    for _ in 0..100 {
        actor.pump_workers();
        controls.extend(std::iter::from_fn(|| control_rx.try_recv().ok()));
        if controls.iter().any(|message| {
            message
                .as_text()
                .is_some_and(|text| text.contains(r#""state":"start""#))
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(controls.iter().any(|message| {
        message
            .as_text()
            .is_some_and(|text| text.contains(r#""state":"start""#))
    }));

    for _ in 0..100 {
        actor.pump_workers();
        controls.extend(std::iter::from_fn(|| control_rx.try_recv().ok()));
        if actor.phase() == SessionPhase::Listening {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    controls.extend(std::iter::from_fn(|| control_rx.try_recv().ok()));

    assert_eq!(actor.phase(), SessionPhase::Listening);
    assert_eq!(actor.dialogue_history(), ["xin chao", "Xin chao ban."]);
    assert_eq!(
        controls
            .iter()
            .filter(|message| message
                .as_text()
                .is_some_and(|text| text.contains(r#""state":"stop""#)))
            .count(),
        1,
        "listen:start must leave the active turn to its normal terminal stop"
    );
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    assert_eq!(actor.phase(), SessionPhase::Ready);
}
