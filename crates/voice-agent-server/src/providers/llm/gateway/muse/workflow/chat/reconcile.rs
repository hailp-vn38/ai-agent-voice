use serde_json::Value;

use super::super::extract_value_text;

pub(super) fn is_stale_followup_history(
    history_json: &Value,
    expected_user_turns: usize,
    effective_prompt: &str,
) -> bool {
    if expected_user_turns <= 1 {
        return false;
    }
    let evs = history_json
        .get("result")
        .and_then(|r| r.get("chat_events"))
        .or_else(|| history_json.get("chat_events"))
        .and_then(Value::as_array);
    let Some(evs) = evs else {
        return false;
    };
    let user_evs: Vec<&Value> = evs
        .iter()
        .filter(|e| {
            let p = e.get("payload").unwrap_or(e);
            e.get("role").and_then(Value::as_str) == Some("user")
                || p.get("role").and_then(Value::as_str) == Some("user")
                || e.get("event_name").and_then(Value::as_str) == Some("message.user")
                || e.get("event").and_then(Value::as_str) == Some("message.user")
        })
        .collect();
    if user_evs.len() >= expected_user_turns {
        return false;
    }
    let Some(&ev) = user_evs.last() else {
        return true;
    };
    let p = ev.get("payload").unwrap_or(ev);
    let last_u_text = ev
        .get("display_text")
        .and_then(extract_value_text)
        .or_else(|| p.get("display_text").and_then(extract_value_text))
        .or_else(|| p.get("content").and_then(extract_value_text))
        .or_else(|| ev.get("content").and_then(extract_value_text))
        .unwrap_or_default();
    last_u_text.trim() != effective_prompt.trim()
}

pub(super) fn reconcile_and_emit_history_text<F>(
    emitted_text: &mut String,
    candidate_full: &str,
    on_delta: &mut F,
) -> bool
where
    F: FnMut(String) + Send,
{
    if candidate_full.is_empty() || candidate_full.len() <= emitted_text.len() {
        return false;
    }
    if emitted_text.is_empty() {
        *emitted_text = candidate_full.to_string();
        on_delta(candidate_full.to_string());
        return true;
    }
    if candidate_full.starts_with(&*emitted_text) {
        let delta = candidate_full[emitted_text.len()..].to_string();
        *emitted_text = candidate_full.to_string();
        if !delta.is_empty() {
            on_delta(delta);
            return true;
        }
        return false;
    }
    if let Some(idx) = candidate_full.find(&*emitted_text) {
        let delta = candidate_full[idx + emitted_text.len()..].to_string();
        *emitted_text = candidate_full.to_string();
        if !delta.is_empty() {
            on_delta(delta);
            return true;
        }
        return false;
    }

    // Suffix / prefix overlap matching (`muse-api-server-main/src/core/client.js:1397-1410`)
    let max_possible = emitted_text.len().min(candidate_full.len());
    let mut max_overlap = 0usize;
    for i in (1..=max_possible).rev() {
        if candidate_full.is_char_boundary(i) && emitted_text.ends_with(&candidate_full[..i]) {
            max_overlap = i;
            break;
        }
    }
    let delta = candidate_full[max_overlap..].to_string();
    if !delta.is_empty() {
        emitted_text.push_str(&delta);
        on_delta(delta);
        return true;
    }
    false
}

pub(super) fn extract_message_done_text(payload: &Value) -> String {
    if let Some(msgs) = payload
        .get("transcript")
        .and_then(|t| t.get("messages"))
        .and_then(Value::as_array)
    {
        let mut blocks = Vec::new();
        for m in msgs {
            if m.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(t) = m
                    .get("content")
                    .and_then(extract_value_text)
                    .or_else(|| m.get("text").and_then(extract_value_text))
                    .or_else(|| m.get("display_text").and_then(extract_value_text))
                && !t.is_empty()
            {
                blocks.push(t);
            }
        }
        if !blocks.is_empty() {
            return blocks.join("\n\n");
        }
    }
    payload
        .get("content")
        .and_then(extract_value_text)
        .or_else(|| payload.get("text").and_then(extract_value_text))
        .or_else(|| payload.get("display_text").and_then(extract_value_text))
        .unwrap_or_default()
}
