# 11: Phân bổ mặc định của Built-in Job Skill không còn khoảng trống tool

**What to build:**

- Tách phần "lưu câu trả lời thành synthesis" khỏi `wiki-query` thành Built-in Job Skill mới `wiki-synthesis-filing`, gán mặc định cho `wiki_curator`. `wiki-query` chỉ còn hướng dẫn đọc.
- Mỗi Built-in Job Skill khai báo `requires_tools` là tập tool tối thiểu chung cho mọi Sub-Agent được gán mặc định. Hướng dẫn dùng tool mà chỉ một số Sub-Agent có được viết theo dạng có điều kiện (ví dụ `trading-chart` với tool ghi Wiki và tool tạo Markdown).
- Gợi ý skill cho một Built-in Sub-Agent không còn chứa skill mà Sub-Agent đó không thể có đủ tool; bỏ `daily-synthesis-staging` khỏi gợi ý của `media_creator` và `media_processor`.
- Thêm test bất biến: với phân bổ mặc định, không Built-in Sub-Agent nào có khoảng trống tool.

Nhờ ticket 01, deployment chưa sửa `wiki-query` tự nhận bản mới; `wiki-synthesis-filing` được ghi ra như skill mới.

Xem spec: [Skill Tool Requirement](../spec.md#skill-tool-requirement-adr-0144).

**Blocked by:** 10, 01

**Status:** ready-for-agent

- [ ] Test bất biến qua: mọi Built-in Sub-Agent có danh sách khoảng trống rỗng với cấu hình mặc định
- [ ] `wiki-query` không còn nhắc tới tool ghi Wiki; `wiki_researcher` không có khoảng trống
- [ ] `wiki-synthesis-filing` là Built-in Job Skill, không xoá được, gán mặc định cho `wiki_curator`, và mọi tool nó khai báo đều thuộc tool của `wiki_curator`
- [ ] Mọi Built-in Job Skill được gán mặc định cho ít nhất một Sub-Agent đều khai báo `requires_tools`
- [ ] Gợi ý của `media_creator` và `media_processor` không còn `daily-synthesis-staging`
- [ ] Danh sách hash bản mặc định cũ của ticket 01 không cần sửa: bản `wiki-query` trước thay đổi này vẫn được nhận là chưa sửa nhờ manifest hoặc danh sách đã có
- [ ] Bộ skill của kit Wiki dành cho client agent trong `docs/knowledge/` không bị đổi
