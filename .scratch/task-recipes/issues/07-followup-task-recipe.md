# 07: Follow-up trên Task recipe

**What to build:** Admin gửi follow-up trên một Job recipe như Job thường. Lượt follow-up chạy agent loop của `daily_root`, lấy kết quả, artifact và bước dừng của recipe làm ngữ cảnh, để agent làm tiếp hoặc trả lời về việc đã làm. Recipe đã ghim không bị chạy lại.

**Blocked by:** 02, 06

**Status:** ready-for-agent

- [ ] Follow-up đi qua đường Task Follow-up hiện có, vào Agent Session của `daily_root`, kèm tóm tắt có cấu trúc gồm recipe, tham số, trạng thái từng bước, artifact và `stop_reason`.
- [ ] Recipe không chạy lại, và con trỏ bước không đổi sau follow-up.
- [ ] Mỗi Agent Run được đánh dấu là lượt recipe hay lượt tự do. Timeline trên trang chi tiết đánh dấu riêng lượt tự do.
- [ ] Integration test TaskManager cho thấy follow-up chạy một lượt agent loop có ngữ cảnh recipe, kể cả khi recipe đã dừng giữa chừng.
