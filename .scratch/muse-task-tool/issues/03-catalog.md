# 03: Cloud worker, catalog và Admin

Status: resolved
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

Cấu hình cloud_worker mặc định tắt, provider tag agent_task, prompt an toàn/use-once/collect/Voice, Admin hiển thị. Test và tài liệu đồng bộ settings giữ tùy chỉnh. Sở hữu agent_settings, prompt, Admin; không sửa task_flow/mod.rs.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

- 2026-11-01: Agent catalog nhận ticket; xác nhận integration tip là ancestor trước triển khai. Dùng TDD tại seam Admin/settings và orchestrator đã chốt trong spec.

## Answer

- Thêm Built-in Sub-Agent `cloud_worker` dưới daily_root, mặc định OFF, chỉ có tool `muse_task`; discovery và prompt root chỉ liệt kê khi bật.
- Catalog suy ra tag `agent_task` chỉ cho Muse, expose cho Admin, validate/chọn `tool_providers.muse_task`; không đổi `LlmCapabilityTag`. Backend accessor `agent_task_provider()` resolve live theo settings mỗi invocation.
- Admin có `default_enabled` để Reset cloud_worker về OFF, không coi trạng thái OFF mặc định là tùy chỉnh; reserve ID built-in trong form tạo custom agent.
- Prompt con nêu use-once, send/collect, dữ liệu ngoài, không tự gửi Memory/Wiki và dữ liệu nhạy cảm; block root hạn chế Task Voice. Khi sync, thay block discovery cũ, giữ nội dung Admin ngoài block và settings đã lưu. [Hướng dẫn nâng cấp](../../../docs/muse-task-settings-upgrade.md).
- TDD: RED Admin mặc định (10 thay vì11 agents), RED thiếu tag catalog Muse, RED nút Reset bật cloud_worker và RED prompt còn discovery cũ; từng slice đã GREEN.
- Validation qua wrapper: Admin HTTP `background_settings_integrates_llm_providers_and_configures_subagents` PASS 1 test; public settings/prompt `muse_task_settings_upgrade` PASS 1 test; BackgroundPage.test.ts PASS 8 tests; web typecheck PASS; cargo fmt PASS; diff whitespace PASS. Runtime Host Tool/closer và Muse live thuộc ticket khác, không được suy ra từ các PASS này.
- Đã xác nhận ancestry integration trước triển khai; merge `feat/muse-task-tool` trước completion trả Already up to date. Không push.
