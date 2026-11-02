# 04: Khung chat và Graph theo câu trả lời

**What to build:** Admin hỏi ngay trong tab Graph, đọc câu trả lời stream ở cột phải và nhìn Graph ở scope Câu trả lời lớn dần bên cạnh. Bấm node xem chi tiết tại chỗ, mở trang rồi quay lại vẫn còn nguyên hội thoại và đồ thị. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 03 (agent và sự kiện SSE)

**Status:** resolved

- [x] Làm trước: tách SSE parser dùng chung, test Jobs và Notifications vẫn qua
- [x] Khung chat rộng 380px thay cột Outline & links ở tab Graph, nút thu gọn cột phải điều khiển chat; các tab khác giữ nguyên outline
- [x] Dưới 950px chat là ngăn kéo từ dưới lên, cao 55%, mở bằng nút nổi trên Graph
- [x] Gửi câu hỏi chuyển Graph sang scope `answer`; Graph lớn dần theo sự kiện `graph` và không gọi API Graph
- [x] Không vẽ lại khi `revision` không đổi; không có kết quả thì Graph giữ nguyên
- [x] Bấm node ở scope `answer` mở inspector tại chỗ có nút Mở trang; ở Toàn bộ và Cục bộ vẫn mở trang như cũ
- [x] Sang tab đọc rồi quay lại vẫn còn hội thoại và đồ thị; chuyển Toàn bộ hoặc Cục bộ không mất chat
- [x] Câu trả lời render qua `renderMarkdown`; trạng thái tool hiện trong lúc chạy
- [x] `Dừng` hủy lượt và giữ phần đã nhận; mất kết nối giữ phần đã nhận và có `Thử lại`; web không tự kết nối lại
- [x] Xóa hội thoại về Graph Toàn bộ
- [x] Chưa cấu hình provider hiện hướng dẫn tới Settings; `wiki_chat_busy` có thông báo riêng
- [x] Chỉ đọc tin hoàn tất cho trình đọc màn hình; `prefers-reduced-motion` tắt hiệu ứng
- [x] Chuỗi giao diện có ở cả hai ngôn ngữ; đúng ở chế độ sáng và tối
- [x] Test bằng cách mount trang Knowledge, mock `fetch` với luồng SSE dựng sẵn và mock Cytoscape

## Comments

## Answer

Đã triển khai tại `cdd158ec`. Test KnowledgeGraph/KnowledgePage 32 test; full web suite 430 test, web typecheck đạt. Đã kiểm tra giao diện cục bộ ở sáng/tối và viewport 700px; hội thoại và graph vẫn hiển thị.
