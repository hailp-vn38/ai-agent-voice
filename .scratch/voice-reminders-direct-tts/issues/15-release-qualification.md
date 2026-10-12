# 15: Qualification toàn flow và hướng dẫn rollout/rollback

**What to build:** Operator nhận release có compatibility matrix, bằng chứng online/offline end-to-end và cách bật/tắt, giám sát, rollback mà không mất lịch.

**Blocked by:** 03: Client yêu cầu Direct TTS trên Voice Session; 10: Studio Assistant chọn Device rồi quản lý Reminder; 11: Agent Detail tổng hợp lịch của các Device hiện tại; 14: Nghiệm thu hardware với nhiều lịch và lỗi mạng/nguồn.

Status: ready-for-agent

- [ ] End-to-end Voice/Web/Studio creation và CRUD → DB save/cache sync → online hoặc hardware offline delivery → drain ACK → terminal sync pass; no LLM on due.
- [ ] Matrix old firmware/new firmware/Studio/server reminders disabled, default off và deny-by-default grants, no legacy Voice/MCP regressions.
- [ ] Rust fmt/clippy all-targets/all-features/test workspace, Web test/typecheck/build và firmware build/hardware gates có actual results; lint chỉ nếu repo có script.
- [ ] Privacy-safe create/due/completed/retry/expired/sync/cache ACK/playback ACK/lateness/direct TTS metrics và correlation logs có kiểm tra, không full text/ID labels.
- [ ] Official protocol/flow, rollout/gate/grant/TTL/retention, late/uncertain/offline handling, reassignment, feature-disable/shutdown và forward-only compatible backup rollback được bàn giao.
- [ ] Không xóa persisted schedule/schema hoặc thêm stubs để pass; thiếu hardware chỉ server-ready, firmware-unverified và full release gate chưa complete.
- [ ] Cập nhật evidence/status ADR theo thực tế triển khai; không tự push/merge/deploy. Full DoD chỉ đạt khi tất cả required gates có bằng chứng.
