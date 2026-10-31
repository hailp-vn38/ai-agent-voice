use std::collections::VecDeque;

use serde_json::{Value, json};

use super::super::workflow::encode_rgba_png;

pub(in crate::providers::llm::gateway) const SINGLE_COOKIE: &str = "hatch_sess=sess_token_1";
pub(in crate::providers::llm::gateway) const SECOND_COOKIE: &str = "hatch_sess=sess_token_2";

pub(in crate::providers::llm::gateway) const VALID_PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

pub(in crate::providers::llm::gateway) const VALID_MP4_BYTES: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00, 0x00,
    b'm', b'p', b'4', b'2', b'i', b's', b'o', b'm',
];

/// Synthetic MP4 whose duration `inspect_mp4_duration_secs` reads from the `mdat` `dur=Ns` marker.
pub(in crate::providers::llm::gateway) fn mp4_with_duration(secs: u32) -> Vec<u8> {
    let mut out = VALID_MP4_BYTES.to_vec();
    let payload = format!("dur={secs}s");
    let len = (8 + payload.len()) as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b"mdat");
    out.extend_from_slice(payload.as_bytes());
    out
}

#[derive(Clone)]
pub(in crate::providers::llm::gateway) struct FakeStreamTurn {
    pub status: u32,
    pub assigned_session_id: Option<String>,
    pub events: Vec<Value>,
    pub history_polls: VecDeque<Vec<Value>>,
}

impl FakeStreamTurn {
    pub fn chat_reply(deltas: Vec<&str>, final_text: &str) -> Self {
        let mut events: Vec<Value> = deltas
            .into_iter()
            .map(|d| {
                json!({
                    "event": "delta.text_append",
                    "payload": { "text": d }
                })
            })
            .collect();
        events.push(json!({
            "event": "delta.message_done",
            "payload": {
                "transcript": {
                    "messages": [
                        { "role": "assistant", "content": final_text }
                    ]
                }
            }
        }));
        let history = vec![
            json!({ "role": "user", "display_text": "prompt" }),
            json!({ "role": "assistant", "status": "completed", "display_text": final_text }),
        ];
        Self {
            status: 200,
            assigned_session_id: None,
            events,
            history_polls: VecDeque::from([history]),
        }
    }

    pub fn media_reply(
        presentations: Vec<Value>,
        history_polls: Vec<Vec<Value>>,
        extra_events: Vec<Value>,
    ) -> Self {
        let mut events = extra_events;
        for p in presentations {
            events.push(json!({
                "event": "delta.presentation",
                "payload": p,
            }));
        }
        events.push(json!({
            "event": "agent.status",
            "payload": { "activity_code": "idle" }
        }));
        Self {
            status: 200,
            assigned_session_id: None,
            events,
            history_polls: VecDeque::from(history_polls),
        }
    }
}

pub(super) fn make_test_rgba_png(
    width: u32,
    height: u32,
    fg_range: std::ops::Range<u32>,
) -> Vec<u8> {
    let mut rgba = vec![255u8; (width * height * 4) as usize];
    for y in fg_range.clone() {
        for x in fg_range.clone() {
            let idx = ((y * width + x) * 4) as usize;
            rgba[idx] = 220;
            rgba[idx + 1] = 30;
            rgba[idx + 2] = 40;
            rgba[idx + 3] = 255;
        }
    }
    encode_rgba_png(width, height, &rgba)
}
