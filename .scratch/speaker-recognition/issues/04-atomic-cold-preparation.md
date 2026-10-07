# 04: Cold preparation dùng admission nguyên tử

**What to build:** Prepare, admission hoặc prewarm chỉ dựng native runtime khi voice pipeline và native enrollment đều rảnh; cold switch busy không làm đổi profile hiện tại.

**Blocked by:** 03: Giới hạn toàn pipeline và báo busy trên WS.

**Status:** ready-for-agent

- [ ] Manager materialization acquire cùng admission state nguyên tử; không check-rảnh-rồi-load, không tạo model manager/cache riêng.
- [ ] Giữ quyền qua build/readiness/warmup tới terminal acknowledgement; HTTP timeout/cancel không trả sớm; capture/enrollment mới busy trong thời gian đó.
- [ ] Áp dụng startup/background prewarm, prepare, diagnostics, admission và switch; max_parallel_loads không thay gate. Ready backing runtime hot acquire giữ capacity hiện có.
- [ ] Cold target khi session giữ slot trả busy không build, giữ profile; operator prepare trước session. Enrollment admission hook có sẵn để sample slice sử dụng.
- [ ] Public prepare/WS race tests qua manager chứng minh atomic exclusion, hot reuse và late completion; Provider UI/API phân biệt busy với permanent unavailable.
