# 07: Follow-up trên Task recipe

**What to build:** Admin gửi follow-up trên một Job recipe như Job thường. Lượt follow-up chạy agent loop của `daily_root`, lấy tham số, `RecipeProgress` và artifact của recipe làm ngữ cảnh, để agent làm tiếp hoặc trả lời về việc đã làm. Root recipe không nhận thêm lượt nào.

**Blocked by:** 02, 06

**Status:** ready-for-agent

- [ ] Follow-up đầu tiên trên Task recipe mở một Agent Session `daily_root` gắn với Task, kèm tóm tắt có cấu trúc gồm recipe, tham số, `RecipeProgress` và artifact. Các follow-up sau đó tiếp tục trên cùng session này.
- [ ] Root recipe không nhận lượt mới, và `RecipeProgress` không đổi sau follow-up.
- [ ] Mỗi Agent Run của Task được đánh dấu là lượt recipe hay lượt tự do. Timeline trên trang chi tiết đánh dấu riêng lượt tự do.
- [ ] Integration test TaskManager cho thấy follow-up chạy một lượt agent loop có ngữ cảnh recipe, kể cả khi recipe đã dừng giữa chừng.
