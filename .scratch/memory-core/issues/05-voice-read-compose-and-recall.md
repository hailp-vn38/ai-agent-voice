# Voice đọc: compose seam, actor và `server_memory_recall`

Type: task
Status: ready-for-agent
Blocked by: 04

## Scope

Mốc 5 của [spec](../spec.md), GĐ 2 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md) mục 5.

- `compose_turn_system(base, &TurnPromptInputs)` thay các biến thể compose hiện có.
- `SessionMemory` qua admission, không bao giờ từ chối session.
- `server_memory_recall`; cả năm điểm của tool catalog; đổi tên `AsyncToolOutcome::Reminder` thành `Server`.
- Tool ghi và cổng ý định thuộc ticket 07. `<memory_policy>` đi cùng khối Memory ở compose seam, nên làm ở đây với nội dung đầy đủ của spec.

User stories: 39, 42 đến 46, 58 (phần đọc), 59, 61, 66.

## Acceptance

- Mọi dòng mốc 5 trong ma trận test của spec đều có test qua.
- Có benchmark `begin_speech_delivery` có và không có Memory ở lượt không gọi tool, và báo kết quả đo.
- `scripts/agent-test.sh targeted prompt`, `targeted memory`, `targeted builtin` trên `voice-agent-server` xanh.
