# 07: Follow-up trên Task recipe

**What to build:** Admin gửi follow-up trên một Job recipe như Job thường. Lượt follow-up chạy agent loop của `daily_root`, lấy tham số, `RecipeProgress` và artifact của recipe làm ngữ cảnh, để agent làm tiếp hoặc trả lời về việc đã làm. Root recipe không nhận thêm lượt nào.

**Blocked by:** 02, 06

**Status:** resolved

- [x] Follow-up đầu tiên trên Task recipe mở một Agent Session `daily_root` gắn với Task, kèm tóm tắt có cấu trúc gồm recipe, tham số, `RecipeProgress` và artifact. Các follow-up sau đó tiếp tục trên cùng session này.
- [x] Root recipe không nhận lượt mới, và `RecipeProgress` không đổi sau follow-up.
- [x] Mỗi Agent Run của Task được đánh dấu là lượt recipe hay lượt tự do. Timeline trên trang chi tiết đánh dấu riêng lượt tự do.
- [x] Integration test TaskManager cho thấy follow-up chạy một lượt agent loop có ngữ cảnh recipe, kể cả khi recipe đã dừng giữa chừng.

## Answer

Đã merge vào integration branch `feat/task-recipes` qua server `7cbe476` và flow `e51fd0e`. Follow-up đầu mở/reuse `daily_root` session riêng trên cùng Task, mang context JSON gồm recipe/version/params/progress/artifacts; các follow-up tiếp tục session đó. Recipe root và RecipeProgress giữ nguyên. Lịch sử Agent Run có `run_kind` và timeline phân biệt recipe/free.

Kiểm tra: Flow package tests pass; Admin Web 530/530; targeted projection và TaskManager stopped-recipe follow-up pass. Server package suite còn lỗi ngoài ticket tại `tests/vieneu.rs:66` vì thiếu `config.toml`; flow `fast` còn lỗi lint nền tại `src/tools/pdf.rs:142`.
