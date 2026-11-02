# 01: Graph theo truy vấn

**What to build:** Admin nhập từ khóa trên tab Graph và thấy đúng cụm Wiki Page và Source liên quan cùng Wikilink có thật giữa chúng, không cần cấu hình LLM. Trang khớp và trang ngữ cảnh nhìn khác nhau, có chú thích. Đây cũng là lát đầu tiên của Answer graph builder mà các ticket sau dùng lại. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] Tách phần gộp cạnh của Knowledge Graph endpoint để dùng chung, scope Global và Local trả đúng như trước
- [x] `scope=query&q=...` trả node có `tier` và `score`; thiếu hoặc rỗng `q` trả `400 invalid_input`; không xác thực bị từ chối
- [x] Kết quả không có `category = system`, `wiki/index.md`, `wiki/log.md`, kể cả khi các trang đó khớp từ khóa
- [x] Kết quả gồm cả Wiki Page và Source; mở rộng 1 bước, ưu tiên láng giềng nối với nhiều hạt giống nhất
- [x] Tối đa 60 node, không cắt hạt giống trước `context`, vượt giới hạn thì `truncated = true`
- [x] Mọi node và cạnh trả về đều tồn tại trong dữ liệu Graph đầy đủ
- [x] Graph có ô nhập ở scope truy vấn, kiểu node theo mức giữ màu theo loại trang, có chú thích mức và thông báo khi không có kết quả hoặc bị cắt
- [x] Bấm node ở Toàn bộ và Cục bộ vẫn mở trang như cũ
- [x] Đo trên vault thật với "trading", "giao dịch", "robot" và một từ vô nghĩa, chốt `min_score` thành hằng số
- [x] Xác nhận embedder có chạy khi truy vấn không; ghi số đo và kết luận vào Nhật ký của tài liệu triển khai
- [x] Test qua hai seam của spec: HTTP thật với vault tạm, và mount trang Knowledge với Cytoscape mock

## Comments

## Answer

Đã hoàn tất trên nhánh tích hợp: `d16861c1` (query graph), kiểm tra vault tạm ghi trong [nhật ký triển khai](../../../docs/knowledge/wiki-chat-graph-implementation.md). Ngưỡng `QUERY_GRAPH_MIN_SCORE = 0.12`. HTTP focus 2/2, web suite 427 test; rustfmt và diff check đạt. Việc gọi embedder được xác nhận theo đường mã `search_vector`, không có instrumentation đếm lời gọi.
