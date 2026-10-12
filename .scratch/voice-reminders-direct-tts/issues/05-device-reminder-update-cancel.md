# 05: Sửa và hủy Reminder với CAS từ Admin Web

**What to build:** Admin sửa hoặc hủy lịch của Device, phát hiện thay đổi đồng thời và không mất lịch khi đổi Agent.

**Blocked by:** 04: Tạo và xem Reminder bền vững ở Device Detail.

Status: resolved

- [x] PATCH/cancel conditional row revision đúng convention, stale edit trả conflict hiển thị trên UI; hủy là soft cancel, không xóa audit.
- [x] Update/cancel và tăng đúng một Schedule Revision khi thay đổi pending cùng transaction; replay idempotent/rollback không tăng, khác payload conflict.
- [x] Mutation keys caller-bound dùng chung create/update/cancel, retention không xóa key trước cửa sổ retry hỗ trợ.
- [x] UI có edit/cancel confirmation, timezone/time validation và không tự nhận synced sau commit; API không cho chéo target Device.
- [x] Reassignment giữ owner/Reminder; Device delete theo FK/retention hiện hữu có regression test, không vô tình xóa lịch khi đổi Agent.
- [x] HTTP concurrency + restart + Web tests xác nhận hai writer không lost update, cancel replay an toàn và terminal lịch không bị sửa/phát lại sai.

## Answer

Added public `DeviceNotifications::update(device_id,reminder_id,expected_row_revision,caller_namespace,mutation_key,input,audit_request_id)` and `cancel(device_id,reminder_id,expected_row_revision,caller_namespace,mutation_key,audit_request_id)`. `UpdateReminder` supports optional message/time/timezone fields; unspecified values remain intact, ambiguous local time and both time inputs fail validation. Create/update share validation and seven-day bounded mutation-key maintenance. Mutations acquire BEGIN IMMEDIATE, replay before CAS, bind reminder to its Device, reject terminal rows, increment row and Device schedule revisions atomically with response/key/audit. Cancel is soft terminal cancellation. Rescheduling clears next-attempt backoff but preserves attempts_used; playback admission must revalidate row identity/status in delivery tickets.

Admin PATCH `/devices/{device_id}/reminders/{reminder_id}` and POST `.../{reminder_id}/cancel` (empty JSON object) require authenticated Admin, quoted If-Match and Idempotency-Key. Audit correlation remains server-generated and independent of client mutation keys; revision conflicts use existing best-effort audit convention.

Device Reminder UI edits the selected row with its observed revision, preserves timezone and durable key on network retry, validates explicit UTC offset/relative bounds, refreshes conflict state and displays localized 409 guidance. Cancellation uses the existing accessible ConfirmDialog, with explanatory offline-sync copy and retry identity. Pending status never implies sync/playback. Changing Device fences stale results and clears edit/cancel selection.

Evidence: public Rust reminder tests (5) passed, including two concurrent writers yielding exactly one winner, rollback revision, cross-Device rejection, terminal immutability, update/cancel replay after restart, payload conflict, assignment retention and Device delete/recreate cascade. Web full suite 52 files/216 tests passed, focused Reminder component 4 tests passed, typecheck/build passed. Mandatory `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --workspace` passed before commit (workspace compile10m39s; optional model/hardware tests remain ignored as documented).
