# 08: Tool `recipe_start` cho `daily_root`

**What to build:** Khi người dùng yêu cầu bằng giọng nói hoặc gõ mục tiêu tự do khớp với một recipe, `daily_root` gọi `recipe_start` để tạo một Task recipe mới thay vì tự làm từng bước. Voice Task Tool Suite giữ nguyên 9 tool.

**Blocked by:** 02

**Status:** resolved

- [x] HostTool `recipe_start` chỉ được cấp cho `daily_root`. Tool nhận `recipe_id` và `params`; mô tả tool liệt kê các recipe đang có kèm tóm tắt tham số.
- [x] Tool validate tham số qua cùng đường với `CreateTaskRequest`, rồi trả id Task mới hoặc lỗi kèm `validation_path` để model sửa.
- [x] Tool được phân loại `Mutating` và có khóa idempotency theo lượt gọi. Gọi lại với cùng khóa không tạo Task thứ hai.
- [x] Voice Task Tool Suite không thay đổi.
- [x] Integration test với ScriptedGateway phủ ba trường hợp: tạo thành công, tham số sai, và gọi lặp.

## Answer

Đã merge vào integration branch `feat/task-recipes` qua commit `4efb5c3`. `recipe_start` chỉ được thêm vào `daily_root`; tool validate theo catalog TaskManager, trả `validation_path`, và replay cùng invocation trả lại kết quả mà không tạo Task trùng. Voice Task Tool Suite giữ đúng 9 tool. Các test mục tiêu recipe_start, hai fixture playlist mới, allowlist `daily_root` và Voice suite đều pass. `scripts/agent-test.sh package voice-agent-server` vẫn dừng ở test `vieneu` do workspace không có `config.toml` tại base; đây là suite failure ngoài acceptance của ticket, không được coi là pass.
