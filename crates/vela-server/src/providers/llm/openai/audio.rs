use serde::Serialize;
use serde_json::Value;

use crate::providers::llm::{LlmError, LlmOutput};

use super::provider::ConfiguredOpenAiLlm;

impl ConfiguredOpenAiLlm {
    pub(super) async fn synthesize_speech(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        if text.trim().is_empty() {
            return Err(LlmError::Rejected);
        }
        #[derive(Serialize)]
        struct SpeechPayload<'a> {
            model: &'a str,
            input: &'a str,
            voice: &'a str,
            response_format: &'static str,
            speed: f32,
        }
        let endpoint = self.endpoint("audio/speech")?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&SpeechPayload {
                model: &self.model,
                input: text,
                voice: voice.filter(|v| !v.trim().is_empty()).unwrap_or("alloy"),
                response_format: "wav",
                speed: speed.unwrap_or(1.0).clamp(0.25, 4.0),
            })
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        let response = Self::check_response(response).await?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if content_type.contains("json") {
            return self.decode_media_response(response, "audio/wav").await;
        }
        let bytes = response.bytes().await.map_err(|_| LlmError::Failed)?;
        if bytes.is_empty() {
            return Err(LlmError::Rejected);
        }
        Ok(LlmOutput::single_media(bytes.to_vec(), "audio/wav"))
    }

    pub(super) async fn transcribe_audio(
        &self,
        wav_bytes: Vec<u8>,
        language: Option<&str>,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        if wav_bytes.is_empty() {
            return Err(LlmError::Rejected);
        }
        let part = reqwest::multipart::Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|_| LlmError::Failed)?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", part);
        if let Some(lang) = language.filter(|l| !l.trim().is_empty()) {
            form = form.text("language", lang.to_owned());
        }
        let endpoint = self.endpoint("audio/transcriptions")?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        let response = Self::check_response(response).await?;
        let body = response.text().await.map_err(|_| LlmError::Failed)?;
        let transcript = if let Ok(json) = serde_json::from_str::<Value>(&body) {
            json.get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(LlmError::Rejected)?
                .to_owned()
        } else {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                return Err(LlmError::Rejected);
            }
            trimmed.to_owned()
        };
        Ok(LlmOutput::Text(transcript))
    }
}
