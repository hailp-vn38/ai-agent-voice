use super::{VisionError, VisionProvider, VisionRequest, VisionResponse};
use crate::config::{OpenAiVisionConfig, SecretString};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use url::Url;

pub struct OpenAiVisionProvider {
    client: reqwest::Client,
    endpoint: Url,
    api_key: SecretString,
    model: String,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
}

impl OpenAiVisionProvider {
    pub fn new(config: OpenAiVisionConfig) -> Result<Self, VisionError> {
        let mut base = config.base_url;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let endpoint = base
            .join("chat/completions")
            .map_err(|_| VisionError::InvalidRequest)?;
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint,
            api_key: config.api_key,
            model: config.model,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: config.top_p,
        })
    }
}

#[derive(Serialize)]
struct Payload {
    model: String,
    messages: [Message; 1],
    stream: bool,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
}
#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: [Content; 2],
}
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}
#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[async_trait::async_trait]
impl VisionProvider for OpenAiVisionProvider {
    fn adapter(&self) -> &'static str {
        "openai_vision"
    }
    async fn analyze(&self, request: VisionRequest) -> Result<VisionResponse, VisionError> {
        let payload = Payload {
            model: self.model.clone(),
            stream: false,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            top_p: self.top_p,
            messages: [Message {
                role: "user",
                content: [
                    Content::Text {
                        text: request.question.to_string(),
                    },
                    Content::ImageUrl {
                        image_url: ImageUrl {
                            url: format!(
                                "data:{};base64,{}",
                                request.mime_type,
                                STANDARD.encode(request.image.as_ref())
                            ),
                        },
                    },
                ],
            }],
        };
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.api_key.expose())
            .json(&payload)
            .send()
            .await
            .map_err(|_| VisionError::Transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(
                if matches!(status.as_u16(), 401 | 403 | 429) || status.is_client_error() {
                    VisionError::UpstreamRejected
                } else if matches!(status.as_u16(), 408 | 504) {
                    VisionError::Timeout
                } else {
                    VisionError::Transport
                },
            );
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| VisionError::InvalidResponse)?;
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(VisionError::InvalidResponse)?;
        Ok(VisionResponse {
            text: text.to_owned(),
        })
    }
}
