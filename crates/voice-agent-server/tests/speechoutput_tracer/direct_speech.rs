use super::harness::*;

#[tokio::test]
async fn admin_direct_speech_uses_current_session_and_rejects_offline_legacy_and_overrides() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let asr_calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(CountedAsr(Arc::clone(&asr_calls))),
        Arc::new(FailingLlm),
        Arc::new(RecordingTts(Arc::clone(&requests))),
    ));
    let (base, task) = start_router_with(providers).await;
    let http = reqwest::Client::new();
    let path = format!("{base}/api/admin/devices/tracer-device/speak");
    let body = serde_json::json!({"request_id":"admin-1","text":"An admin notification: {\"voltage\":5}."});
    assert_eq!(
        http.post(&path).json(&body).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let response = http
        .post(&path)
        .bearer_auth("direct-admin")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "device_offline"
    );
    let mut legacy = connect_router(&base).await;
    assert_eq!(
        http.post(&path)
            .bearer_auth("direct-admin")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let mut socket = connect_router_features(&base, true).await;
    let _ = timeout(Duration::from_secs(2), legacy.next()).await;
    let presence: serde_json::Value = http
        .get(format!("{base}/api/admin/devices/tracer-device"))
        .bearer_auth("direct-admin")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(presence["ws_online"], true);
    assert_eq!(presence["direct_tts_supported"], true);
    let spoof = serde_json::json!({"request_id":"spoof","text":"hello","provider_id":"evil"});
    assert_eq!(
        http.post(&path)
            .bearer_auth("direct-admin")
            .json(&spoof)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let invalid = serde_json::json!({"request_id":"bad","text":"\u{0001}"});
    assert_eq!(
        http.post(&path)
            .bearer_auth("direct-admin")
            .json(&invalid)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let response = http
        .post(&path)
        .bearer_auth("direct-admin")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["status"],
        "accepted"
    );
    let mut observed = Vec::new();
    let mut audio = 0;
    loop {
        let frame = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    value["type"], "tts",
                    "Direct Speech invoked ASR/LLM: {value}"
                );
                let state = value["state"].as_str().unwrap();
                if state == "start" || state == "stop" {
                    assert_eq!(value["source"], "admin_direct");
                    assert_eq!(value["request_id"], "admin-1");
                }
                observed.push(state.to_owned());
                if state == "result" {
                    assert_eq!(value["outcome"], "completed");
                    break;
                }
            }
            Message::Binary(packet) => {
                assert_eq!(observed, ["start", "sentence_start"]);
                let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
                assert_eq!(
                    decoder.decode(&packet, &mut [0_i16; 1_440], false).unwrap(),
                    1_440
                );
                audio += 1;
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
    assert_eq!(observed, ["start", "sentence_start", "stop", "result"]);
    assert_eq!(audio, 1);
    assert_eq!(
        *requests.lock().unwrap(),
        ["An admin notification: {\"voltage\":5}."]
    );
    assert_eq!(asr_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        http.post(&path)
            .bearer_auth("direct-admin")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    for _ in 0..40 {
        if asr_calls.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        asr_calls.load(Ordering::SeqCst),
        1,
        "manual capture must open before admission is probed"
    );
    let busy = http
        .post(&path)
        .bearer_auth("direct-admin")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(
        busy.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "session_busy"
    );
    socket.close(None).await.unwrap();
    for _ in 0..40 {
        let presence: serde_json::Value = http
            .get(format!("{base}/api/admin/devices/tracer-device"))
            .bearer_auth("direct-admin")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if presence["ws_online"] == false {
            task.abort();
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("closed session retained live presence");
}
