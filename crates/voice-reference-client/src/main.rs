use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use opus2::{Application, Channels, Encoder};
use serde_json::json;
use sherpa_onnx::{
    OnlineRecognizer, OnlineRecognizerConfig, SileroVadModelConfig, VadModelConfig,
    VoiceActivityDetector,
};
use std::path::{Path, PathBuf};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[derive(Debug, Parser)]
#[command(
    name = "voice-reference-client",
    about = "Independent voice WebSocket v1 reference client"
)]
struct Args {
    #[arg(long)]
    ota: String,
    #[arg(long, default_value = "reference-client-01")]
    device_id: String,
    #[arg(long, default_value = "reference-client")]
    client_id: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run local Silero VAD and Zipformer ASR smoke checks against a WAV without a server.
    TestVadAsrWav {
        #[arg(default_value = "docs/audio.wav")]
        file: PathBuf,
        #[arg(long, default_value = "models/vad/silero_vad.onnx")]
        vad_model: PathBuf,
        #[arg(long, default_value = "models/asr/zipformer-30m-vi")]
        asr_model_dir: PathBuf,
    },
    Handshake,
    Listen,
    SendOpus {
        file: PathBuf,
    },
    /// Encode a PCM16 mono WAV as canonical 16 kHz/60 ms Opus and send it.
    SendWav {
        #[arg(default_value = "docs/audio.wav")]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = ListenModeArg::Manual)]
        mode: ListenModeArg,
    },
    /// Complete hello, then require one raw binary packet matching this hex fixture.
    ReceiveBinary {
        #[arg(long)]
        expected_hex: String,
    },
    /// Decode one canonical 24 kHz mono downlink Opus packet produced by a Voice Protocol Server.
    DecodeDownlinkOpus {
        file: PathBuf,
    },
    ProtocolTest {
        #[arg(long)]
        case: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ListenModeArg {
    Manual,
    Auto,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}

async fn run(args: Args) -> anyhow::Result<()> {
    if let Command::TestVadAsrWav {
        file,
        vad_model,
        asr_model_dir,
    } = &args.command
    {
        return test_vad_asr_wav(file, vad_model, asr_model_dir);
    }
    if let Command::DecodeDownlinkOpus { file } = &args.command {
        return decode_downlink_opus(file);
    }
    let ota: serde_json::Value = reqwest::Client::new()
        .post(&args.ota)
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let websocket = ota
        .get("websocket")
        .context("OTA response lacks websocket")?;
    let url = websocket
        .get("url")
        .and_then(|value| value.as_str())
        .context("OTA response lacks websocket.url")?;
    let token = websocket
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse()?);
    headers.insert("Device-Id", args.device_id.parse()?);
    headers.insert("Client-Id", args.client_id.parse()?);
    if !token.is_empty() {
        headers.insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (mut socket, _) = connect_async(request).await.context("connect WebSocket")?;
    let hello = json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}});
    match args.command {
        Command::ProtocolTest { case } => send_protocol_case(&mut socket, &case).await?,
        command => {
            socket.send(Message::Text(hello.to_string().into())).await?;
            print_next(&mut socket).await?;
            match command {
                Command::Handshake => {}
                Command::Listen => send_listen(&mut socket).await?,
                Command::SendOpus { file } => {
                    send_listen_start(&mut socket).await?;
                    socket
                        .send(Message::Binary(tokio::fs::read(file).await?.into()))
                        .await?;
                }
                Command::SendWav { file, mode } => send_wav(&mut socket, &file, mode).await?,
                Command::ReceiveBinary { expected_hex } => {
                    let expected = decode_hex(&expected_hex)?;
                    match socket.next().await {
                        Some(Ok(Message::Binary(actual))) if actual == expected => {
                            println!("received matching binary packet ({} bytes)", actual.len());
                        }
                        Some(Ok(Message::Binary(actual))) => bail!(
                            "binary payload mismatch: expected {} bytes, received {} bytes",
                            expected.len(),
                            actual.len()
                        ),
                        Some(Ok(other)) => bail!("expected binary packet, received {other:?}"),
                        Some(Err(error)) => return Err(error.into()),
                        None => bail!("WebSocket closed before a binary packet arrived"),
                    }
                }
                Command::ProtocolTest { .. } => unreachable!(),
                Command::TestVadAsrWav { .. } => unreachable!(),
                Command::DecodeDownlinkOpus { .. } => unreachable!(),
            }
        }
    }
    Ok(())
}

fn decode_downlink_opus(file: &Path) -> anyhow::Result<()> {
    let packet = std::fs::read(file)
        .with_context(|| format!("read canonical downlink packet from {}", file.display()))?;
    let decoded = voice_reference_client::decode_canonical_downlink_opus_packet(&packet)?;
    println!("decoded canonical 24 kHz mono Opus packet: {decoded} samples");
    Ok(())
}

fn test_vad_asr_wav(file: &Path, vad_model: &Path, asr_model_dir: &Path) -> anyhow::Result<()> {
    let pcm = read_wav_as_uplink_pcm(file)?;
    let samples = pcm
        .iter()
        .map(|sample| *sample as f32 / i16::MAX as f32)
        .collect::<Vec<_>>();
    let vad_config = VadModelConfig {
        sample_rate: 16_000,
        num_threads: 1,
        provider: Some("cpu".into()),
        silero_vad: SileroVadModelConfig {
            model: Some(vad_model.display().to_string()),
            threshold: 0.5,
            min_silence_duration: 0.6,
            min_speech_duration: 0.18,
            window_size: 512,
            max_speech_duration: 30.0,
        },
        ..VadModelConfig::default()
    };
    let vad = VoiceActivityDetector::create(&vad_config, 60.0)
        .context("create Silero VAD; verify --vad-model")?;
    for chunk in samples.chunks(512) {
        vad.accept_waveform(chunk);
    }
    vad.flush();
    if vad.is_empty() {
        bail!("Silero VAD detected no speech in {}", file.display());
    }

    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder =
        Some(asr_model_dir.join("encoder.onnx").display().to_string());
    config.model_config.transducer.decoder =
        Some(asr_model_dir.join("decoder.onnx").display().to_string());
    config.model_config.transducer.joiner =
        Some(asr_model_dir.join("joiner.onnx").display().to_string());
    config.model_config.tokens = Some(asr_model_dir.join("tokens.txt").display().to_string());
    config.model_config.num_threads = 2;
    config.model_config.provider = Some("cpu".into());
    config.decoding_method = Some("greedy_search".into());
    let recognizer = OnlineRecognizer::create(&config)
        .context("create Zipformer recognizer; verify --asr-model-dir")?;
    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, &samples);
    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    let text = recognizer
        .get_result(&stream)
        .context("Zipformer returned no result")?
        .text
        .trim()
        .to_owned();
    if text.is_empty() {
        bail!(
            "Zipformer produced an empty transcript for {}",
            file.display()
        );
    }
    println!("VAD speech detected; ASR returned a non-empty final");
    Ok(())
}

async fn send_listen<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_listen_start(socket).await?;
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"stop"}).to_string().into(),
        ))
        .await?;
    Ok(())
}

async fn send_listen_start<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await?;
    Ok(())
}

async fn send_wav<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    path: &Path,
    mode: ListenModeArg,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let pcm = read_wav_as_uplink_pcm(path)?;
    if pcm.is_empty() {
        bail!("WAV input has no PCM samples");
    }
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip)
        .context("initialize uplink Opus encoder")?;
    send_listen_start_mode(socket, mode).await?;
    let mut packets = 0usize;
    for frame in pcm.chunks(960) {
        let mut canonical_frame = [0_i16; 960];
        canonical_frame[..frame.len()].copy_from_slice(frame);
        let mut packet = [0_u8; 4_000];
        let encoded = encoder
            .encode(&canonical_frame, &mut packet)
            .context("encode canonical uplink Opus frame")?;
        if encoded == 0 {
            bail!("Opus encoder produced an empty uplink packet");
        }
        socket
            .send(Message::Binary(packet[..encoded].to_vec().into()))
            .await?;
        packets += 1;
        // A captured WAV is replayed at the wire cadence of a real 60 ms microphone frame.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
    if matches!(mode, ListenModeArg::Manual) {
        socket
            .send(Message::Text(
                json!({"type":"listen","state":"stop"}).to_string().into(),
            ))
            .await?;
    } else {
        // Auto mode must endpoint from audio, never from a client stop command.
        for _ in 0..20 {
            let mut packet = [0_u8; 4_000];
            let encoded = encoder.encode(&[0_i16; 960], &mut packet)?;
            socket
                .send(Message::Binary(packet[..encoded].to_vec().into()))
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
    }
    wait_for_exactly_one_stt(socket).await?;
    println!(
        "sent {packets} canonical 60 ms Opus packets from {}",
        path.display()
    );
    Ok(())
}

async fn send_listen_start_mode<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    mode: ListenModeArg,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mode = match mode {
        ListenModeArg::Manual => "manual",
        ListenModeArg::Auto => "auto",
    };
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"start","mode": mode})
                .to_string()
                .into(),
        ))
        .await?;
    Ok(())
}

/// A single replay must yield one V1 STT final, never a duplicate transcript event.
async fn wait_for_exactly_one_stt<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut stt_count = 0;
    loop {
        let timeout_duration = if stt_count == 0 {
            std::time::Duration::from_secs(10)
        } else {
            std::time::Duration::from_millis(200)
        };
        let message = match tokio::time::timeout(timeout_duration, socket.next()).await {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(None) if stt_count == 1 => return Ok(()),
            Ok(None) => bail!("WebSocket closed before STT"),
            Err(_) if stt_count == 1 => return Ok(()),
            Err(_) => bail!("timed out waiting for STT"),
        };
        match message {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if value.get("type").and_then(|item| item.as_str()) == Some("stt") {
                    let _transcript = value
                        .get("text")
                        .and_then(|item| item.as_str())
                        .context("STT message has no text")?;
                    stt_count += 1;
                    if stt_count > 1 {
                        bail!("received more than one STT for one replay");
                    }
                }
            }
            Message::Close(frame) => bail!("WebSocket closed before STT: {frame:?}"),
            _ => {}
        }
    }
}

fn read_wav_as_uplink_pcm(path: &Path) -> anyhow::Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open WAV input {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
        || spec.channels != 1
    {
        bail!("WAV must be PCM16 mono");
    }
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .context("read PCM16 WAV samples")?;
    match spec.sample_rate {
        16_000 => Ok(samples),
        24_000 => Ok(resample_24khz_to_16khz(&samples)),
        rate => bail!("WAV sample rate must be 16000 or 24000 Hz, got {rate}"),
    }
}

fn resample_24khz_to_16khz(input: &[i16]) -> Vec<i16> {
    let output_len = input.len().saturating_mul(2) / 3;
    (0..output_len)
        .map(|index| {
            let position = index * 3;
            let lower = position / 2;
            let upper = (lower + 1).min(input.len().saturating_sub(1));
            if position.is_multiple_of(2) {
                input.get(lower).copied().unwrap_or_default()
            } else {
                ((i32::from(input[lower]) + i32::from(input[upper])) / 2) as i16
            }
        })
        .collect()
}

fn decode_hex(value: &str) -> anyhow::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        bail!("expected hex must contain an even number of characters");
    }
    let (pairs, _) = value.as_bytes().as_chunks::<2>();
    pairs
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex input is valid UTF-8");
            u8::from_str_radix(text, 16).with_context(|| format!("invalid hex byte {text:?}"))
        })
        .collect()
}

async fn send_protocol_case<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    case: &str,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match case {
        "binary-before-hello" => socket.send(Message::Binary(vec![1, 2, 3].into())).await?,
        "invalid-audio-profile" => socket.send(Message::Text(json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":48000,"channels":1,"frame_duration":60}}).to_string().into())).await?,
        other => bail!("unknown protocol case: {other}"),
    }
    print_next(socket).await
}

async fn print_next<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(message) = socket.next().await {
        println!("{:?}", message?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::ws::{Message as AxumMessage, WebSocketUpgrade},
        response::IntoResponse,
        routing::{get, post},
    };
    use tokio::net::TcpListener;

    #[test]
    fn receive_binary_requires_an_even_length_hex_fixture() {
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn receive_binary_decodes_a_raw_packet_fixture() {
        assert_eq!(decode_hex("00fF10").unwrap(), vec![0, 255, 16]);
    }

    #[test]
    fn wav_resampler_produces_canonical_16khz_sample_count() {
        assert_eq!(
            resample_24khz_to_16khz(&[0, 10, 20, 30, 40, 50]),
            vec![0, 15, 30, 45]
        );
    }

    #[test]
    fn docs_audio_wav_is_accepted_as_uplink_input() {
        let pcm = read_wav_as_uplink_pcm(Path::new("../../docs/audio.wav")).unwrap();
        assert!(!pcm.is_empty());
        assert!(pcm.chunks(960).all(|frame| frame.len() <= 960));
    }

    #[test]
    #[ignore = "requires locally downloaded Silero and Zipformer model artifacts"]
    fn docs_audio_wav_passes_local_vad_and_asr_smoke() {
        test_vad_asr_wav(
            Path::new("../../docs/audio.wav"),
            Path::new("../../models/vad/silero_vad.onnx"),
            Path::new("../../models/asr/zipformer-30m-vi"),
        )
        .unwrap();
    }

    async fn fixture_ws(upgrade: WebSocketUpgrade) -> impl IntoResponse {
        upgrade.on_upgrade(|mut socket| async move {
            let _ = socket.recv().await;
            socket
                .send(AxumMessage::Text(
                    json!({
                        "type": "hello",
                        "transport": "websocket",
                        "session_id": "fixture",
                        "audio_params": {
                            "format": "opus",
                            "sample_rate": 24000,
                            "channels": 1,
                            "frame_duration": 60
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            socket
                .send(AxumMessage::Binary(vec![0, 255, 16].into()))
                .await
                .unwrap();
        })
    }

    async fn duplicate_stt_ws(upgrade: WebSocketUpgrade) -> impl IntoResponse {
        upgrade.on_upgrade(|mut socket| async move {
            let _ = socket.recv().await;
            socket
                .send(AxumMessage::Text(
                    json!({"type": "hello", "session_id": "fixture"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            for _ in 0..2 {
                socket
                    .send(AxumMessage::Text(
                        json!({"type": "stt", "text": "private fixture transcript"})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        })
    }

    #[tokio::test]
    async fn receive_binary_accepts_the_exact_packet_from_an_ota_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let ws_url = format!("ws://{address}/voice/v1/");
        let ota_url = format!("http://{address}/voice/ota/");
        let app = Router::new()
            .route(
                "/voice/ota/",
                post(move || {
                    let ws_url = ws_url.clone();
                    async move { Json(json!({"websocket": {"url": ws_url, "token": ""}})) }
                }),
            )
            .route("/voice/v1/", get(fixture_ws));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let result = run(Args {
            ota: ota_url,
            device_id: "reference-client-01".into(),
            client_id: "reference-client".into(),
            command: Command::ReceiveBinary {
                expected_hex: "00ff10".into(),
            },
        })
        .await;

        task.abort();
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn wav_replay_rejects_a_duplicate_stt() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route("/voice/v1/", get(duplicate_stt_ws));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (mut socket, _) = connect_async(format!("ws://{address}/voice/v1/"))
            .await
            .unwrap();
        socket
            .send(Message::Text(json!({"type": "hello"}).to_string().into()))
            .await
            .unwrap();
        let _ = socket.next().await.unwrap().unwrap();

        let result = wait_for_exactly_one_stt(&mut socket).await;

        task.abort();
        assert!(result.is_err(), "a duplicate STT must fail the replay");
    }
}
