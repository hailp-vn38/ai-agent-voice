# 06: Required Job Skill nằm sẵn trong system prompt của Sub-Agent

**What to build:** Built-in Sub-Agent có danh sách Required Job Skill cố định trong code: `wiki_ingestor` có `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`; `wiki_curator` có `wiki-lint`, `wiki-query`; các Sub-Agent khác không có. Server ghép nội dung các skill này vào file prompt của Sub-Agent thành một khối do server sở hữu, đặt sau prompt của Admin hoặc prompt mặc định, mở đầu bằng câu nói rõ các skill đã được nạp, phải làm theo trừ khi trái với prompt phía trên, và không cần gọi `skill` cho chúng.

Khối được ghi lại lúc khởi động, khi lưu skill và khi lưu cấu hình Sub-Agent. Prompt đã lưu và prompt Admin thấy không chứa khối. Required Job Skill luôn có trong `allowed_skills` hiệu lực; `allowed_skills` Admin gửi thiếu chúng thì server tự thêm lại. Projection cấu hình Sub-Agent trong Admin API thêm danh sách Required Job Skill.

Chỉ đụng server. Tiền lệ: khối danh sách Sub-Agent trong prompt của Root Orchestrator, và `wiki-chat` trong Wiki Ask.

Xem spec: [Required Job Skill](../spec.md#required-job-skill-adr-0143); [ADR 0143](../../../docs/adr/0143-required-job-skills-preloaded-into-agent-prompts.md).

**Blocked by:** không

**Status:** ready-for-agent

- [ ] Sau khi dựng app, file prompt của `wiki_ingestor` chứa nội dung hiện tại của cả ba Required Job Skill; file prompt của `wiki_curator` chứa hai skill của nó; các Sub-Agent khác không có khối
- [ ] Admin sửa nội dung một Required Job Skill qua Admin API: file prompt chứa nội dung mới mà không cần dựng lại app
- [ ] Admin đặt system prompt riêng cho `wiki_ingestor`: file prompt gồm prompt riêng đó rồi tới khối; Admin API trả lại đúng prompt Admin đã gửi, không có khối
- [ ] Prompt đã lưu chứa sẵn một bản chụp cũ của khối: file prompt chỉ có một khối, với nội dung hiện tại
- [ ] Lưu `allowed_skills` của `wiki_ingestor` thiếu `wiki-ingest` thành công và cấu hình hiệu lực vẫn có `wiki-ingest`
- [ ] Projection của mỗi Sub-Agent có danh sách Required Job Skill; rỗng với Root Orchestrator và Custom Sub-Agent
- [ ] Required Job Skill không có bản hợp lệ trên đĩa: khối dùng nội dung bản nhúng
- [ ] Test bất biến: mọi Required Job Skill là Built-in Job Skill tồn tại
