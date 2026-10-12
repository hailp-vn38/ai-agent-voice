# 04: Tạo và xem Reminder bền vững ở Device Detail

**What to build:** Admin lưu một Reminder cho physical Device và xem lại sau server restart, với trạng thái saved/chưa sync trung thực.

**Blocked by:** None (can start immediately).

Status: resolved

- [x] Forward-only migration và DeviceNotifications giữ Reminder theo Device, Agent grant default deny có Admin GET/PATCH + revision và setting UI để operator cho phép delivery, Schedule Revision/row revision tách biệt; bảo toàn dữ liệu cũ.
- [x] Admin authenticated create/list có filter/page, quota tối đa 500 pending, request identity/error/audit conventions và UI form/list loading/error/i18n/accessibility.
- [x] Server clock tính relative 1–31.536.000 giây hoặc absolute RFC3339/timezone; đúng một time input, validate text/controls/bytes, giờ mơ hồ không đoán.
- [x] Create + Schedule Revision + durable mutation key/response atomic; retry cùng payload replay sau concurrency/restart, khác payload conflict, không tăng revision khi replay/rollback.
- [x] Device owner không đổi khi reassignment; Virtual Device không được chọn làm physical wake target; enabled không được trình bày là WS online.
- [x] Public HTTP tests/fake clock/Web tests chứng minh persist/restart, auth/validation/quota và sync pending/unsupported; chưa có scheduler không được báo reminder đã phát.

## Answer

Implemented forward-only migration `0104_device_reminders.sql`: Agent permission default false with independent policy revision; Device-owned reminder/schedule tables, durable mutation keys, cache membership and delivery attempts. Existing tables/data are preserved; reminder/cache rows follow Device deletion through FK cascade.

`database::notifications::DeviceNotifications` exposes async create/list/grant/set_grant with an injectable server clock. Create uses one BEGIN IMMEDIATE transaction for quota, row/schedule revisions, response replay, seven-day supported mutation-key retention and privacy-safe audited commit. Admin create requires Idempotency-Key and route-bound physical Device; same key/payload replays across concurrent calls/restart, changed payload conflicts. Absolute timestamps require RFC3339 UTC offset; ambiguous local times are rejected. List includes filter/page/total/has_more and schedule summary; session presence and capability are unknown until active registry wiring, never inferred from enabled. Saved reminders remain pending with no claim of playback/sync.

Admin routes: GET/POST `/api/admin/devices/{device_id}/reminders`; GET/PATCH `/api/admin/agents/{key}/reminder-policy` with quoted If-Match. Device Detail has accessible create/list/filter/pagination and en/vi text; Agent Tools tab has permission setting. Browser retries preserve mutation identity after network failure, route-change responses are fenced, failed permission saves retain denied checkbox state.

Evidence: three public HTTP/service fake-clock tests cover persistence/restart, concurrent replay, conflict, validation, 500 pending quota and rollback revision, default-deny/CAS grants, assignment ownership, pagination and Virtual Device rejection. Web full suite: 52 files / 214 tests passed; focused new component tests: 3 passed; typecheck and production build passed. Final integration includes ticket01; `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --workspace` passed before commit (workspace compile 12m05s; optional model/hardware tests retain their documented ignored status).

Followups 05/11: reuse public `CreateReminder`, `Reminder`, `NotificationError` and `DeviceNotifications::create(device_id,caller_namespace,mutation_key,input,source,audit_request_id)`, `list(device_id,status,page,page_size)`, `grant(key)` and `set_grant(key,enabled,revision,audit_request_id)`. Sync/presence/scheduler and per-tool live authorization are subsequent ticket responsibilities.
