# 07: Server: `Vault` dùng chung và `KnowledgeWriteGate`

Status: resolved
Blocked by: 01
Spec: ../spec.md (user story 14)
Thiết kế: docs/wiki-desktop/git-sync.md#server

## Mục tiêu

Trên `vela-server`, có một chỗ để chặn mọi lần ghi Knowledge qua HTTP/MCP trong lúc sync, và một `Arc<Vault>` duy nhất.

## Phạm vi

- Mở một `Arc<Vault>` khi `[wiki] enabled`, cất trong `AppState`. `KnowledgeMcp` nhận `Arc<Vault>` thay vì tự `Vault::open` (bỏ `.expect` trong constructor; lỗi mở vault thì MCP báo unavailable như hiện tại).
- `KnowledgeWriteGate` (`tokio::sync::RwLock<()>` + thời điểm ghi cuối) trong `AppState`.
- Middleware axum: các route `/knowledge/*` có method `POST`/`PUT`/`PATCH`/`DELETE` giữ read guard trong suốt request và cập nhật thời điểm ghi cuối. Tool MCP có quyền ghi cũng giữ read guard.
- Route đọc và SSE `/knowledge/events` không đi qua gate.

## Tiêu chí xong

- [x] Test: giữ write guard → request `PUT /knowledge/pages/...` chờ tới khi nhả; `GET` không chờ.
- [x] Test: tool MCP ghi chờ write guard tương tự.
- [x] Test hồi quy Knowledge admin và Knowledge MCP hiện có vẫn PASS.
- [x] `scripts/agent-test.sh` chạy target `vela-server` liên quan trước, rồi workspace.

## Comments

- 2026-11-09: Targeted middleware, MCP write, Knowledge Admin search, MCP read-only denial, and Knowledge MCP HTTP compatibility tests PASS. `scripts/agent-test.sh full` ran after the targeted checks but exited 101 on the unrelated `trusted_aec_auto_barge_in_stops_a_once_and_delivers_b_on_the_same_socket` failure at `crates/vela-server/tests/protocol_e2e.rs:581` (B TTS delivery was not observed). Saved log: `/private/var/folders/j6/50n8d4j97m7ddxpvtwpglnjh0000gn/T/ai-agent-voice-test-logs/agent-test.full.Q81mSp`.
## Review follow-up

- Server sync bounds write-gate acquisition by the configured Git command timeout and publishes a typed timeout status if the gate remains held.
