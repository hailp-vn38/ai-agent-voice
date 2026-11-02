# 05: ADR, glossary và hướng dẫn kiểm chứng

Status: resolved
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

Chọn số ADR chưa dùng; glossary/README và validation Muse thật PASS/FAIL/NOT_RUN, runbook mới cho nghiên cứu/file/restart. Không nâng NOT_RUN từ fake test.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

- 2026-11-02: Port, Host Tool, catalog đã có commit; contract closer ổn định ở dd56c92e. Có thể cập nhật tài liệu song song; phân biệt rõ các gate runtime/live còn mở, đồng bộ lần cuối sau tích hợp.

- 2026-11-02: Agent docs nhận ticket trên `feat/muse-task-docs`; cập nhật ADR/glossary, runbook và evidence đã lọc.

## Answer

- Ghi [ADR 0137](../../../docs/adr/0137-subagent-owned-muse-task-sessions.md): con sở hữu session/account, journal chống gửi trùng, archive trước ACK-only cleanup, workaround public API lib 0.6 và drift guard; video giữ phạm vi cũ.
- Cập nhật Job Session/Continuation Key và Cloud Worker trong glossary; README dẫn settings upgrade, hợp đồng use-once và giới hạn Task Voice.
- Bàn giao [validation 2026-11-02](../../../docs/providers/muse-task-validation-2026-11-02.md) với stage0 hai send/file/checksum/history; browser event, cleanup ACK và nghiệm thu Background Task còn NOT_RUN. Snapshot test giả tách khỏi live.
- Bàn giao [runbook](../../../docs/providers/muse-task-runbook.md) cho nghiên cứu/file, reconnect, kill/restart, archive/cleanup và quota, kèm cách thu thập evidence đã lọc.
- Validation tài liệu qua wrapper: changed/new local links và Markdown fences PASS; diff whitespace PASS. Lượt all-link đầu FAIL: một slug ADR mới đã sửa; bốn link README có sẵn trỏ vendor/orchestrator_agent_rs, vendor/orchestrator_task_rs, vendor/llm-wiki-rs và skills không tồn tại, ngoài phạm vi ticket này. Không chạy build/test Rust cho thay đổi tài liệu.
- Resolved chỉ áp dụng phần bàn giao tài liệu; ticket 07/live và nghiệm thu cuối không được coi hoàn tất từ trạng thái này.
