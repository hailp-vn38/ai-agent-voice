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
    ProtocolTest {
        #[arg(long)]
        case: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
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
                Command::ProtocolTest { .. } => unreachable!(),
            }
        }
    }
    Ok(())
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
