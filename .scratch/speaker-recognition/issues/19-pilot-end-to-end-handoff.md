# 19: Qualification public và bàn giao pilot V1

**What to build:** Operator có một đường kiểm chứng đầy đủ enrollment web→Observe ESP32→Required guarded, cùng hướng dẫn vận hành vừa đủ và reports rõ software compatibility khác real qualification.

**Blocked by:** 13: Giải quyết conflict bằng discovery theo đợt, 17: Switch Template bằng hot runtime và quyền đúng, 18: Báo cáo calibration và tải pilot có thể tái lập.

**Status:** ready-for-agent

- [ ] Scenario qua production process public Admin HTTP/Voice WS bằng Reference Integration Client kiểm các slice hoàn chỉnh, controlled restart và independent wire contracts; không test-only AppState hoặc runtime bypass.
- [ ] Acceptance mốc đầu có enrollment web, Observe ESP32, applicable allowlist/recovery và pilot envelope. Required chỉ bật với đúng exact qualification/tool rights, không từ CI deterministic đơn thuần.
- [ ] Mandatory Qualification không cần tải model hoặc credentials. Real CAM++/ESP32 evidence chạy khi operator cung cấp corpus/máy; nếu chưa có, report NOT_RUN và prerequisite thiếu, giữ preliminary/Required unavailable.
- [ ] Rust/frontend/protocol/admin/runtime checks đạt. API collection, Reference Client và runbook đã được cập nhật cùng các slice; scenario cuối kiểm compatibility giữa chúng.
- [ ] Runbook có operator review Persona/prompt/context/tool results và re-review khi thay nguồn, không workflow eligibility mới; history cách ly và giới hạn replay/privacy rõ ràng.
- [ ] Không thêm hidden bypass, model manager, scheduler, RBAC hoặc eligibility product. Handoff ghi baseline, tests, Mandatory result và Optional Runtime Evidence thật; không deploy Required hoặc claim accuracy khi thiếu evidence.
