# 07: Server: `Vault` dùng chung và `KnowledgeWriteGate`

Status: ready-for-agent
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

- [ ] Test: giữ write guard → request `PUT /knowledge/pages/...` chờ tới khi nhả; `GET` không chờ.
- [ ] Test: tool MCP ghi chờ write guard tương tự.
- [ ] Test hồi quy Knowledge admin và Knowledge MCP hiện có vẫn PASS.
- [ ] `scripts/agent-test.sh` chạy target `vela-server` liên quan trước, rồi workspace.
