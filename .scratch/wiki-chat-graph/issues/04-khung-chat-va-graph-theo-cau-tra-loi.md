# 04: Khung chat và Graph theo câu trả lời

**What to build:** Admin hỏi ngay trong tab Graph, đọc câu trả lời stream ở cột phải và nhìn Graph ở scope Câu trả lời lớn dần bên cạnh. Bấm node xem chi tiết tại chỗ, mở trang rồi quay lại vẫn còn nguyên hội thoại và đồ thị. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 03 (agent và sự kiện SSE)

**Status:** ready-for-agent

- [ ] Làm trước: tách SSE parser dùng chung, test Jobs và Notifications vẫn qua
- [ ] Khung chat rộng 380px thay cột Outline & links ở tab Graph, nút thu gọn cột phải điều khiển chat; các tab khác giữ nguyên outline
- [ ] Dưới 950px chat là ngăn kéo từ dưới lên, cao 55%, mở bằng nút nổi trên Graph
- [ ] Gửi câu hỏi chuyển Graph sang scope `answer`; Graph lớn dần theo sự kiện `graph` và không gọi API Graph
- [ ] Không vẽ lại khi `revision` không đổi; không có kết quả thì Graph giữ nguyên
- [ ] Bấm node ở scope `answer` mở inspector tại chỗ có nút Mở trang; ở Toàn bộ và Cục bộ vẫn mở trang như cũ
- [ ] Sang tab đọc rồi quay lại vẫn còn hội thoại và đồ thị; chuyển Toàn bộ hoặc Cục bộ không mất chat
- [ ] Câu trả lời render qua `renderMarkdown`; trạng thái tool hiện trong lúc chạy
- [ ] `Dừng` hủy lượt và giữ phần đã nhận; mất kết nối giữ phần đã nhận và có `Thử lại`; web không tự kết nối lại
- [ ] Xóa hội thoại về Graph Toàn bộ
- [ ] Chưa cấu hình provider hiện hướng dẫn tới Settings; `wiki_chat_busy` có thông báo riêng
- [ ] Chỉ đọc tin hoàn tất cho trình đọc màn hình; `prefers-reduced-motion` tắt hiệu ứng
- [ ] Chuỗi giao diện có ở cả hai ngôn ngữ; đúng ở chế độ sáng và tối
- [ ] Test bằng cách mount trang Knowledge, mock `fetch` với luồng SSE dựng sẵn và mock Cytoscape
