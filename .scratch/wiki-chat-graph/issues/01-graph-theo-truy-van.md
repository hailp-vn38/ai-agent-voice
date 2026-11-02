# 01: Graph theo truy vấn

**What to build:** Admin nhập từ khóa trên tab Graph và thấy đúng cụm Wiki Page và Source liên quan cùng Wikilink có thật giữa chúng, không cần cấu hình LLM. Trang khớp và trang ngữ cảnh nhìn khác nhau, có chú thích. Đây cũng là lát đầu tiên của Answer graph builder mà các ticket sau dùng lại. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Tách phần gộp cạnh của Knowledge Graph endpoint để dùng chung, scope Global và Local trả đúng như trước
- [ ] `scope=query&q=...` trả node có `tier` và `score`; thiếu hoặc rỗng `q` trả `400 invalid_input`; không xác thực bị từ chối
- [ ] Kết quả không có `category = system`, `wiki/index.md`, `wiki/log.md`, kể cả khi các trang đó khớp từ khóa
- [ ] Kết quả gồm cả Wiki Page và Source; mở rộng 1 bước, ưu tiên láng giềng nối với nhiều hạt giống nhất
- [ ] Tối đa 60 node, không cắt hạt giống trước `context`, vượt giới hạn thì `truncated = true`
- [ ] Mọi node và cạnh trả về đều tồn tại trong dữ liệu Graph đầy đủ
- [ ] Graph có ô nhập ở scope truy vấn, kiểu node theo mức giữ màu theo loại trang, có chú thích mức và thông báo khi không có kết quả hoặc bị cắt
- [ ] Bấm node ở Toàn bộ và Cục bộ vẫn mở trang như cũ
- [ ] Đo trên vault thật với "trading", "giao dịch", "robot" và một từ vô nghĩa, chốt `min_score` thành hằng số
- [ ] Xác nhận embedder có chạy khi truy vấn không; ghi số đo và kết luận vào Nhật ký của tài liệu triển khai
- [ ] Test qua hai seam của spec: HTTP thật với vault tạm, và mount trang Knowledge với Cytoscape mock
