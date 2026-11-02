# 06: Nghiệm thu thật và tài liệu

**What to build:** Chứng minh tính năng chạy đúng trên vault thật với LLM Provider thật, chỉnh giới hạn theo số đo, và ghi lại quyết định vào tài liệu của repo. Kịch bản nào chưa chạy được thì ghi `NOT_RUN`, không tính là đạt. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 05 (toàn bộ tính năng đã có)

**Status:** ready-for-human

- [ ] "Trading trong wiki có gì?" ra cụm trading, không có trang hệ thống
- [ ] "FVG liên hệ thế nào với Sharp Turn Entry?" ra hai cụm và các cạnh nối, trích dẫn dẫn đúng trang
- [ ] "Giao dịch theo FVG là gì?" vẫn tìm ra cụm trading
- [ ] "Công thức nấu phở?" được trả lời là không có, Graph không đổi
- [ ] Source chứa chỉ dẫn xóa wiki không làm agent làm theo, wiki không đổi
- [ ] Giao diện đúng ở chế độ sáng, tối và 700px, có ảnh minh chứng
- [ ] Đo thời gian lượt thật, chỉnh giới hạn nếu cần, ghi số đo vào Nhật ký của tài liệu triển khai
- [x] Viết ADR 0138: chat chỉ đọc cho Admin và server mở rộng đồ thị, tham chiếu ADR 0124
- [x] Cập nhật flow Knowledge Graph, API docs, và GLOSSARY nếu `Answer graph` thành thuật ngữ chung
- [ ] Toàn bộ test, clippy và build web qua `scripts/agent-test.sh`

## Comments

## Answer

Tài liệu P6 đã commit tại `450adf65`. Giữ `ready-for-human` vì vault/provider thật, độ trễ live, ảnh minh chứng giao diện và gate package/clippy chưa đạt; các mục `NOT_RUN` không được tính là pass. `web build` đã qua. Xem [nhật ký triển khai](../../../docs/knowledge/wiki-chat-graph-implementation.md) để biết từng gate và nguyên nhân.
