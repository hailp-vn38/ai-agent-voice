use std::collections::VecDeque;

use ring::rand::{SecureRandom, SystemRandom};

use super::TtsError;

pub(super) fn sample(logits: &[f32], history: &VecDeque<i32>) -> Result<i32, TtsError> {
    let mut bytes = [0; 4];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| TtsError::Failed)?;
    let draw = (u32::from_le_bytes(bytes) >> 8) as f32 / (1_u32 << 24) as f32;
    sample_with_draw(logits, history, draw)
}

fn sample_with_draw(logits: &[f32], history: &VecDeque<i32>, draw: f32) -> Result<i32, TtsError> {
    if logits.is_empty() || logits.iter().any(|value| !value.is_finite()) {
        return Err(TtsError::Failed);
    }
    let mut candidates = logits
        .iter()
        .copied()
        .enumerate()
        .map(|(code, mut value)| {
            if history.contains(&(code as i32)) {
                value = if value < 0.0 {
                    value * 1.2
                } else {
                    value / 1.2
                };
            }
            (code as i32, value / 0.8)
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    candidates.truncate(25);
    let max = candidates[0].1;
    let total = candidates
        .iter()
        .map(|(_, value)| (value - max).exp())
        .sum::<f32>();
    let mut cumulative = 0.0;
    let mut nucleus = Vec::new();
    for (code, value) in candidates {
        let probability = (value - max).exp() / total;
        nucleus.push((code, probability));
        cumulative += probability;
        if cumulative >= 0.95 {
            break;
        }
    }
    let target = draw * cumulative;
    let mut sum = 0.0;
    for (code, probability) in &nucleus {
        sum += probability;
        if target < sum {
            return Ok(*code);
        }
    }
    Ok(nucleus.last().ok_or(TtsError::Failed)?.0)
}

pub(super) fn remember(history: &mut VecDeque<i32>, code: i32) {
    history.push_back(code);
    if history.len() > 64 {
        history.pop_front();
    }
}

#[cfg(test)]
#[path = "sampling_tests.rs"]
mod tests;
