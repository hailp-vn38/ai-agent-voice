use std::{
    io::Cursor,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
    meta::MetadataOptions, probe::Hint,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::super::{TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker};
use crate::{audio::PcmF32Mono, config::ChillAudioWsConfig};

pub(crate) struct ChillAudioWsProvider {
    config: ChillAudioWsConfig,
}
impl ChillAudioWsProvider {
    pub(crate) fn new(config: ChillAudioWsConfig) -> Self {
        Self { config }
    }
}
impl TtsProvider for ChillAudioWsProvider {
    fn adapter(&self) -> &'static str {
        "chillaudio_ws"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| TtsError::Failed)?;
        Ok(Box::new(ChillAudioWsWorker {
            runtime,
            config: self.config.clone(),
        }))
    }
}
struct ChillAudioWsWorker {
    runtime: tokio::runtime::Runtime,
    config: ChillAudioWsConfig,
}
impl TtsWorker for ChillAudioWsWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let audio = self
            .runtime
            .block_on(fetch_mp3(&self.config, &request.text, cancelled))?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        on_pcm(decode_mp3(audio)?)
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}
#[derive(Serialize)]
struct StartTask {
    appkey: String,
    event: &'static str,
    namespace: &'static str,
    payload: String,
    token: String,
    version: &'static str,
}
async fn fetch_mp3(
    config: &ChillAudioWsConfig,
    text: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, TtsError> {
    let operation = async {
        let (mut ws, _) = connect_async(config.ws_url.as_str())
            .await
            .map_err(|_| TtsError::RemoteConnection)?;
        let payload = serde_json::json!({"audio_config":{"bit_rate":128000,"format":"mp3","sample_rate":24000},"speaker":config.voice,"text":text});
        let task = StartTask {
            appkey: config.app_key.expose().into(),
            event: "StartTask",
            namespace: "TTS",
            payload: serde_json::to_string(&payload).map_err(|_| TtsError::Failed)?,
            token: config.token.expose().into(),
            version: "sdk_v1",
        };
        ws.send(Message::Text(
            serde_json::to_string(&task)
                .map_err(|_| TtsError::Failed)?
                .into(),
        ))
        .await
        .map_err(|_| TtsError::RemoteConnection)?;
        let mut audio = Vec::new();
        loop {
            tokio::select! {
            message = ws.next() => match message { Some(Ok(Message::Binary(bytes))) => audio.extend_from_slice(&bytes), Some(Ok(Message::Text(text))) => match serde_json::from_str::<serde_json::Value>(text.as_ref()).map_err(|_| TtsError::RemoteTask)?.get("event").and_then(serde_json::Value::as_str) { Some("TaskEnd"|"TaskFinished") => break, Some("TaskFailed") => return Err(TtsError::RemoteTask), _ => {} }, Some(Ok(Message::Ping(payload))) => ws.send(Message::Pong(payload)).await.map_err(|_| TtsError::RemoteConnection)?, Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Err(TtsError::RemoteConnection), _ => {} }, _ = tokio::time::sleep(Duration::from_millis(25)) => if cancelled.load(Ordering::Acquire) { let _ = ws.close(None).await; return Err(TtsError::Failed); } }
        }
        if audio.len() <= 100 {
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
