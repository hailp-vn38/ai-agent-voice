use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use futures_util::StreamExt;

use super::config::VieneuHttpConfig;
use crate::{
    audio::PcmF32Mono,
    providers::tts::{TtsDiagnosticRequest, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker},
};

pub(crate) struct VieneuHttpProvider {
    config: VieneuHttpConfig,
}

impl VieneuHttpProvider {
    pub(crate) fn new(config: VieneuHttpConfig) -> Self {
        Self { config }
    }
}

impl TtsProvider for VieneuHttpProvider {
    fn adapter(&self) -> &'static str {
        "vieneu_http"
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        if request.voice.as_ref().is_some_and(|voice| {
            voice.trim().is_empty() || voice.len() > 128 || voice.chars().any(char::is_control)
        }) || request
            .language
            .as_ref()
            .is_some_and(|language| language != &self.config.language)
        {
            Err(TtsError::DiagnosticOverrideUnsupported)
        } else {
            Ok(())
        }
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| TtsError::Failed)?;
        // A redirect must not move text or credentials to a different service.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // The worker runtime pauses between requests and cannot observe idle socket closes.
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| TtsError::RemoteConnection)?;
        Ok(Box::new(VieneuHttpWorker {
            runtime,
            client,
            config: self.config.clone(),
        }))
    }
}

struct VieneuHttpWorker {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    config: VieneuHttpConfig,
}

impl TtsWorker for VieneuHttpWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        self.runtime.block_on(async {
            let operation = fetch_pcm(&self.client, &self.config, &request.text, cancelled, on_pcm);
            let cancellation = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    if cancelled.load(Ordering::Acquire) { break; }
                }
            };
            tokio::select! {
                _ = cancellation => Err(TtsError::Failed),
                result = tokio::time::timeout(Duration::from_millis(self.config.timeout_ms), operation) =>
                    result.map_err(|_| TtsError::RemoteTimeout)?,
            }
        })
    }

    fn synthesize_diagnostic(
        &mut self,
        request: &TtsSynthesisRequest,
        diagnostic: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let original = self.config.voice.clone();
        if let Some(voice) = &diagnostic.voice {
            self.config.voice.clone_from(voice);
        }
        let result = self.synthesize(request, cancelled, on_pcm);
        self.config.voice = original;
        result
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

async fn fetch_pcm(
    client: &reqwest::Client,
    config: &VieneuHttpConfig,
    text: &str,
    cancelled: &AtomicBool,
    on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
) -> Result<(), TtsError> {
    let deadline = Instant::now() + Duration::from_millis(config.timeout_ms);
    let endpoint = format!(
        "{}/audio/speech",
        config.base_url.as_str().trim_end_matches('/')
    );
    let mut request = client.post(endpoint).json(&serde_json::json!({
        "model": "vieneu-v3-turbo", "input": text, "voice": config.voice,
        "response_format": "pcm", "stream_format": "audio", "sample_rate": 24_000,
    }));
    if !config.api_key.expose().is_empty() {
        request = request.bearer_auth(config.api_key.expose());
    }
    let response = request
        .send()
        .await
        .map_err(|_| TtsError::RemoteConnection)?;
    if !response.status().is_success() {
        return Err(TtsError::RemoteTask);
    }
    if response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next().map(str::trim) != Some("audio/pcm"))
        || response
            .headers()
            .get("x-sample-rate")
            .and_then(|v| v.to_str().ok())
            != Some("24000")
    {
        return Err(TtsError::AudioDecode);
    }

    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    let mut total_bytes = 0usize;
    while let Some(chunk) = stream.next().await {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        if Instant::now() >= deadline {
            return Err(TtsError::RemoteTimeout);
        }
        let chunk = chunk.map_err(|_| TtsError::RemoteConnection)?;
        total_bytes = total_bytes
            .checked_add(chunk.len())
            .ok_or(TtsError::AudioDecode)?;
        // Bound untrusted output to two minutes of mono PCM16 at 24 kHz.
        if total_bytes > 24_000 * 2 * 120 {
            return Err(TtsError::AudioDecode);
        }
        pending.extend_from_slice(&chunk);
        let (pairs, remainder) = pending.as_chunks::<2>();
        let samples: Vec<f32> = pairs
            .iter()
            .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
            .collect();
        let remainder = remainder.first().copied();
        pending.clear();
        if let Some(byte) = remainder {
            pending.push(byte);
        }
        if !samples.is_empty() {
            on_pcm(PcmF32Mono::new(samples, 24_000))?;
        }
    }
    if total_bytes == 0 || !pending.is_empty() {
        return Err(TtsError::AudioDecode);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(TtsError::Failed);
    }
    // PCM delivery is synchronous; a blocked consumer can outlast Tokio's timer poll.
    if Instant::now() >= deadline {
        return Err(TtsError::RemoteTimeout);
    }
    Ok(())
}
