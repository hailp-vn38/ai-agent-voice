use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;

use crate::providers::llm::{LlmError, LlmOutput};

use super::provider::ConfiguredOpenAiLlm;

impl ConfiguredOpenAiLlm {
    pub(super) async fn decode_media_response(
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

    async fn try_decode_direct_media(
        &self,
        value: &Value,
        default_mime_type: &str,
    ) -> Result<Option<LlmOutput>, LlmError> {
        if value.get("error").is_some_and(|err| !err.is_null()) {
            return Err(LlmError::Rejected);
        }
        if matches!(
            value.get("status").and_then(Value::as_str),
            Some("failed" | "cancelled" | "canceled" | "expired")
        ) {
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
            return Ok(Some(LlmOutput::Media {
                bytes,
                mime_type: default_mime_type.to_owned(),
            }));
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
                return Ok(Some(LlmOutput::Media {
                    bytes,
                    mime_type: default_mime_type.to_owned(),
                }));
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
            return Ok(Some(LlmOutput::Media {
                bytes: bytes.to_vec(),
                mime_type: default_mime_type.to_owned(),
            }));
        }

        Ok(None)
    }

    async fn fetch_video_content(
        &self,
        video_id: &str,
        default_mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        let content_endpoint = self.endpoint(&format!("videos/{video_id}/content"))?;
        let response = self
            .client
            .get(content_endpoint)
            .bearer_auth(api_key)
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
        self.try_decode_direct_media(&value, default_mime_type)
            .await?
            .ok_or(LlmError::Rejected)
    }

    async fn decode_media_json(
        &self,
        value: Value,
        default_mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        if let Some(media) = self
            .try_decode_direct_media(&value, default_mime_type)
            .await?
        {
            return Ok(media);
        }

        if default_mime_type.starts_with("video/")
            && let Some(video_id) = value
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
        {
            match value.get("status").and_then(Value::as_str) {
                Some("completed" | "succeeded") => {
                    return self.fetch_video_content(video_id, default_mime_type).await;
                }
                Some("queued" | "in_progress" | "pending" | "processing" | "running") => {
                    let api_key = self.ensure_api_key()?;
                    let status_endpoint = self.endpoint(&format!("videos/{video_id}"))?;
                    let deadline =
                        tokio::time::Instant::now() + Duration::from_secs(self.timeout_seconds);
                    let mut poll_delay = Duration::from_millis(25);
                    loop {
                        let now = tokio::time::Instant::now();
                        if now >= deadline {
                            return Err(LlmError::Failed);
                        }
                        tokio::time::sleep(poll_delay.min(deadline - now)).await;
                        poll_delay = (poll_delay * 2).min(Duration::from_millis(1_000));

                        let poll_resp = self
                            .client
                            .get(status_endpoint.clone())
                            .bearer_auth(api_key)
                            .send()
                            .await
                            .map_err(|_| LlmError::Failed)?;
                        let poll_resp = Self::check_response(poll_resp).await?;
                        let poll_val: Value =
                            poll_resp.json().await.map_err(|_| LlmError::Rejected)?;
                        if let Some(media) = self
                            .try_decode_direct_media(&poll_val, default_mime_type)
                            .await?
                        {
                            return Ok(media);
                        }
                        match poll_val.get("status").and_then(Value::as_str) {
                            Some("completed" | "succeeded") => {
                                return self.fetch_video_content(video_id, default_mime_type).await;
                            }
                            Some(
                                "queued" | "in_progress" | "pending" | "processing" | "running",
                            ) => continue,
                            _ => return Err(LlmError::Rejected),
                        }
                    }
                }
                _ => {}
            }
        }

        Err(LlmError::Rejected)
    }

    pub(super) async fn generate_image(
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

    pub(super) async fn generate_video(
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
        struct VideoCreatePayload<'a> {
            model: &'a str,
            prompt: &'a str,
            size: String,
            seconds: u32,
        }
        #[derive(Serialize)]
        struct VideoGenerationFallbackPayload<'a> {
            model: &'a str,
            prompt: &'a str,
            size: String,
            seconds: u32,
            duration: u32,
        }
        let size = format!("{width}x{height}");
        let endpoint = self.endpoint("videos")?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&VideoCreatePayload {
                model: &self.model,
                prompt,
                size: size.clone(),
                seconds: duration_secs,
            })
            .send()
            .await
            .map_err(|_| LlmError::Failed)?;
        let response = if matches!(response.status().as_u16(), 404 | 405) {
            let fallback = self.endpoint("videos/generations")?;
            self.client
                .post(fallback)
                .bearer_auth(api_key)
                .json(&VideoGenerationFallbackPayload {
                    model: &self.model,
                    prompt,
                    size,
                    seconds: duration_secs,
                    duration: duration_secs,
                })
                .send()
                .await
                .map_err(|_| LlmError::Failed)?
        } else {
            response
        };
        self.decode_media_response(response, "video/mp4").await
    }
}
