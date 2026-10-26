use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;
use url::Url;

#[cfg(test)]
use crate::config::SecretString;
use crate::providers::{
    llm::{
        ChatMessage, LlmCapabilityTag, LlmError, LlmEventStream, LlmOperation, LlmOutput,
        LlmProvider, LlmRequest, collect_chat_stream, effective_capability_tags,
        openai::config::OpenAiConfig, stream_openai_request, valid_generation_bounds,
        valid_llm_tags,
    },
    vision::{OpenAiVisionCompletion, VisionError, execute_openai_vision_completion},
};

pub(crate) struct ConfiguredOpenAiLlm {
    api_key: String,
    base_url: Url,
    base_url_str: String,
    model: String,
    timeout_seconds: u64,
    max_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    temperature_percent: Option<u16>,
    tags: Vec<LlmCapabilityTag>,
    client: reqwest::Client,
}

impl ConfiguredOpenAiLlm {
    pub(crate) fn from_config(config: &OpenAiConfig) -> Result<Self, LlmError> {
        if config.model.trim().is_empty()
            || config.timeout_ms == 0
            || !valid_generation_bounds(
                config.max_tokens,
                config.context_limit_tokens,
                config.reasoning_effort.as_deref(),
                config.temperature_percent,
            )
            || !valid_llm_tags(config.tags.as_deref())
        {
            return Err(LlmError::Failed);
        }
        let mut base_url = config.base_url.clone();
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let timeout_seconds = config.timeout_ms.div_ceil(1_000).max(1);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| LlmError::Failed)?;
        Ok(Self {
            api_key: config.api_key.expose().to_owned(),
            base_url_str: config.base_url.as_str().to_owned(),
            base_url,
            model: config.model.clone(),
            timeout_seconds,
            max_tokens: config.max_tokens,
            reasoning_effort: config.reasoning_effort.clone(),
            temperature_percent: config.temperature_percent,
            tags: effective_capability_tags(config.tags.as_deref()),
            client,
        })
    }

    #[cfg(test)]
    pub(crate) fn build(
        api_key: &str,
        base_url: &str,
        model: &str,
        timeout_seconds: u64,
    ) -> Result<Self, LlmError> {
        if model.trim().is_empty() || timeout_seconds == 0 {
            return Err(LlmError::Failed);
        }
        let parsed_url = Url::parse(base_url).map_err(|_| LlmError::Failed)?;
        Self::from_config(&OpenAiConfig {
            api_key: SecretString::new(api_key),
            base_url: parsed_url,
            model: model.to_owned(),
            timeout_ms: timeout_seconds.saturating_mul(1_000),
            ..OpenAiConfig::default()
        })
    }

    fn temperature(&self) -> Option<f32> {
        self.temperature_percent.map(|pct| f32::from(pct) / 100.0)
    }

    fn resolved_reasoning_effort(
        &self,
        force_reasoning: bool,
    ) -> Option<llm::chat::ReasoningEffort> {
        match self.reasoning_effort.as_deref() {
            Some("low") => Some(llm::chat::ReasoningEffort::Low),
            Some("medium") => Some(llm::chat::ReasoningEffort::Medium),
            Some("high") => Some(llm::chat::ReasoningEffort::High),
            _ if force_reasoning || self.tags.contains(&LlmCapabilityTag::Reasoning) => {
                Some(llm::chat::ReasoningEffort::Medium)
            }
            _ => None,
        }
    }

    fn endpoint(&self, relative: &str) -> Result<Url, LlmError> {
        self.base_url
            .join(relative.trim_start_matches('/'))
            .map_err(|_| LlmError::Failed)
    }

    fn ensure_api_key(&self) -> Result<&str, LlmError> {
        if self.api_key.trim().is_empty() {
            Err(LlmError::Failed)
        } else {
            Ok(&self.api_key)
        }
    }

    async fn open_chat_stream(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmEventStream, LlmError> {
        let system = request.messages.iter().find_map(|message| match message {
            ChatMessage::System { content } => Some(content.as_str()),
            _ => None,
        });
        let mut builder = llm::builder::LLMBuilder::new()
            .backend(llm::builder::LLMBackend::OpenAI)
            .api_key(&self.api_key)
            .base_url(&self.base_url_str)
            .model(&self.model)
            .timeout_seconds(self.timeout_seconds);
        if let Some(system) = system {
            builder = builder.system(system);
        }
        if let Some(max_tokens) = self.max_tokens {
            builder = builder.max_tokens(max_tokens);
        }
        if let Some(temperature) = self.temperature() {
            builder = builder.temperature(temperature);
        }
        if let Some(effort) = self.resolved_reasoning_effort(force_reasoning) {
            builder = builder.reasoning(true).reasoning_effort(effort);
        }
        let client = builder.build().map_err(|_| LlmError::Failed)?;
        stream_openai_request(client.as_ref(), request, self.adapter()).await
    }

    async fn complete_chat(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmOutput, LlmError> {
        let stream = self.open_chat_stream(request, force_reasoning).await?;
        collect_chat_stream(stream).await
    }

    async fn check_response(
        response: reqwest::Response,
    ) -> Result<reqwest::Response, LlmError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if matches!(status.as_u16(), 408 | 504) {
            Err(LlmError::Failed)
        } else {
            Err(LlmError::Rejected)
        }
    }

    async fn decode_media_response(
        &self,
        response: reqwest::Response,
        default_mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        let response = Self::check_response(response).await?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !content_type.is_empty() && !content_type.contains("json") {
            let bytes = response.bytes().await.map_err(|_| LlmError::Failed)?;
            if bytes.is_empty() {
                return Err(LlmError::Rejected);
            }
            return Ok(LlmOutput::Media {
                bytes: bytes.to_vec(),
                mime_type: default_mime_type.to_owned(),
            });
        }

        let value: Value = response.json().await.map_err(|_| LlmError::Rejected)?;
        self.decode_media_json(value, default_mime_type).await
    }

    async fn decode_media_json(
        &self,
        value: Value,
        default_mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        if value.get("error").is_some_and(|err| !err.is_null()) {
            return Err(LlmError::Rejected);
        }
        if let Some(b64) = value
            .pointer("/data/0/b64_json")
            .or_else(|| value.get("b64_json"))
            .or_else(|| value.get("audio"))
            .and_then(Value::as_str)
        {
            let bytes = STANDARD.decode(b64.trim()).map_err(|_| LlmError::Rejected)?;
            if bytes.is_empty() {
                return Err(LlmError::Rejected);
            }
            return Ok(LlmOutput::Media {
                bytes,
                mime_type: default_mime_type.to_owned(),
            });
        }

        if let Some(url) = value
            .pointer("/data/0/url")
            .or_else(|| value.get("url"))
            .and_then(Value::as_str)
        {
            if let Some(rest) = url.strip_prefix("data:") {
                let (_meta, b64_part) = rest.split_once(',').ok_or(LlmError::Rejected)?;
                let bytes = STANDARD
                    .decode(b64_part.trim())
                    .map_err(|_| LlmError::Rejected)?;
                if bytes.is_empty() {
                    return Err(LlmError::Rejected);
                }
                return Ok(LlmOutput::Media {
                    bytes,
                    mime_type: default_mime_type.to_owned(),
                });
            }
            let media_resp = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|_| LlmError::Failed)?;
            let media_resp = Self::check_response(media_resp).await?;
            let bytes = media_resp.bytes().await.map_err(|_| LlmError::Failed)?;
            if bytes.is_empty() {
                return Err(LlmError::Rejected);
            }
            return Ok(LlmOutput::Media {
                bytes: bytes.to_vec(),
                mime_type: default_mime_type.to_owned(),
            });
        }

        if default_mime_type.starts_with("video/")
            && value.get("status").and_then(Value::as_str) == Some("completed")
            && let Some(video_id) = value.get("id").and_then(Value::as_str)
            && !video_id.trim().is_empty()
        {
            let api_key = self.ensure_api_key()?;
            let content_endpoint = self.endpoint(&format!("videos/{video_id}/content"))?;
            let media_resp = self
                .client
                .get(content_endpoint)
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|_| LlmError::Failed)?;
            let media_resp = Self::check_response(media_resp).await?;
            let bytes = media_resp.bytes().await.map_err(|_| LlmError::Failed)?;
            if bytes.is_empty() {
                return Err(LlmError::Rejected);
            }
            return Ok(LlmOutput::Media {
                bytes: bytes.to_vec(),
                mime_type: default_mime_type.to_owned(),
            });
        }

        Err(LlmError::Rejected)
    }

    async fn generate_image(
        &self,
        prompt: &str,
        width: u32,
        height: u32,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        if prompt.trim().is_empty() || width == 0 || height == 0 {
            return Err(LlmError::Rejected);
        }
        #[derive(Serialize)]
        struct ImagePayload<'a> {
            model: &'a str,
            prompt: &'a str,
            n: u8,
            size: String,
        }
        let endpoint = self.endpoint("images/generations")?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&ImagePayload {
                model: &self.model,
                prompt,
                n: 1,
                size: format!("{width}x{height}"),
            })
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        self.decode_media_response(response, "image/png").await
    }

    async fn generate_video(
        &self,
        prompt: &str,
        width: u32,
        height: u32,
        duration_secs: u32,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        if prompt.trim().is_empty() || width == 0 || height == 0 || duration_secs == 0 {
            return Err(LlmError::Rejected);
        }
        #[derive(Serialize)]
        struct VideoPayload<'a> {
            model: &'a str,
            prompt: &'a str,
            size: String,
            seconds: u32,
            duration: u32,
        }
        let payload = VideoPayload {
            model: &self.model,
            prompt,
            size: format!("{width}x{height}"),
            seconds: duration_secs,
            duration: duration_secs,
        };
        let mut endpoint = self.endpoint("videos")?;
        endpoint.set_query(Some("sync=true"));
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        let response = if matches!(response.status().as_u16(), 404 | 405) {
            let fallback = self.endpoint("videos/generations")?;
            self.client
                .post(fallback)
                .bearer_auth(api_key)
                .json(&payload)
                .send()
                .await
                .map_err(|_| LlmError::Failed)?
        } else {
            response
        };
        self.decode_media_response(response, "video/mp4").await
    }

    async fn synthesize_speech(
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
        Ok(LlmOutput::Media {
            bytes: bytes.to_vec(),
            mime_type: "audio/wav".to_owned(),
        })
    }

    async fn transcribe_audio(
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

    async fn create_embeddings(&self, inputs: Vec<String>) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        if inputs.is_empty() || inputs.iter().any(|s| s.trim().is_empty()) {
            return Err(LlmError::Rejected);
        }
        #[derive(Serialize)]
        struct EmbeddingPayload<'a> {
            model: &'a str,
            input: Vec<String>,
            encoding_format: &'static str,
        }
        let expected_len = inputs.len();
        let endpoint = self.endpoint("embeddings")?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&EmbeddingPayload {
                model: &self.model,
                input: inputs,
                encoding_format: "float",
            })
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        let response = Self::check_response(response).await?;
        let value: Value = response.json().await.map_err(|_| LlmError::Rejected)?;
        let items = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or(LlmError::Rejected)?;
        if items.len() != expected_len {
            return Err(LlmError::Rejected);
        }
        let mut vectors = Vec::with_capacity(items.len());
        let mut dimensions = 0usize;
        for item in items {
            let arr = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or(LlmError::Rejected)?;
            if arr.is_empty() {
                return Err(LlmError::Rejected);
            }
            if dimensions == 0 {
                dimensions = arr.len();
            } else if arr.len() != dimensions {
                return Err(LlmError::Rejected);
            }
            let mut vec = Vec::with_capacity(arr.len());
            for num in arr {
                let val = num.as_f64().ok_or(LlmError::Rejected)? as f32;
                if !val.is_finite() {
                    return Err(LlmError::Rejected);
                }
                vec.push(val);
            }
            vectors.push(vec);
        }
        Ok(LlmOutput::Embeddings {
            vectors,
            dimensions,
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfiguredOpenAiLlm {
    fn supports_tools(&self) -> bool {
        self.supports_tag(LlmCapabilityTag::Chat)
    }

    fn adapter(&self) -> &'static str {
        "openai"
    }

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &self.tags
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Chat)?;
        self.open_chat_stream(request, false).await
    }

    async fn execute_operation(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        match op {
            LlmOperation::Chat { request, reasoning } => {
                self.complete_chat(request, reasoning).await
            }
            LlmOperation::Vision {
                prompt,
                image_bytes,
                mime_type,
            } => {
                let api_key = self.ensure_api_key()?;
                let endpoint = self.endpoint("chat/completions")?;
                let text = execute_openai_vision_completion(OpenAiVisionCompletion {
                    client: &self.client,
                    endpoint: &endpoint,
                    api_key,
                    model: &self.model,
                    question: &prompt,
                    image_bytes: &image_bytes,
                    mime_type: &mime_type,
                    max_tokens: self.max_tokens,
                    temperature: self.temperature(),
                    top_p: None,
                })
                .await
                .map_err(|err| match err {
                    VisionError::InvalidRequest
                    | VisionError::UpstreamRejected
                    | VisionError::InvalidResponse => LlmError::Rejected,
                    VisionError::Transport | VisionError::Timeout | VisionError::Overloaded => {
                        LlmError::Failed
                    }
                })?;
                Ok(LlmOutput::Text(text))
            }
            LlmOperation::ImageGeneration {
                prompt,
                width,
                height,
            } => self.generate_image(&prompt, width, height).await,
            LlmOperation::VideoGeneration {
                prompt,
                width,
                height,
                duration_secs,
            } => {
                self.generate_video(&prompt, width, height, duration_secs)
                    .await
            }
            LlmOperation::AudioSpeech { text, voice, speed } => {
                self.synthesize_speech(&text, voice.as_deref(), speed).await
            }
            LlmOperation::AudioTranscription {
                wav_bytes,
                language,
            } => self.transcribe_audio(wav_bytes, language.as_deref()).await,
            LlmOperation::Embedding { inputs } => self.create_embeddings(inputs).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::{Multipart, State},
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };

    use super::*;
    use crate::providers::llm::LLM_CAPABILITY_TAGS;

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
            })
            .await
            .unwrap();
        assert_eq!(
            image_out,
            LlmOutput::Media {
                bytes: b"FAKE_PNG_IMAGE".to_vec(),
                mime_type: "image/png".into(),
            }
        );

        // 4. Video Generation
        let video_out = provider
            .invoke(LlmOperation::VideoGeneration {
                prompt: "Waves crashing".into(),
                width: 1280,
                height: 720,
                duration_secs: 5,
            })
            .await
            .unwrap();
        assert_eq!(
            video_out,
            LlmOutput::Media {
                bytes: b"FAKE_MP4_VIDEO".to_vec(),
                mime_type: "video/mp4".into(),
            }
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
            LlmOutput::Media {
                bytes: b"RIFF_FAKE_WAV".to_vec(),
                mime_type: "audio/wav".into(),
            }
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
}
