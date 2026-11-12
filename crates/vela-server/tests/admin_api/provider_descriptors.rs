use super::super::*;

#[tokio::test]
async fn provider_adapter_descriptors_and_bootstrap_discovery_are_public_read_only_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let adapters = format!("{base}/api/admin/provider-adapters");

    assert_eq!(
        client.get(&adapters).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    let listed = client
        .get(format!("{adapters}?type=tts"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let mut expected = serde_json::json!([
        {
            "adapter": "zerotts_onnx",
            "type": "tts",
            "display_name": "ZeroTTS"
        },
        {
            "adapter": "chillaudio_ws",
            "type": "tts",
            "display_name": "ChillAudio WebSocket"
        },
        {
            "adapter": "edge_tts",
            "type": "tts",
            "display_name": "Microsoft Edge TTS"
        },
        {
            "adapter": "vieneu",
            "type": "tts",
            "display_name": "VieNeu v3 Turbo Native"
        },
        {
            "adapter": "kokoro_vi_onnx",
            "type": "tts",
            "display_name": "Kokoro Vietnamese ONNX"
        },
        {
            "adapter": "vifasttts_onnx",
            "type": "tts",
            "display_name": "ViFastTts Ngọc Huyền ONNX"
        }
    ]);
    if cfg!(feature = "qualification-providers") {
        expected.as_array_mut().unwrap().insert(
            0,
            serde_json::json!({
                "adapter": "qualification_tts",
                "type": "tts",
                "display_name": "Qualification TTS"
            }),
        );
    }
    assert_eq!(
        listed.json::<serde_json::Value>().await.unwrap()["items"],
        expected
    );
    let bad_filter = client
        .get(format!("{adapters}?type=vision"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(bad_filter.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_filter.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "validation_failed"
    );

    let descriptor = client
        .get(format!("{adapters}/zerotts_onnx"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(descriptor.status(), StatusCode::OK);
    let descriptor = descriptor.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        descriptor["capabilities"]["provider_output_sample_rates"],
        serde_json::json!([48_000])
    );
    assert_eq!(
        descriptor["capabilities"]["voice_delivery_sample_rates"],
        serde_json::json!([24_000])
    );
    assert!(
        descriptor["config_schema"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["key"] == "language")
    );
    assert_eq!(descriptor["discovery"]["voices"], "static");
    assert_eq!(
        descriptor["config_schema"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == "delivery_mode")
            .unwrap()["enum_values"],
        serde_json::json!(["stream", "file"])
    );

    let discovered = client
        .post(format!("{adapters}/zerotts_onnx/capabilities/discover"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"selection":{"model":"zerotts_default"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(discovered.status(), StatusCode::OK);
    let discovered = discovered.json::<serde_json::Value>().await.unwrap();
    assert_eq!(discovered["voices"].as_array().unwrap().len(), 8);
    assert!(
        discovered["voices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|voice| voice["id"] == "maichi")
    );
    assert_eq!(discovered["languages"][0]["id"], "vi-VN");

    let invalid = client
        .post(format!("{adapters}/zerotts_onnx/capabilities/discover"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"selection":{"api_key":"not-allowed"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "capability_discovery_invalid"
    );

    task.abort();
}

#[tokio::test]
async fn descriptor_language_fields_match_provider_config_migration_and_validation() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let providers = format!("{base}/api/admin/providers");

    let legacy = client
        .post(&providers)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "name": "Legacy ZeroTTS",
            "type": "tts",
            "adapter": "zerotts_onnx",
            "config_json": {
                "voice": "maichi"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::CREATED);
    let legacy = legacy.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        legacy["config_json"],
        "{\"voice\":\"maichi\",\"language\":\"vi-VN\",\"preload\":false,\"delivery_mode\":\"stream\"}"
    );
    let mut canonical: serde_json::Value =
        serde_json::from_str(legacy["config_json"].as_str().unwrap()).unwrap();
    canonical
        .as_object_mut()
        .unwrap()
        .insert("adapter".into(), serde_json::json!("zerotts_onnx"));
    assert!(serde_json::from_value::<vela_server::config::TtsInstanceConfig>(canonical).is_ok());

    let incompatible = client
        .post(&providers)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "name": "Wrong language",
            "type": "tts",
            "adapter": "zerotts_onnx",
            "config_json": {
                "voice": "maichi",
                "language": "en-US"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(incompatible.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        incompatible.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_config_invalid"
    );

    let chillaudio = client
        .post(&providers)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "name": "ChillAudio",
            "type": "tts",
            "adapter": "chillaudio_ws",
            "config_json": {
                "voice": "BV421_vivn_streaming"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(chillaudio.status(), StatusCode::CREATED);

    let edge_tts = client
        .post(&providers)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "name": "Edge TTS International",
            "type": "tts",
            "adapter": "edge_tts",
            "config_json": {
                "voice": "ja-JP-NanamiNeural",
                "language": "ja-JP",
                "rate_percent": 10,
                "pitch_hz": -5
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(edge_tts.status(), StatusCode::CREATED);

    task.abort();
}
