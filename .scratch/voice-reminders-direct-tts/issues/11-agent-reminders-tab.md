# 11: Agent Detail tổng hợp lịch của các Device hiện tại

**What to build:** Admin xem/filter/paginate lịch toàn Agent và mở Device để quản lý, kể cả khi Device được chuyển Agent.

**Blocked by:** 05: Sửa và hủy Reminder với CAS từ Admin Web.

Status: ready-for-agent

- [ ] Agent aggregate query join current Device assignment, không dùng Agent lúc tạo; reassignment chuyển view nhưng không chuyển/xóa Reminder owner.
- [ ] Reminders tab có KPI dữ liệu thực, Device/status/time filters, pagination, create/edit/cancel và Device navigation tái dùng Device workflows.
- [ ] Device picker chỉ physical trong current authorized Agent scope, request target/revision đúng; 409/auth/loading/error/i18n/keyboard/ARIA có checks.
- [ ] Saved/known sync/delivery states không bị fabricate; reuse hợp đồng sync bổ sung khi có Ticket 06, không phải blocker để làm aggregate CRUD.
- [ ] Admin HTTP + Web tests chứng minh multi-Device filtering, reassignment race, pagination, wrong Agent/target và không coi enabled là online.
