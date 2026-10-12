# 11: Agent Detail tổng hợp lịch của các Device hiện tại

**What to build:** Admin xem/filter/paginate lịch toàn Agent và mở Device để quản lý, kể cả khi Device được chuyển Agent.

**Blocked by:** 05: Sửa và hủy Reminder với CAS từ Admin Web.

Status: resolved

- [ ] Agent aggregate query join current Device assignment, không dùng Agent lúc tạo; reassignment chuyển view nhưng không chuyển/xóa Reminder owner.
- [ ] Reminders tab có KPI dữ liệu thực, Device/status/time filters, pagination, create/edit/cancel và Device navigation tái dùng Device workflows.
- [ ] Device picker chỉ physical trong current authorized Agent scope, request target/revision đúng; 409/auth/loading/error/i18n/keyboard/ARIA có checks.
- [ ] Saved/known sync/delivery states không bị fabricate; reuse hợp đồng sync bổ sung khi có Ticket 06, không phải blocker để làm aggregate CRUD.
- [ ] Admin HTTP + Web tests chứng minh multi-Device filtering, reassignment race, pagination, wrong Agent/target và không coi enabled là online.

## Answer

Added `DeviceNotifications::list_agent(key,&AgentReminderQuery)` with one SQLite read transaction. Reminder ownership remains Device-based; aggregate joins the current Device Agent assignment. Filters support Device/status/due_from_ms/due_to_ms, bounded page/page_size and stable due/id sorting. Responses return items/total/has_more, lifecycle KPI counts for the selected Device/time range across all pages (status-separated), and paginated physical Device picker options (`device_page`, 200 items, `device_has_more`). Virtual Devices are excluded. Wrong-Agent Device filters fail closed.

GET `/api/admin/agents/{key}/reminders` exposes this query. Scoped Device CRUD/list routes `/api/admin/agents/{key}/devices/{device_id}/reminders[/{reminder_id}[/cancel]]` reuse existing adapters and `DeviceNotifications::scoped_to_agent(key)`. Current Agent ownership is checked inside the same read/write transaction before mutation-key replay, so a reassigned Device cannot accept a stale Agent-page edit/create/cancel. No SQL lives in HTTP adapters and existing Device/tool APIs remain unchanged.

Agent Detail now includes keyboard/ARIA Reminders tab with real KPI counts, Device/status/explicit-offset time filters, total/page navigation, Device detail links, paginated physical picker and shared `DeviceReminders` form/edit/cancel workflows. Shared component accepts optional Agent scope and fences both Device/Agent changes; mutations refresh aggregate data. Error/loading states and en/vi text are included. Sync status remains pending/unknown from current backend capability; enabled is never labeled WS-online and saved is never labeled played.

Evidence: public HTTP test covers multi-Device filtering, stable page2, lifecycle KPI/status filtering, Virtual Device picker exclusion, invalid time range, authenticated routes, wrong-target create and reassignment race rejection; current Agent view moves while owner/reminder persists. Web full suite54 files/220 tests passed, focused3 suites/8 tests passed, typecheck/build passed. `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed using the isolated ticket build cache.
