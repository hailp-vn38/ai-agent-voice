# Voice ghi: cổng ý định, ba builtin ghi và sự kiện

Type: task
Status: ready-for-agent
Blocked by: 05

## Scope

Mốc 7 của [spec](../spec.md), bước 3.1 đến 3.3 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md).

- Đấu `classify_intent` trên `ChatMessage::User` cuối cùng vào actor.
- `server_memory_remember`, `server_memory_correct`, `server_memory_forget`, chỉ có ở `read_write`.
- Kết quả forget có `limits`; memory tool tính vào `requires_job_receipt`.
- Sự kiện `memory_claim_without_receipt`, chỉ gồm `session_id` và `turn_id`.

User stories: 47 đến 58, 62, 69.

## Acceptance

- Mọi dòng mốc 7 trong ma trận test của spec đều có test qua, gồm test chống injection qua tool result.
- `scripts/agent-test.sh targeted memory`, `targeted builtin` trên `voice-agent-server` xanh, rồi `full`.
