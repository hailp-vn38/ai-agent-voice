use super::harness::*;

#[tokio::test]
async fn config_diagnostics_acquire_unbound_cold_deployment_and_reuse_prepare_without_db_collision()
{
    let (base, _db, builds, task) = server(true).await;
    let c = client(&base).await;
    for key in ["primary", "config:llm:primary", "config:llm:primary"] {
        let response = c
            .post(format!("{base}/providers/{key}/test/llm"))
            .json(&json!({"input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["provider_key"], key);
        assert_eq!(result["runtime"]["runtime_matches_desired"], true);
        assert_eq!(result["runtime"]["requires_restart"], false);
        assert_eq!(
            result["runtime"]["tested_provider_id"].is_null(),
            key.starts_with("config:")
        );
        assert!(!result.to_string().contains("SECRET_SENTINEL"));
        if key == "primary" {
            let cold = c
                .get(format!("{base}/providers/config:llm:primary/capabilities"))
                .send()
                .await
                .unwrap();
            assert_eq!(cold.status(), StatusCode::CONFLICT);
            assert!(
                cold.text()
                    .await
                    .unwrap()
                    .contains("provider_runtime_not_loaded")
            );
            assert_eq!(builds.load(Ordering::SeqCst), 1);
        }
    }
    let missing = c
        .post(format!("{base}/providers/config:llm:missing/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.text().await.unwrap().contains("provider_not_found"));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let response = c
        .post(format!("{base}/providers/config:llm:primary/prepare"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let capabilities = get(
        &c,
        &format!("{base}/providers/config:llm:primary/capabilities"),
    )
    .await;
    assert_eq!(capabilities["source"], "config");
    assert_eq!(capabilities["capabilities"]["voices"][0], "fixture");
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let response = c
        .post(format!("{base}/providers/config:tts:primary/test/tts"))
        .json(&json!({"text":"hello", "language":"vi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "audio/wav");
    assert_eq!(response.headers()["x-provider-key"], "config:tts:primary");
    assert!(!response.headers().contains_key("x-provider-tested-id"));
    assert!(response.bytes().await.unwrap().starts_with(b"RIFF"));
    task.abort();
}

#[tokio::test]
async fn legacy_config_diagnostic_requires_manager_and_never_uses_matching_db_key() {
    let (base, _db, builds, task) = server(false).await;
    let response = client(&base).await
        .post(format!("{base}/providers/config:llm:primary/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("config_provider_requires_managed_runtime")
    );
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn config_vad_and_asr_keep_validation_and_public_provenance() {
    let (base, _db, builds, task) = server(true).await;
    let c = client(&base).await;
    let result = c
        .post(format!("{base}/providers/config:vad:vad/test/vad"))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["provider_key"], "config:vad:vad");
    assert_eq!(result["source"], "config");
    assert_eq!(result["result"]["probability"], 0.25);
    let bad = c
        .post(format!("{base}/providers/config:vad:vad/test/vad"))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let mut wav = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..1600 {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
    let result = c
        .post(format!("{base}/providers/config:asr:asr/test/asr"))
        .header("content-type", "audio/wav")
        .body(wav.into_inner())
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["provider_key"], "config:asr:asr");
    assert_eq!(result["result"]["text"], "recognized");
    assert_eq!(result["runtime"]["tested_provider_id"], Value::Null);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn config_cold_acquisition_is_bounded_and_busy_without_releasing_the_native_attempt() {
    let (base, _db, builds, task, manager) =
        server_configured(true, std::time::Duration::from_millis(300), false, 100).await;
    let manager = manager.unwrap();
    let c = client(&base).await;
    let url = format!("{base}/providers/config:llm:primary/test/llm");
    let pending = tokio::spawn({
        let c = c.clone();
        let url = url.clone();
        async move {
            c.post(url)
                .json(&json!({"input":"hello"}))
                .send()
                .await
                .unwrap()
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while builds.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let busy = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    let timed_out = pending.await.unwrap();
    assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(manager.accounting().reserved_bytes > 0);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while manager.accounting().reserved_build_bytes > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let result = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn config_operation_timeout_quarantines_deployment_lease_without_terminal_ack() {
    let (base, _db, builds, task, manager) =
        server_configured(true, std::time::Duration::ZERO, true, 100).await;
    let manager = manager.unwrap();
    let c = client(&base).await;
    let url = format!("{base}/providers/config:llm:primary/test/llm");
    let response = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(manager.accounting().active_leases, 1);
    let busy = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(manager.accounting().active_leases, 1);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    task.abort();
}
