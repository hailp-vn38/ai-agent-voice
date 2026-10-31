use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::super::noise::{CipherState, NoiseXXInitiator};
use super::super::proto::{
    DecodedServiceFrame, NoiseFrameDecoder, decode_service_response_frame,
};
use super::types::VmError;

/// Internal seam allowing replacement of the live WebSocket Noise transport with `FakeMuseVmTransport` in tests.
#[async_trait]
pub(crate) trait MuseVmTransport: Send + Sync {
    async fn connect(
        &self,
        wss_url: &str,
    ) -> Result<(Box<dyn MuseVmSender>, Box<dyn MuseVmReceiver>), VmError>;
}

#[async_trait]
pub(crate) trait MuseVmSender: Send {
    /// Encrypts and sends a sequence of fragmented `NoiseTransportFrame` payloads in strict nonce order.
    async fn send_frames(&mut self, plaintext_frames: Vec<Vec<u8>>) -> Result<(), VmError>;
    async fn send_ping(&mut self) -> Result<(), VmError> {
        Ok(())
    }
    async fn close(&mut self);
}

#[async_trait]
pub(crate) trait MuseVmReceiver: Send {
    /// Receives, decrypts, and reassembles a complete `DecodedServiceFrame`.
    async fn recv_frame(&mut self) -> Result<Option<DecodedServiceFrame>, VmError>;
}

/// Production WebSocket + `Noise_XX_25519_AESGCM_SHA256` transport to `wss://hatch.metaaivm.com/v1/noise`.
pub(crate) struct WsNoiseVmTransport;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct WsNoiseVmSender {
    sink: futures_util::stream::SplitSink<WsStream, Message>,
    send_cipher: CipherState,
}

#[async_trait]
impl MuseVmSender for WsNoiseVmSender {
    async fn send_frames(&mut self, plaintext_frames: Vec<Vec<u8>>) -> Result<(), VmError> {
        for frame in plaintext_frames {
            let encrypted = self
                .send_cipher
                .encrypt_with_ad(&[], &frame)
                .map_err(VmError::Transport)?;
            self.sink
                .send(Message::Binary(encrypted.into()))
                .await
                .map_err(|e| VmError::Transport(format!("websocket send failed: {e}")))?;
        }
        Ok(())
    }

    async fn send_ping(&mut self) -> Result<(), VmError> {
        self.sink
            .send(Message::Ping(Vec::new().into()))
            .await
            .map_err(|e| VmError::Transport(format!("websocket ping failed: {e}")))
    }

    async fn close(&mut self) {
        let _ = self.sink.close().await;
    }
}

struct WsNoiseVmReceiver {
    stream: futures_util::stream::SplitStream<WsStream>,
    recv_cipher: CipherState,
    decoder: NoiseFrameDecoder,
}

#[async_trait]
impl MuseVmReceiver for WsNoiseVmReceiver {
    async fn recv_frame(&mut self) -> Result<Option<DecodedServiceFrame>, VmError> {
        while let Some(msg_res) = self.stream.next().await {
            let msg = msg_res
                .map_err(|e| VmError::Transport(format!("websocket receive failed: {e}")))?;
            match msg {
                Message::Binary(ciphertext) => {
                    let plaintext = self
                        .recv_cipher
                        .decrypt_with_ad(&[], ciphertext.as_ref())
                        .map_err(VmError::Transport)?;
                    if let Some(assembled) =
                        self.decoder.push(&plaintext).map_err(VmError::Transport)?
                        && let Some(decoded) =
                            decode_service_response_frame(&assembled).map_err(VmError::Transport)?
                    {
                        return Ok(Some(decoded));
                    }
                }
                Message::Close(close_frame) => {
                    if let Some(cf) = close_frame {
                        let code: u16 = cf.code.into();
                        if code == 1008 || code == 4001 || code == 4003 {
                            return Err(VmError::AuthInvalid(format!(
                                "Hatch VM closed WebSocket with auth code {code}: {}",
                                cf.reason
                            )));
                        }
                    }
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl MuseVmTransport for WsNoiseVmTransport {
    async fn connect(
        &self,
        wss_url: &str,
    ) -> Result<(Box<dyn MuseVmSender>, Box<dyn MuseVmReceiver>), VmError> {
        let mut req = wss_url
            .into_client_request()
            .map_err(|e| VmError::Transport(format!("invalid wss url: {e}")))?;
        req.headers_mut()
            .insert("Origin", HeaderValue::from_static("https://muse.ai"));
        req.headers_mut().insert(
            "User-Agent",
            HeaderValue::from_static(super::super::credentials::DEFAULT_USER_AGENT),
        );

        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(64 * 1024 * 1024);
        ws_config.max_frame_size = Some(64 * 1024 * 1024);

        let (mut ws_stream, _) = tokio::time::timeout(
            Duration::from_secs(20),
            connect_async_with_config(req, Some(ws_config), false),
        )
        .await
        .map_err(|_| VmError::Timeout("Hatch VM WebSocket connect timed out".to_string()))?
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("401") || msg.contains("403") {
                VmError::AuthInvalid(format!("Hatch VM rejected WebSocket upgrade: {msg}"))
            } else {
                VmError::Transport(format!("Hatch VM WebSocket connect failed: {msg}"))
            }
        })?;

        let mut initiator = NoiseXXInitiator::new(None).map_err(VmError::Transport)?;
        let msg1 = initiator.write_message_1().map_err(VmError::Transport)?;
        ws_stream
            .send(Message::Binary(msg1.into()))
            .await
            .map_err(|e| VmError::Transport(format!("failed sending Noise message 1: {e}")))?;

        let msg2 = tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(frame_res) = ws_stream.next().await {
                match frame_res {
                    Ok(Message::Binary(bin)) => return Ok(bin.to_vec()),
                    Ok(Message::Close(cf)) => {
                        let detail = cf
                            .map(|c| format!("code={}, reason={}", u16::from(c.code), c.reason))
                            .unwrap_or_else(|| "no close frame".to_string());
                        return Err(VmError::AuthInvalid(format!(
                            "WebSocket closed during Noise handshake: {detail}"
                        )));
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        return Err(VmError::Transport(format!(
                            "WebSocket error during Noise handshake: {e}"
                        )));
                    }
                }
            }
            Err(VmError::Transport(
                "WebSocket EOF before Noise message 2".to_string(),
            ))
        })
        .await
        .map_err(|_| VmError::Timeout("Noise XX handshake timed out".to_string()))??;

        initiator
            .read_message_2(&msg2)
            .map_err(VmError::AuthInvalid)?;
        let (msg3, send_cipher, recv_cipher) =
            initiator.write_message_3().map_err(VmError::AuthInvalid)?;

        ws_stream
            .send(Message::Binary(msg3.into()))
            .await
            .map_err(|e| VmError::Transport(format!("failed sending Noise message 3: {e}")))?;

        let (sink, stream) = ws_stream.split();
        Ok((
            Box::new(WsNoiseVmSender { sink, send_cipher }),
            Box::new(WsNoiseVmReceiver {
                stream,
                recv_cipher,
                decoder: NoiseFrameDecoder::default(),
            }),
        ))
    }
}
