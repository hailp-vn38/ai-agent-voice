use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use url::Url;

use crate::{
    config::SecretString,
    providers::llm::{
        LLM_CAPABILITY_TAGS, LlmCapabilityTag, LlmError, LlmOperation, LlmOutput, LlmProvider,
        LlmRequest,
        openai::{config::OpenAiConfig, provider::ConfiguredOpenAiLlm},
    },
};

#[tokio::test]
async fn capability_guard_rejects_untagged_operations_before_network() {
    let config = OpenAiConfig {
        api_key: SecretString::new("sk-test"),
        base_url: Url::parse("http://127.0.0.1:9/v1").unwrap(),
        model: "text-embedding-3-small".into(),
        timeout_ms: 1_000,
        tags: Some(vec!["embedding".into()]),
        ..OpenAiConfig::default()
    };
    let provider = ConfiguredOpenAiLlm::from_config(&config).unwrap();
    assert_eq!(provider.capability_tags(), &[LlmCapabilityTag::Embedding]);
    assert!(!provider.supports_tools());

    let stream_err = match provider.stream(LlmRequest::from("hello")).await {
        Ok(_) => panic!("expected UnsupportedCapability for stream on embedding-only provider"),
        Err(err) => err,
    };
    assert_eq!(stream_err, LlmError::UnsupportedCapability("chat"));

    let image_err = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "a cat".into(),
            width: 256,
            height: 256,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap_err();
    assert_eq!(image_err, LlmError::UnsupportedCapability("image"));
}

#[tokio::test]
async fn openai_provider_executes_all_seven_capabilities_against_upstream() {
    let captured_responses_body: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let captured_state = Arc::clone(&captured_responses_body);

    let app = Router::new()
        .route(
            "/v1/responses",
            post(
                move |State(state): State<Arc<Mutex<Option<Value>>>>,
                      Json(body): Json<Value>| async move {
                    *state.lock().unwrap() = Some(body);
                    let sse = concat!(
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Reasoned \"}\n\n",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n",
                        "data: [DONE]\n\n"
                    );
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        sse,
                    )
                },
            ),
        )
        .route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                let image_url = body
                    .pointer("/messages/0/content/1/image_url/url")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                assert!(image_url.starts_with("data:image/png;base64,"));
                Json(serde_json::json!({
                    "choices": [{
                        "message": { "content": "Vision detected a badge" }
                    }]
                }))
            }),
        )
        .route(
            "/v1/images/generations",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body.get("size").and_then(Value::as_str), Some("512x512"));
                Json(serde_json::json!({
                    "data": [{
                        "b64_json": STANDARD.encode(b"FAKE_PNG_IMAGE")
                    }]
                }))
            }),
        )
        .route(
            "/v1/videos/generations",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body.get("size").and_then(Value::as_str), Some("1280x720"));
                assert_eq!(body.get("duration").and_then(Value::as_u64), Some(5));
                Json(serde_json::json!({
                    "data": [{
                        "b64_json": STANDARD.encode(b"FAKE_MP4_VIDEO")
                    }]
                }))
            }),
        )
        .route(
            "/v1/audio/speech",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body.get("voice").and_then(Value::as_str), Some("nova"));
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, "audio/wav".parse().unwrap());
                (StatusCode::OK, headers, b"RIFF_FAKE_WAV".to_vec()).into_response()
            }),
        )
        .route(
            "/v1/audio/transcriptions",
            post(|mut multipart: Multipart| async move {
                let mut saw_file = false;
                let mut saw_lang = false;
                while let Some(field) = multipart.next_field().await.unwrap() {
                    match field.name() {
                        Some("file") => {
                            let bytes = field.bytes().await.unwrap();
                            assert_eq!(&bytes[..], b"RIFF_INPUT_WAV");
                            saw_file = true;
                        }
                        Some("language") => {
                            assert_eq!(field.text().await.unwrap(), "vi");
                            saw_lang = true;
                        }
                        _ => {}
                    }
                }
                assert!(saw_file && saw_lang);
                Json(serde_json::json!({ "text": "Xin chào thế giới" }))
            }),
        )
        .route(
            "/v1/embeddings",
            post(|Json(body): Json<Value>| async move {
                let inputs = body.get("input").and_then(Value::as_array).unwrap();
                assert_eq!(inputs.len(), 2);
                Json(serde_json::json!({
                    "data": [
                        { "embedding": [0.1, 0.2, 0.3] },
                        { "embedding": [0.4, 0.5, 0.6] }
                    ]
                }))
            }),
        )
        .with_state(captured_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = OpenAiConfig {
        api_key: SecretString::new("sk-all-caps"),
        base_url: Url::parse(&format!("http://{addr}/v1")).unwrap(),
        model: "gpt-omni-test".into(),
        timeout_ms: 5_000,
        max_tokens: Some(1_024),
        context_limit_tokens: Some(128_000),
        reasoning_effort: Some("high".into()),
        temperature_percent: Some(75),
        tags: Some(
            LLM_CAPABILITY_TAGS
                .iter()
                .map(|tag| (*tag).to_owned())
                .collect(),
        ),
        gateway_binding: None,
    };
    let provider = ConfiguredOpenAiLlm::from_config(&config).unwrap();

    // 1 & 6. Chat + Reasoning
    let chat_out = provider
        .invoke(LlmOperation::Chat {
            request: LlmRequest::from("Solve this"),
            reasoning: true,
        })
        .await
        .unwrap();
    assert_eq!(chat_out, LlmOutput::Text("Reasoned answer".into()));
    let sent_body = captured_responses_body
        .lock()
        .unwrap()
        .clone()
        .expect("responses request body captured");
    assert_eq!(
        sent_body.get("max_output_tokens").and_then(Value::as_u64),
        Some(1_024)
    );
    assert_eq!(
        sent_body
            .pointer("/reasoning/effort")
            .and_then(Value::as_str),
        Some("high")
    );
    let temp = sent_body
        .get("temperature")
        .and_then(Value::as_f64)
        .unwrap();
    assert!((temp - 0.75).abs() < 1e-4);

    // 2. Vision
    let vision_out = provider
        .invoke(LlmOperation::Vision {
            prompt: "Describe image".into(),
            image_bytes: vec![1, 2, 3, 4],
            mime_type: "image/png".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        vision_out,
        LlmOutput::Text("Vision detected a badge".into())
    );

    // 3. Image Generation
    let image_out = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "A futuristic city".into(),
            width: 512,
            height: 512,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(
        image_out,
        LlmOutput::single_media(b"FAKE_PNG_IMAGE".to_vec(), "image/png")
    );

    // 4. Video Generation
    let video_out = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "Waves crashing".into(),
            width: 1280,
            height: 720,
            duration_secs: 5,
            first_frame: None,
        })
        .await
        .unwrap();
    assert_eq!(
        video_out,
        LlmOutput::single_media(b"FAKE_MP4_VIDEO".to_vec(), "video/mp4")
    );

    // 5a. Audio Speech (TTS)
    let speech_out = provider
        .invoke(LlmOperation::AudioSpeech {
            text: "Xin chào".into(),
            voice: Some("nova".into()),
            speed: Some(1.0),
        })
        .await
        .unwrap();
    assert_eq!(
        speech_out,
        LlmOutput::single_media(b"RIFF_FAKE_WAV".to_vec(), "audio/wav")
    );

    // 5b. Audio Transcription (STT)
    let stt_out = provider
        .invoke(LlmOperation::AudioTranscription {
            wav_bytes: b"RIFF_INPUT_WAV".to_vec(),
            language: Some("vi".into()),
        })
        .await
        .unwrap();
    assert_eq!(stt_out, LlmOutput::Text("Xin chào thế giới".into()));

    // 7. Embedding
    let emb_out = provider
        .invoke(LlmOperation::Embedding {
            inputs: vec!["first".into(), "second".into()],
        })
        .await
        .unwrap();
    assert_eq!(
        emb_out,
        LlmOutput::Embeddings {
            vectors: vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]],
            dimensions: 3,
        }
    );
}

#[tokio::test]
async fn video_generation_polls_async_job_status_until_completed_and_fetches_content() {
    let poll_count = Arc::new(Mutex::new(0u32));
    let poll_state = Arc::clone(&poll_count);

    let app = Router::new()
        .route(
            "/v1/videos",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body.get("model").and_then(Value::as_str), Some("sora-2"));
                assert_eq!(
                    body.get("prompt").and_then(Value::as_str),
                    Some("A drone shot over mountains")
                );
                assert_eq!(body.get("size").and_then(Value::as_str), Some("1280x720"));
                assert_eq!(body.get("seconds").and_then(Value::as_u64), Some(8));
                assert!(
                    body.get("duration").is_none(),
                    "POST /v1/videos must not send legacy duration field"
                );
                Json(serde_json::json!({
                    "id": "video_job_42",
                    "object": "video",
                    "status": "queued"
                }))
            }),
        )
        .route(
            "/v1/videos/video_job_42",
            get(move || {
                let poll_state = Arc::clone(&poll_state);
                async move {
                    let mut count = poll_state.lock().unwrap();
                    *count += 1;
                    let status = if *count < 2 {
                        "in_progress"
                    } else {
                        "completed"
                    };
                    Json(serde_json::json!({
                        "id": "video_job_42",
                        "object": "video",
                        "status": status
                    }))
                }
            }),
        )
        .route(
            "/v1/videos/video_job_42/content",
            get(|| async move {
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
                (StatusCode::OK, headers, b"ASYNC_SORA_MP4_BYTES".to_vec()).into_response()
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let provider = ConfiguredOpenAiLlm::from_config(&OpenAiConfig {
        api_key: SecretString::new("sk-sora"),
        base_url: Url::parse(&format!("http://{addr}/v1")).unwrap(),
        model: "sora-2".into(),
        timeout_ms: 5_000,
        tags: Some(vec!["video".into()]),
        ..OpenAiConfig::default()
    })
    .unwrap();

    let video_out = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "A drone shot over mountains".into(),
            width: 1280,
            height: 720,
            duration_secs: 8,
            first_frame: None,
        })
        .await
        .unwrap();

    assert_eq!(*poll_count.lock().unwrap(), 2);
    assert_eq!(
        video_out,
        LlmOutput::single_media(b"ASYNC_SORA_MP4_BYTES".to_vec(), "video/mp4")
    );
}
