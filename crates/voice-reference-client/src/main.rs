use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
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
    Handshake,
    Listen,
    SendOpus {
        file: std::path::PathBuf,
    },
    /// Complete hello, then require one raw binary packet matching this hex fixture.
    ReceiveBinary {
        #[arg(long)]
        expected_hex: String,
    },
    ProtocolTest {
        #[arg(long)]
        case: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}

async fn run(args: Args) -> anyhow::Result<()> {
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
            socket.send(Message::Text(hello.to_string())).await?;
            print_next(&mut socket).await?;
            match command {
                Command::Handshake => {}
                Command::Listen => {
                    socket
                        .send(Message::Text(
                            json!({"type":"listen","state":"start"}).to_string(),
                        ))
                        .await?;
                    socket
                        .send(Message::Text(
                            json!({"type":"listen","state":"stop"}).to_string(),
                        ))
                        .await?;
                }
                Command::SendOpus { file } => {
                    socket
                        .send(Message::Text(
                            json!({"type":"listen","state":"start"}).to_string(),
                        ))
                        .await?;
                    socket
                        .send(Message::Binary(tokio::fs::read(file).await?))
                        .await?;
                }
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
            }
        }
    }
    Ok(())
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
        "binary-before-hello" => socket.send(Message::Binary(vec![1, 2, 3])).await?,
        "invalid-audio-profile" => socket.send(Message::Text(json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":48000,"channels":1,"frame_duration":60}}).to_string())).await?,
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
        extract::ws::{Message as AxumMessage, WebSocketUpgrade},
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
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
                    .to_string(),
                ))
                .await
                .unwrap();
            socket
                .send(AxumMessage::Binary(vec![0, 255, 16]))
                .await
                .unwrap();
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
}
