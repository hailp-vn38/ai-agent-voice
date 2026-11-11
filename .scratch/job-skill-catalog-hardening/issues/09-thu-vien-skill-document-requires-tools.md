# 09: Tài liệu skill của thư viện mang `requires_tools`

**What to build:** Trong repo thư viện orchestrator-agent, tài liệu skill nhận thêm trường frontmatter tuỳ chọn `requires_tools`: danh sách tên tool cách nhau bằng dấu phẩy trên một dòng. Trường này đi qua parse và render không suy hao. Danh sách rỗng không được render ra, để tài liệu không có trường này giữ nguyên kết quả render như trước. Tên phải đúng định dạng tên tool; thư viện không kiểm tra tool có tồn tại hay không, và trường này không ảnh hưởng tới quyền hay việc nạp skill.

Có thể phát hành chung một phiên bản với ticket 03. **Publish lên crates.io cần người duyệt.**

Xem spec: [Skill Tool Requirement](../spec.md#skill-tool-requirement-adr-0144); [ADR 0144](../../../docs/adr/0144-job-skill-tool-requirements.md).

**Blocked by:** không

**Status:** ready-for-agent

- [ ] Parse rồi render một tài liệu có `requires_tools` cho lại đúng danh sách đó, đúng thứ tự
- [ ] Tài liệu không có `requires_tools` render ra đúng từng byte như phiên bản thư viện trước
- [ ] Tên tool sai định dạng là lỗi trường không hợp lệ; tên trùng lặp bị gộp
- [ ] Skill có `requires_tools` nạp được bởi agent không có các tool đó
- [ ] Nội dung trả về cho agent khi nạp skill không đổi so với trước
- [ ] Phiên bản mới được publish và ghi vào ticket này
