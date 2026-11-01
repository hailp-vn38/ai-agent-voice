# 02: MuseTaskTool và artifact

Status: resolved
Type: task
Blocked by: 01 (interface; có thể chuẩn bị song song)

Spec: [muse-task-tool](../spec.md)

Tool resumable, validation, journal, artifact và test qua Orchestrator thật với FakeAgentTaskProvider. Sở hữu services/task_flow/muse_task/tool.rs và tests; không sửa task_flow/mod.rs.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

## Answer

- Triển khai `MuseTaskTool` Resumable với guard Sub-Agent, validation `INVALID_ARGUMENT`, key theo Agent Session, journal scope/pending arguments và deadline tuyệt đối giữ qua retry. Property schema để permissive vì prevalidator của orchestrator-agent 0.6.0 dùng mã `INVALID_ARGUMENTS`; server tự validate object arguments để giữ mã hợp đồng, không sửa thư viện.
- Replay `Committed` giữ nguyên content/metadata/artifact IDs, terminal failed/expired/cancelled không gọi provider. Timeout/stalled giữ partial text và hướng dẫn collect; RetryLater/unknown giữ operation mở; hủy người dùng ghi Cancelled, shutdown giữ operation để phục hồi.
- Xuất file qua ArtifactStore có scope, allowlist extension, tên an toàn, giới hạn byte, SHA-256/MIME/size/origin; Markdown dùng Muse file hoặc fallback message; message Unicode hoặc có JSON escape lớn chuyển thành artifact với preview có giới hạn. Files thiếu có status partial và warning rõ ràng.
- TDD qua seam đã xác nhận: Orchestrator thật + SQLite/ArtifactStore tạm + scripted LLM + FakeAgentTaskProvider; probe gọi HostTool công khai bằng ToolContext do runtime mint để kiểm replay/cancel/terminal checkpoint. Có RED→GREEN cho Muse-file origin, message Unicode/JSON escape vượt cap, provider failure mapping, committed/error replay, terminal guard, cancellation và deadline khi provider đang chờ.
- Validation sau merge integration `121f0335`: `CARGO_TARGET_DIR=/tmp/muse-task-host-target scripts/agent-test.sh targeted services::task_flow::muse_task::tests` **PASS 15 tests**; `scripts/agent-test.sh check voice-agent-server` (cùng target dir) **PASS**; file rustfmt và `git diff --check` **PASS**.
- Library 0.6.0 đổi Dispatch::RetryLater thành waiting_retry với code `PROVIDER_UNAVAILABLE` và giữ detail; test xác nhận qua waiting_descendant/resume_waiting, invocation và deadline không đổi. Gate Muse thật ở ticket 07, lifecycle ở ticket 04 và review ở ticket 06 vẫn riêng.
