# 05: Liên kết trích dẫn và node

**What to build:** Câu trả lời và Graph nối với nhau theo hai chiều: Admin đi từ trích dẫn tới node và ngược lại, xem lại đồ thị của câu trả lời cũ, và có gợi ý để bắt đầu khi chat còn trống. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 04 (khung chat và scope Câu trả lời)

**Status:** ready-for-agent

- [ ] Mỗi câu trả lời có danh sách nguồn đánh số theo thứ tự xuất hiện, lấy từ `citations`
- [ ] Trích dẫn trong bằng chứng thành `[n]` bấm được; link ngoài bằng chứng hiện là văn bản thường
- [ ] Rê chuột lên trích dẫn làm sáng node và mờ phần còn lại; rê chuột lên node làm sáng trích dẫn tương ứng
- [ ] Bấm trích dẫn chọn node và căn giữa Graph, không rời tab Graph
- [ ] `Xem trên Graph` ở tin cũ đưa Graph về đồ thị của tin đó
- [ ] Bước tool xong thu thành một dòng tóm tắt, mở ra được để xem truy vấn và các trang đã thấy
- [ ] Chat trống có ba gợi ý câu hỏi lấy từ các thư mục cấp 1 của wiki; bấm gợi ý là gửi câu hỏi
- [ ] Test bằng cách mount trang Knowledge, mock `fetch` và Cytoscape
