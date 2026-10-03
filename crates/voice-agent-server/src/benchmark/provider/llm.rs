use std::time::Instant;

use futures_util::StreamExt;

use crate::providers::llm::LlmRequest;
use crate::providers::{LlmEvent, LlmProvider};

#[derive(Clone, Debug, serde::Serialize)]
pub struct LlmRunMetrics {
    pub ttft_ms: Option<f64>,
    pub total_ms: f64,
    pub text_delta_count: usize,
    pub output_chars: usize,
    pub tool_call_count: usize,
}

pub async fn run_llm_provider(
    provider: &dyn LlmProvider,
    request: LlmRequest,
) -> Result<LlmRunMetrics, crate::providers::LlmError> {
    let started = Instant::now();
    let mut stream = provider.stream(request).await?;
    let mut ttft_ms = None;
    let mut text_delta_count = 0;
    let mut output_chars = 0;
    let mut tool_call_count = 0;
    let mut finished = false;
    while let Some(event) = stream.next().await {
        match event? {
            LlmEvent::TextDelta(text) => {
                text_delta_count += 1;
                output_chars += text.chars().count();
                if !text.trim().is_empty() {
                    ttft_ms.get_or_insert_with(|| elapsed_ms(started));
                }
            }
            LlmEvent::ToolCall(_) => tool_call_count += 1,
            LlmEvent::Finished => {
                finished = true;
                break;
            }
        }
    }
    if !finished {
        return Err(crate::providers::LlmError::Failed);
    }
    Ok(LlmRunMetrics {
        ttft_ms,
        total_ms: elapsed_ms(started),
        text_delta_count,
        output_chars,
        tool_call_count,
    })
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
