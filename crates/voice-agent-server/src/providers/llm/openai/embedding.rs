use serde::Serialize;
use serde_json::Value;

use crate::providers::llm::{LlmError, LlmOutput};

use super::provider::ConfiguredOpenAiLlm;

impl ConfiguredOpenAiLlm {
    pub(super) async fn create_embeddings(&self, inputs: Vec<String>) -> Result<LlmOutput, LlmError> {
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
