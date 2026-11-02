# 06: Nghiệm thu thật và tài liệu

**What to build:** Chứng minh tính năng chạy đúng trên vault thật với LLM Provider thật, chỉnh giới hạn theo số đo, và ghi lại quyết định vào tài liệu của repo. Kịch bản nào chưa chạy được thì ghi `NOT_RUN`, không tính là đạt. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 05 (toàn bộ tính năng đã có)

**Status:** ready-for-agent

- [ ] "Trading trong wiki có gì?" ra cụm trading, không có trang hệ thống
- [ ] "FVG liên hệ thế nào với Sharp Turn Entry?" ra hai cụm và các cạnh nối, trích dẫn dẫn đúng trang
- [ ] "Giao dịch theo FVG là gì?" vẫn tìm ra cụm trading
- [ ] "Công thức nấu phở?" được trả lời là không có, Graph không đổi
- [ ] Source chứa chỉ dẫn xóa wiki không làm agent làm theo, wiki không đổi
- [ ] Giao diện đúng ở chế độ sáng, tối và 700px, có ảnh minh chứng
- [ ] Đo thời gian lượt thật, chỉnh giới hạn nếu cần, ghi số đo vào Nhật ký của tài liệu triển khai
- [ ] Viết ADR 0138: chat chỉ đọc cho Admin và server mở rộng đồ thị, tham chiếu ADR 0124
- [ ] Cập nhật flow Knowledge Graph, API docs, và GLOSSARY nếu `Answer graph` thành thuật ngữ chung
- [ ] Toàn bộ test, clippy và build web qua `scripts/agent-test.sh`
