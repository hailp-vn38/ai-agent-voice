# Muse Task implementation map

## Notes

Spec: [spec.md](spec.md). Base: 4315248cdb6019085bb02f509afa61e83d1efab9. Integration: feat/muse-task-tool.

01 gateway và 03 catalog độc lập. 02 chuẩn bị theo interface đã chốt, tích hợp sau 01. 04 xây theo port 373e5e99 và journal contract đã chốt, nghiệm thu đợi tích hợp 01/02; 05 đợi 04; 06 review sau 05 và đã xử lý findings trong 593d79ef. 07 live là gate riêng, NOT_RUN không được coi resolved.

## Decisions-so-far

- [02 Host tool](issues/02-host-tool.md) resolved; câu trả lời và triển khai tại `fcbe18fc`: durable scoped tool, journal recovery và artifact publication; targeted tests 15 PASS, cargo check PASS.

- [03 catalog](issues/03-catalog.md) resolved; tích hợp ở 5d194db5. Mặc định OFF, tag Muse-only, bảo toàn prompt tùy chỉnh; HTTP/settings/Web/typecheck PASS.

- Không sửa thư viện/schema; dùng seam đã được xác nhận trong spec.

- [03 Catalog/Admin](issues/03-catalog.md): default OFF và live Muse provider selection; refresh discovery giữ Admin prompts; Admin HTTP, prompt upgrade, web UI/typecheck đã PASS.

- [05 Docs](issues/05-docs.md) resolved: [ADR 0137](../../docs/adr/0137-subagent-owned-muse-task-sessions.md), glossary/README, [validation](../../docs/providers/muse-task-validation-2026-11-02.md) và [runbook](../../docs/providers/muse-task-runbook.md); stage0 có hai send cùng phiên, file kiểm checksum, history 200 và DELETE 2xx. Full runtime/restart/cleanup/quota vẫn NOT_RUN.

## Fog

- Stage0 Muse thật đã đo hai send cùng phiên, Markdown/file, history 200 và DELETE 2xx ACK; không có browser-event proof. Nghiệm thu Background Task, reconnect, kill/restart, parent đọc artifact và task/file/quota sau cleanup còn mở. Xem báo cáo validation; không suy PASS live từ test giả.
