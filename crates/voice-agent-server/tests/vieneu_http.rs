use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Response, StatusCode},
    routing::post,
};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use voice_agent_server::{
    config::{RuntimeConfig, TtsInstanceConfig},
    database::provider_config,
    providers::{
        compiled_provider_registry,
        tts::{TtsBinding, TtsError, TtsSynthesisRequest, TtsWorker},
    },
};

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, server)
}

fn worker(base_url: &str, timeout_ms: u64) -> Box<dyn TtsWorker> {
    let config: TtsInstanceConfig = serde_json::from_value(json!({
        "adapter": "vieneu_http", "base_url": base_url, "voice": "Mai Anh",
        "api_key": "fixture-secret", "timeout_ms": timeout_ms,
    }))
    .unwrap();
    compiled_provider_registry()
        .tts_factory("vieneu_http")
        .unwrap()
        .build(&config, &RuntimeConfig::default())
        .unwrap()
        .open_worker()
        .unwrap()
}

fn request() -> TtsSynthesisRequest {
    TtsSynthesisRequest {
        text: "Xin chào Việt Nam".into(),
        selection: TtsBinding::readiness(),
    }
}

#[test]
fn a_retained_worker_reconnects_after_the_service_closes_an_idle_connection() {
    use std::io::{BufRead, BufReader, Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let started = std::time::Instant::now();
            let socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(started.elapsed() < Duration::from_secs(3));
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            // Accepted sockets inherit nonblocking mode on some hosts. The
            // request reader below requires blocking reads with a deadline.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut socket = BufReader::new(socket);
            let mut length = None;
            loop {
                let mut line = String::new();
                assert!(socket.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            socket.read_exact(&mut vec![0; length.unwrap()]).unwrap();
            socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nX-Sample-Rate: 24000\r\nContent-Length: 2\r\n\r\n\x01\x00").unwrap();
            // The client runtime stops polling between syntheses, like Uvicorn's idle timeout.
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let mut worker = worker(&url, 2000);
    for _ in 0..2 {
        let mut samples = 0;
        worker
            .synthesize(&request(), &AtomicBool::new(false), &mut |pcm| {
                samples += pcm.samples().len();
                Ok(())
            })
            .unwrap();
        assert_eq!(samples, 1);
        worker.reset().unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }
    server.join().unwrap();
}

fn pcm_response(body: Body) -> Response<Body> {
    Response::builder()
        .header("content-type", "audio/pcm")
        .header("x-sample-rate", "24000")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn diagnostic_voice_override_does_not_change_the_retained_default() {
    let (sent, mut received) = tokio::sync::mpsc::channel(2);
    let app = Router::new().route(
        "/v1/audio/speech",
        post(move |axum::Json(body): axum::Json<Value>| {
            let sent = sent.clone();
            async move {
                sent.send(body["voice"].clone()).await.unwrap();
                pcm_response(Body::from(vec![0, 1]))
            }
        }),
    );
    let (url, server) = serve(app).await;
    tokio::task::spawn_blocking(move || {
        let mut worker = worker(&url, 2000);
        let cancelled = AtomicBool::new(false);
        let diagnostic = voice_agent_server::providers::tts::TtsDiagnosticRequest {
            text: "Xin chào".into(),
            voice: Some("Trúc Ly".into()),
            language: None,
        };
        worker
            .synthesize_diagnostic(&request(), &diagnostic, &cancelled, &mut |_| Ok(()))
            .unwrap();
        worker
            .synthesize(&request(), &cancelled, &mut |_| Ok(()))
            .unwrap();
    })
    .await
    .unwrap();
    assert_eq!(received.recv().await.unwrap(), "Trúc Ly");
    assert_eq!(received.recv().await.unwrap(), "Mai Anh");
    server.abort();
}

#[tokio::test]
async fn streams_before_completion_preserves_split_samples_and_sends_the_vieneu_contract() {
    let (release, tail) = tokio::sync::oneshot::channel();
    let tail = Arc::new(tokio::sync::Mutex::new(Some(tail)));
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let app = Router::new().route(
        "/v1/audio/speech",
        post(
            move |headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
                let sent = sent.clone();
                let tail = tail.clone();
                async move {
                    sent.send((headers, body)).await.unwrap();
                    let tail = tail.lock().await.take().unwrap();
                    let chunks = stream::once(async {
                        Ok::<_, std::io::Error>(Bytes::from_static(&[0, 128, 255]))
                    })
                    .chain(stream::once(async move {
                        tail.await.unwrap();
                        Ok(Bytes::from_static(&[127, 0, 0]))
                    }));
                    pcm_response(Body::from_stream(chunks))
                }
            },
        ),
    );
    let (url, server) = serve(app).await;
    let first = Arc::new(tokio::sync::Notify::new());
    let observed = first.clone();
    let task = tokio::task::spawn_blocking(move || {
        let mut worker = worker(&url, 2000);
        let mut samples = Vec::new();
        worker
            .synthesize(&request(), &AtomicBool::new(false), &mut |pcm| {
                assert_eq!(pcm.sample_rate_hz(), 24_000);
                samples.extend_from_slice(pcm.samples());
                observed.notify_one();
                Ok(())
            })
            .unwrap();
        worker.reset().unwrap();
        samples
    });
    tokio::time::timeout(Duration::from_secs(1), first.notified())
        .await
        .unwrap();
    assert!(
        !task.is_finished(),
        "first PCM must arrive before the response ends"
    );
    let (headers, body) = received.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer fixture-secret");
    assert_eq!(
        body,
        json!({
            "model": "vieneu-v3-turbo", "input": "Xin chào Việt Nam", "voice": "Mai Anh",
            "response_format": "pcm", "stream_format": "audio", "sample_rate": 24000,
        })
    );
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap(), vec![-1.0, 32767.0 / 32768.0, 0.0]);
    server.abort();
}

#[tokio::test]
async fn rejects_http_errors_wrong_audio_contract_and_empty_or_incomplete_pcm() {
    for (status, content_type, rate, body, expected) in [
        (401, "audio/pcm", "24000", vec![0, 0], "remote"),
        (429, "audio/pcm", "24000", vec![0, 0], "remote"),
        (302, "audio/pcm", "24000", vec![0, 0], "remote"),
        (200, "audio/wav", "24000", vec![0, 0], "decode"),
        (200, "audio/pcm", "48000", vec![0, 0], "decode"),
        (200, "audio/pcm", "24000", vec![], "decode"),
        (200, "audio/pcm", "24000", vec![0, 0, 255], "decode"),
        (
            200,
            "audio/pcm",
            "24000",
            vec![0; 24_000 * 2 * 120 + 1],
            "decode",
        ),
    ] {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(move || {
                let body = body.clone();
                async move {
                    Response::builder()
                        .status(StatusCode::from_u16(status).unwrap())
                        .header("content-type", content_type)
                        .header("x-sample-rate", rate)
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let (url, server) = serve(app).await;
        let error = tokio::task::spawn_blocking(move || {
            worker(&url, 1000)
                .synthesize(&request(), &AtomicBool::new(false), &mut |_| Ok(()))
                .unwrap_err()
        })
        .await
        .unwrap();
        assert!(
            match expected {
                "remote" => matches!(error, TtsError::RemoteTask),
                _ => matches!(error, TtsError::AudioDecode),
            },
            "{error:?}"
        );
        server.abort();
    }
}

#[tokio::test]
async fn cancellation_and_total_timeout_stop_a_stalled_stream() {
    for cancel in [false, true] {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(|| async {
                let chunks =
                    stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(&[0, 0])) })
                        .chain(stream::pending());
                pcm_response(Body::from_stream(chunks))
            }),
        );
        let (url, server) = serve(app).await;
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let first = Arc::new(tokio::sync::Notify::new());
        let observed = first.clone();
        let task = tokio::task::spawn_blocking(move || {
            worker(&url, if cancel { 5000 } else { 100 }).synthesize(&request(), &flag, &mut |_| {
                observed.notify_one();
                Ok(())
            })
        });
        tokio::time::timeout(Duration::from_secs(1), first.notified())
            .await
            .unwrap();
        if cancel {
            cancelled.store(true, Ordering::Release);
        }
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            if cancel {
                matches!(error, TtsError::Failed)
            } else {
                matches!(error, TtsError::RemoteTimeout)
            },
            "{error:?}"
        );
        server.abort();
    }
}

#[tokio::test]
async fn a_consumer_that_blocks_past_the_deadline_cannot_report_success() {
    let app = Router::new().route(
        "/v1/audio/speech",
        post(|| async { pcm_response(Body::from(vec![0u8; 2])) }),
    );
    let (url, server) = serve(app).await;
    let (release, blocked) = std::sync::mpsc::channel();
    let first = Arc::new(tokio::sync::Notify::new());
    let observed = first.clone();
    let task = tokio::task::spawn_blocking(move || {
        worker(&url, 100).synthesize(&request(), &AtomicBool::new(false), &mut |_| {
            observed.notify_one();
            blocked.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
    });
    tokio::time::timeout(Duration::from_secs(1), first.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    release.send(()).unwrap();
    assert!(matches!(task.await.unwrap(), Err(TtsError::RemoteTimeout)));
    server.abort();
}

#[test]
fn desired_config_is_credential_free_and_rejects_unsafe_urls_and_invalid_selection() {
    let config = json!({"base_url":"http://vieneu:8000/v1", "voice":"Mai Anh"});
    let canonical = provider_config::validate("vieneu_http", &config).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&canonical).unwrap()["language"],
        "vi-VN"
    );
    for (key, value) in [
        ("base_url", json!("file:///tmp/audio")),
        ("base_url", json!("http://user:password@vieneu:8000/v1")),
        ("base_url", json!("http://vieneu:8000/v1?key=secret")),
        ("base_url", json!("http://vieneu:8000/v1#fragment")),
        ("voice", json!("\n")),
        ("language", json!("en-US")),
        ("timeout_ms", json!(0)),
        ("timeout_ms", json!(120001)),
        ("api_key", json!("forbidden")),
    ] {
        let mut bad = config.clone();
        bad[key] = value;
        assert!(
            provider_config::validate("vieneu_http", &bad).is_err(),
            "{key}"
        );
    }
    let instance: TtsInstanceConfig = serde_json::from_value(json!({
        "adapter":"vieneu_http", "base_url":"http://vieneu:8000/v1", "voice":"Mai Anh", "api_key":"never-display"
    })).unwrap();
    assert!(!format!("{instance:?}").contains("never-display"));
    assert!(
        !serde_json::to_string(&instance)
            .unwrap()
            .contains("never-display")
    );
}

#[tokio::test]
async fn database_runtime_resolves_bearer_credentials_and_returns_diagnostic_audio() {
    use voice_agent_server::{
        config::AppConfig,
        database::{
            DesiredProvider,
            secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
        },
        providers::{materialize_provider, tts::TtsDiagnosticRequest},
        services::provider_diagnostic::ProviderDiagnosticOperation,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
            Ok(SecretValue::new("database-fixture-secret".into()))
        }
    }
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let app = Router::new().route(
        "/v1/audio/speech",
        post(move |headers: HeaderMap| {
            let sent = sent.clone();
            async move {
                sent.send(headers).await.unwrap();
                pcm_response(Body::from(vec![0, 128, 0, 64]))
            }
        }),
    );
    let (url, server) = serve(app).await;
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "vad"
asr = "asr"
llm = "llm"
tts = "tts"
"#,
    )
    .unwrap();
    let catalog = tokio::task::spawn_blocking(move || {
        materialize_provider(
            &config,
            &DesiredProvider {
                id: 1,
                revision: 1,
                key: "vieneu".into(),
                kind: "tts".into(),
                adapter: "vieneu_http".into(),
                config_json: provider_config::validate(
                    "vieneu_http",
                    &json!({"base_url":url, "voice":"Mai Anh"}),
                )
                .unwrap(),
                secret_ref: Some("fixture".into()),
            },
            &Secrets,
        )
        .unwrap()
    })
    .await
    .unwrap();
    let runtime = catalog.tts("vieneu").unwrap();
    let diagnostic = TtsDiagnosticRequest {
        text: "Xin chào".into(),
        voice: None,
        language: Some("vi-VN".into()),
    };
    runtime.validate_diagnostic(&diagnostic).unwrap();
    let mut operation = runtime.diagnostic(diagnostic);
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        operation.execute(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        received.recv().await.unwrap()["authorization"],
        "Bearer database-fixture-secret"
    );
    let reader = hound::WavReader::new(std::io::Cursor::new(output.wav)).unwrap();
    assert_eq!(reader.spec().sample_rate, 24_000);
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(reader.duration(), 2);
    drop(operation);
    assert!(
        tokio::task::spawn_blocking(move || runtime.shutdown_acknowledged())
            .await
            .unwrap()
    );
    server.abort();
}
