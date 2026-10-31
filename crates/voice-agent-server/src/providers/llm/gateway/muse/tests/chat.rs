use std::collections::VecDeque;
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{Value, json};

use super::harness::{
    FakeMuseVmTransport, FakeStreamTurn, FakeVmAccount, SECOND_COOKIE, SINGLE_COOKIE,
    build_gateway_muse_provider,
};
use crate::providers::llm::{ChatMessage, LlmEvent, LlmOperation, LlmOutput, LlmProvider, LlmRequest};

#[tokio::test]
async fn chat_stream_emits_realtime_deltas_and_reuses_session_id_on_followup() {
    let mut turn1 =
        FakeStreamTurn::chat_reply(vec!["Xin ", "chào ", "bạn!"], "Xin chào bạn!");
    turn1.assigned_session_id = Some("sess-hot-1".to_string());
    let turn2 = FakeStreamTurn::chat_reply(vec!["Hôm nay ", "trời đẹp."], "Hôm nay trời đẹp.");

    let acc = FakeVmAccount::new(vec![turn1, turn2]);
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    // Turn 1: stream deltas
    let req1 = LlmRequest::text_turn("Chào Muse".to_string());
    let mut stream = provider.stream(req1).await.expect("open stream");
    let mut deltas = Vec::new();
    while let Some(item) = stream.next().await {
        match item.expect("stream event") {
            LlmEvent::TextDelta(d) => deltas.push(d),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(deltas.concat(), "Xin chào bạn!");
    assert!(
        acc.written_files()
            .iter()
            .any(|(path, _)| path == "AGENTS.md"),
        "must write isolation policy AGENTS.md on connect"
    );

    // Turn 2: follow-up in the same conversation must reuse `sess-hot-1` and only send the latest user prompt
    let req2 = LlmRequest {
        messages: vec![
            ChatMessage::User {
                content: "Chào Muse".to_string(),
            },
            ChatMessage::AssistantText {
                content: "Xin chào bạn!".to_string(),
            },
            ChatMessage::User {
                content: "Thời tiết thế nào?".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let reply2 = provider
        .invoke(LlmOperation::Chat {
            request: req2,
            reasoning: false,
        })
        .await
        .expect("turn 2 chat");
    assert_eq!(reply2, LlmOutput::Text("Hôm nay trời đẹp.".to_string()));

    let reqs = acc.stream_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[1].get("session_id").and_then(Value::as_str),
        Some("sess-hot-1"),
        "follow-up turn must reuse hot session_id"
    );
    assert_eq!(
        reqs[1]["items"][0]["text"].as_str(),
        Some("Thời tiết thế nào?"),
        "follow-up turn must send only the new user message"
    );
}

#[tokio::test]
async fn pool_rotates_to_second_account_on_quota_or_auth_failure() {
    let acc1 = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec!["You have reached your daily limit. Please try again tomorrow."],
        "You have reached your daily limit. Please try again tomorrow.",
    )]);
    let acc2 = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec!["Phản hồi từ tài khoản #2"],
        "Phản hồi từ tài khoản #2",
    )]);

    let transport = FakeMuseVmTransport::new(vec![acc1, acc2]);
    let provider =
        build_gateway_muse_provider(&[SINGLE_COOKIE, SECOND_COOKIE], transport).await;

    let req = LlmRequest::text_turn("Kiểm tra failover".to_string());
    let reply = provider
        .invoke(LlmOperation::Chat {
            request: req,
            reasoning: false,
        })
        .await
        .expect("pool rotates to second account on quota limit");
    assert_eq!(reply, LlmOutput::Text("Phản hồi từ tài khoản #2".to_string()));
}

#[tokio::test]
async fn chat_stream_waits_for_in_progress_assistant_and_reconciles_full_history_text() {
    let full_turn1_part1 = "# Mesut Özil — Tổng quan sự nghiệp & hoạt động sau giải nghệ\n\n**Mesut Özil** (sinh 15/10/1988 tại Gelsenkirchen) là cựu tiền vệ tấn công người Đức gốc Thổ Nhĩ Kỳ.";
    let full_turn1_part2 =
        "1. **Sự nghiệp**: Schalke 04, Werder Bremen, Real Madrid, Arsenal, Fenerbahçe, İstanbul Başakşehir.\n2. **Danh hiệu**: Vô địch World Cup 2014 cùng đội tuyển Đức.";
    let expected_turn1 = format!("{full_turn1_part1}\n\n{full_turn1_part2}");

    let flattened_turn1_user =
        "[System]\nTrả lời chi tiết kèm nguồn.\n\n[User]\nHãy tìm thông tin về Mesut Özil";
    let in_progress_poll = vec![
        json!({
            "event_name": "message.user",
            "payload": { "display_text": flattened_turn1_user }
        }),
        json!({
            "event_name": "message.assistant",
            "payload": { "display_text": "# Mesut Öz" }
        }),
    ];

    let turn1 = FakeStreamTurn {
        status: 200,
        assigned_session_id: Some("sess-ozil-1".to_string()),
        events: vec![
            json!({
                "type": "event",
                "event": "sessions.updated",
                "payload": {
                    "change": "updated",
                    "session": {
                        "session_id": "sess-ozil-1",
                        "status": "running"
                    }
                }
            }),
            json!({
                "type": "event",
                "event": "agent.status",
                "payload": {
                    "chat_context": { "chat_id": "sess-ozil-1" },
                    "status": "online",
                    "activity_code": "working"
                }
            }),
        ],
        history_polls: VecDeque::from(vec![
            in_progress_poll.clone(),
            in_progress_poll.clone(),
            in_progress_poll,
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
                json!({
                    "event_name": "tool_group",
                    "payload": {
                        "status": "completed",
                        "data": { "display_state": "done" }
                    }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "content": [{ "type": "text", "text": full_turn1_part2 }]
                    }
                }),
            ],
        ]),
    };

    let turn2_reply = "Dưới đây là bản tổng hợp chi tiết về Mesut Özil kèm nguồn tham khảo.";
    let turn2 = FakeStreamTurn {
        status: 200,
        assigned_session_id: Some("sess-ozil-1".to_string()),
        events: vec![json!({
            "type": "event",
            "event": "sessions.updated",
            "payload": {
                "change": "updated",
                "session": {
                    "session_id": "sess-ozil-1",
                    "status": "completed"
                }
            }
        })],
        history_polls: VecDeque::from(vec![
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
            ],
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": "Kết quả trước không hiển thị. Hãy thực hiện lại." }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": turn2_reply
                    }
                }),
            ],
        ]),
    };

    let acc = FakeVmAccount::new(vec![turn1, turn2]);
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let req1 = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Trả lời chi tiết kèm nguồn.".to_string(),
            },
            ChatMessage::User {
                content: "Hãy tìm thông tin về Mesut Özil".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let mut stream = provider.stream(req1).await.expect("open turn 1 stream");
    let mut deltas = Vec::new();
    while let Some(item) = stream.next().await {
        match item.expect("turn 1 stream event") {
            LlmEvent::TextDelta(d) => deltas.push(d),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(deltas.concat(), expected_turn1);

    let req2 = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Trả lời chi tiết kèm nguồn.".to_string(),
            },
            ChatMessage::User {
                content: "Hãy tìm thông tin về Mesut Özil".to_string(),
            },
            ChatMessage::AssistantText {
                content: expected_turn1,
            },
            ChatMessage::User {
                content: "Kết quả trước không hiển thị. Hãy thực hiện lại.".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let reply2 = provider
        .invoke(LlmOperation::Chat {
            request: req2,
            reasoning: false,
        })
        .await
        .expect("turn 2 invoke");
    assert_eq!(reply2, LlmOutput::Text(turn2_reply.to_string()));
}
