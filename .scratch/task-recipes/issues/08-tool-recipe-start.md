# 08: Tool `recipe_start` cho `daily_root`

**What to build:** Khi người dùng yêu cầu bằng giọng nói hoặc gõ mục tiêu tự do khớp với một recipe, `daily_root` gọi `recipe_start` để tạo một Task recipe mới thay vì tự làm từng bước. Voice Task Tool Suite giữ nguyên 9 tool.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] HostTool `recipe_start` chỉ được cấp cho `daily_root`. Tool nhận `recipe_id` và `params`; mô tả tool liệt kê các recipe đang có kèm tóm tắt tham số.
- [ ] Tool validate tham số qua cùng đường với `CreateTaskRequest`, rồi trả id Task mới hoặc lỗi kèm `validation_path` để model sửa.
- [ ] Tool được phân loại `Mutating` và có khóa idempotency theo lượt gọi. Gọi lại với cùng khóa không tạo Task thứ hai.
- [ ] Voice Task Tool Suite không thay đổi.
- [ ] Integration test với ScriptedGateway phủ ba trường hợp: tạo thành công, tham số sai, và gọi lặp.
