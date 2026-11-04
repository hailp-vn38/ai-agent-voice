use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use uuid::Uuid;

use super::super::{TtsDiagnosticRequest, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker};
use super::drm::{adjust_clock_skew_from_rfc2822_date, generate_muid, generate_sec_ms_gec};
use super::voices::{
    canonical_voice_name, is_valid_language, is_valid_voice_and_language, voice_locale,
};
use crate::{audio::PcmF32Mono, config::EdgeTtsConfig};

const EDGE_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
const EDGE_ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";

pub(crate) struct EdgeTtsProvider {
    config: EdgeTtsConfig,
}

impl EdgeTtsProvider {
    pub(crate) fn new(config: EdgeTtsConfig) -> Self {
        Self { config }
    }
}

impl TtsProvider for EdgeTtsProvider {
    fn adapter(&self) -> &'static str {
        "edge_tts"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| TtsError::Failed)?;
        Ok(Box::new(EdgeTtsWorker {
            runtime,
            config: self.config.clone(),
        }))
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        let voice = request.voice.as_deref().unwrap_or(&self.config.voice);
        let Some(inferred_locale) = voice_locale(voice) else {
            return Err(TtsError::UnsupportedVoice(voice.into()));
        };
        let language = match request.language.as_deref() {
            Some(lang) => lang,
            None => {
                if request.voice.is_some() {
                    inferred_locale
                } else {
                    &self.config.language
                }
            }
        };
        if !is_valid_language(language) || !is_valid_voice_and_language(voice, language) {
            return Err(TtsError::UnsupportedLanguage(language.into()));
        }
        Ok(())
    }
}

struct EdgeTtsWorker {
    runtime: tokio::runtime::Runtime,
    config: EdgeTtsConfig,
}

impl TtsWorker for EdgeTtsWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let mut effective = self.config.clone();
        if !request.selection.voice.is_empty() && request.selection.voice != "__readiness__" {
            effective.voice = request.selection.voice.clone();
        }
        if !request.selection.language.is_empty() && request.selection.language != "und" {
            effective.language = request.selection.language.clone();
        }
        if !is_valid_voice_and_language(&effective.voice, &effective.language) {
            if let Some(locale) = voice_locale(&effective.voice) {
                effective.language = locale.to_string();
            } else {
                return Err(TtsError::UnsupportedVoice(effective.voice));
            }
        }
        let audio = self
            .runtime
            .block_on(fetch_mp3(&effective, &request.text, cancelled))?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        on_pcm(decode_mp3(audio)?)
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

pub(crate) fn sanitize_input_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if matches!(code, 0..=8 | 11..=12 | 14..=31) {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

pub(crate) fn escape_xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn format_rate_percent(rate_percent: i16) -> String {
    format!("{rate_percent:+0}%")
}

pub(crate) fn format_pitch_hz(pitch_hz: i16) -> String {
    format!("{pitch_hz:+0}Hz")
}

pub(crate) fn build_ssml(config: &EdgeTtsConfig, raw_text: &str) -> Result<String, TtsError> {
    let canonical_voice = canonical_voice_name(&config.voice)
        .ok_or_else(|| TtsError::UnsupportedVoice(config.voice.clone()))?;
    let escaped = escape_xml_text(&sanitize_input_text(raw_text));
    let pitch = format_pitch_hz(config.pitch_hz);
    let rate = format_rate_percent(config.rate_percent);
    Ok(format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='{lang}'><voice name='{canonical_voice}'><prosody pitch='{pitch}' rate='{rate}' volume='+0%'>{escaped}</prosody></voice></speak>",
        lang = config.language
    ))
}

fn edge_timestamp() -> String {
    Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

fn build_ws_request(
    config: &EdgeTtsConfig,
    connection_id: &str,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, TtsError> {
    let mut url = config.ws_url.clone();
    let gec = generate_sec_ms_gec(&config.trusted_client_token, &config.sec_ms_gec_version);
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("TrustedClientToken", &config.trusted_client_token);
        pairs.append_pair("ConnectionId", connection_id);
        pairs.append_pair("Sec-MS-GEC", &gec);
        pairs.append_pair("Sec-MS-GEC-Version", &config.sec_ms_gec_version);
    }
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| TtsError::RemoteConnection)?;
    let headers = request.headers_mut();
    headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));
    headers.insert("Origin", HeaderValue::from_static(EDGE_ORIGIN));
    headers.insert("User-Agent", HeaderValue::from_static(EDGE_USER_AGENT));
    headers.insert(
        "Accept-Encoding",
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    let cookie = format!("muid={};", generate_muid());
    if let Ok(cookie_val) = HeaderValue::from_str(&cookie) {
        headers.insert("Cookie", cookie_val);
    }
    Ok(request)
}

async fn connect_edge_ws(
    config: &EdgeTtsConfig,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, TtsError> {
    let connection_id = Uuid::new_v4().simple().to_string();
    let request = build_ws_request(config, &connection_id)?;
    match connect_async(request).await {
        Ok((ws, _)) => Ok(ws),
        Err(WsError::Http(response)) if response.status() == StatusCode::FORBIDDEN => {
            let adjusted = response
                .headers()
                .get("Date")
                .and_then(|value| value.to_str().ok())
                .is_some_and(adjust_clock_skew_from_rfc2822_date);
            if !adjusted {
                return Err(TtsError::RemoteConnection);
            }
            let retry_id = Uuid::new_v4().simple().to_string();
            let retry_req = build_ws_request(config, &retry_id)?;
            let (ws, _) = connect_async(retry_req)
                .await
                .map_err(|_| TtsError::RemoteConnection)?;
            Ok(ws)
        }
        Err(_) => Err(TtsError::RemoteConnection),
    }
}

fn parse_header_lines(header_bytes: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(header_bytes);
    let mut headers = HashMap::new();
    for line in text.split("\r\n") {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    headers
}

async fn fetch_mp3(
    config: &EdgeTtsConfig,
    text: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, TtsError> {
    let trimmed = sanitize_input_text(text);
    if trimmed.trim().is_empty() {
        return Err(TtsError::RemoteTask);
    }
    let ssml = build_ssml(config, trimmed.trim())?;
    let operation = async {
        let mut ws = connect_edge_ws(config).await?;
        let speech_config_frame = format!(
            "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"true\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
            ts = edge_timestamp()
        );
        ws.send(Message::Text(speech_config_frame.into()))
            .await
            .map_err(|_| TtsError::RemoteConnection)?;

        let request_id = Uuid::new_v4().simple().to_string();
        let ssml_frame = format!(
            "X-RequestId:{request_id}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n{ssml}",
            ts = edge_timestamp()
        );
        ws.send(Message::Text(ssml_frame.into()))
            .await
            .map_err(|_| TtsError::RemoteConnection)?;

        let mut audio = Vec::new();
        loop {
            tokio::select! {
                message = ws.next() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() < 2 {
                            return Err(TtsError::RemoteTask);
                        }
                        let header_len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
                        if 2 + header_len > bytes.len() {
                            return Err(TtsError::RemoteTask);
                        }
                        let headers = parse_header_lines(&bytes[2..2 + header_len]);
                        let body = &bytes[2 + header_len..];
                        if headers.get("path").map(String::as_str) != Some("audio") {
                            return Err(TtsError::RemoteTask);
                        }
                        match headers.get("content-type").map(String::as_str) {
                            Some("audio/mpeg") => {
                                if body.is_empty() {
                                    return Err(TtsError::RemoteTask);
                                }
                                audio.extend_from_slice(body);
                            }
                            None => {
                                if !body.is_empty() {
                                    return Err(TtsError::RemoteTask);
                                }
                            }
                            Some(_) => return Err(TtsError::RemoteTask),
                        }
                    }
                    Some(Ok(Message::Text(frame))) => {
                        let Some((header_part, _body_part)) = frame.split_once("\r\n\r\n") else {
                            return Err(TtsError::RemoteTask);
                        };
                        let headers = parse_header_lines(header_part.as_bytes());
                        match headers.get("path").map(String::as_str) {
                            Some("turn.end") => {
                                let _ = ws.close(None).await;
                                break;
                            }
                            Some("response" | "turn.start" | "audio.metadata") => {}
                            _ => return Err(TtsError::RemoteTask),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws.send(Message::Pong(payload))
                            .await
                            .map_err(|_| TtsError::RemoteConnection)?;
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        return Err(TtsError::RemoteConnection);
                    }
                    _ => {}
                },
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if cancelled.load(Ordering::Acquire) {
                        let _ = ws.close(None).await;
                        return Err(TtsError::Failed);
                    }
                }
            }
        }
        if audio.is_empty() {
            Err(TtsError::RemoteTask)
        } else {
            Ok(audio)
        }
    };
    tokio::time::timeout(Duration::from_millis(config.timeout_ms), operation)
        .await
        .map_err(|_| TtsError::RemoteTimeout)?
}

fn decode_mp3(bytes: Vec<u8>) -> Result<PcmF32Mono, TtsError> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| TtsError::AudioDecode)?;
    let mut format = probed.format;
    let track = format.default_track().ok_or(TtsError::AudioDecode)?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(TtsError::AudioDecode)?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| TtsError::AudioDecode)?;
    let mut mono = Vec::new();
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder.decode(&packet).map_err(|_| TtsError::AudioDecode)?;
        let channels = decoded.spec().channels.count();
        if channels == 0 {
            return Err(TtsError::AudioDecode);
        }
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        samples.copy_interleaved_ref(decoded);
        for frame in samples.samples().chunks(channels) {
            let sample = frame.iter().sum::<f32>() / channels as f32;
            if !sample.is_finite() {
                return Err(TtsError::AudioDecode);
            }
            mono.push(sample);
        }
    }
    if mono.is_empty() {
        Err(TtsError::AudioDecode)
    } else {
        Ok(PcmF32Mono::new(mono, sample_rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    fn sample_config() -> EdgeTtsConfig {
        serde_json::from_value(serde_json::json!({
            "voice": "vi-VN-HoaiMyNeural",
            "language": "vi-VN",
            "rate_percent": 15,
            "pitch_hz": -5
        }))
        .unwrap()
    }

    #[test]
    fn ssml_escapes_xml_and_formats_rate_and_pitch() {
        let cfg = sample_config();
        let ssml = build_ssml(&cfg, "Xin chào 5 < 10 & 'AI' \"Voice\"\x0b!").unwrap();
        assert!(ssml.contains("xml:lang='vi-VN'"));
        assert!(
            ssml.contains(
                "name='Microsoft Server Speech Text to Speech Voice (vi-VN, HoaiMyNeural)'"
            )
        );
        assert!(ssml.contains("<prosody pitch='-5Hz' rate='+15%' volume='+0%'>"));
        assert!(ssml.contains("Xin chào 5 &lt; 10 &amp; &apos;AI&apos; &quot;Voice&quot; !"));
    }

    #[test]
    fn diagnostic_validation_auto_infers_language_for_foreign_voice_override() {
        let provider = EdgeTtsProvider::new(sample_config());
        let ok_foreign = TtsDiagnosticRequest {
            text: "Hello world".into(),
            voice: Some("en-US-EmmaMultilingualNeural".into()),
            language: None,
        };
        assert!(provider.validate_diagnostic(&ok_foreign).is_ok());

        let mismatched = TtsDiagnosticRequest {
            text: "Hello world".into(),
            voice: Some("en-US-EmmaMultilingualNeural".into()),
            language: Some("vi-VN".into()),
        };
        assert!(matches!(
            provider.validate_diagnostic(&mismatched),
            Err(TtsError::UnsupportedLanguage(_))
        ));
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn connect_edge_ws_retries_once_on_403_with_date_header_and_fetches_audio() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_attempts = Arc::clone(&attempts);

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let step = server_attempts.fetch_add(1, Ordering::SeqCst);
                if step == 0 {
                    let _ = accept_hdr_async(stream, |_req: &Request, _resp: Response| {
                        let mut err = ErrorResponse::new(None);
                        *err.status_mut() = StatusCode::FORBIDDEN;
                        err.headers_mut().insert(
                            "Date",
                            HeaderValue::from_static("Sun, 25 Oct 2026 14:00:00 GMT"),
                        );
                        Err(err)
                    })
                    .await;
                } else {
                    let mut ws =
                        accept_hdr_async(stream, |_req: &Request, resp: Response| Ok(resp))
                            .await
                            .unwrap();
                    let _cfg_msg = ws.next().await.unwrap().unwrap();
                    let _ssml_msg = ws.next().await.unwrap().unwrap();

                    let headers = b"X-RequestId:abc\r\nContent-Type:audio/mpeg\r\nPath:audio\r\n";
                    let header_len = (headers.len() as u16).to_be_bytes();
                    let mut bin_frame = Vec::new();
                    bin_frame.extend_from_slice(&header_len);
                    bin_frame.extend_from_slice(headers);
                    bin_frame.extend_from_slice(b"FAKE_MP3_PAYLOAD");
                    ws.send(Message::Binary(bin_frame.into())).await.unwrap();

                    let turn_end = "X-RequestId:abc\r\nContent-Type:application/json; charset=utf-8\r\nPath:turn.end\r\n\r\n{}";
                    ws.send(Message::Text(turn_end.into())).await.unwrap();
                }
            }
        });

        let mut cfg = sample_config();
        cfg.ws_url = format!("ws://{addr}/edge/v1").parse().unwrap();
        let cancelled = AtomicBool::new(false);
        let mp3 = fetch_mp3(&cfg, "Xin chào", &cancelled).await.unwrap();
        assert_eq!(mp3, b"FAKE_MP3_PAYLOAD");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        server.await.unwrap();
    }
}
