# 10: Admin API báo skill được gán cho Sub-Agent thiếu tool

**What to build:** Tóm tắt skill và payload lưu skill trong Admin API thêm `requires_tools`. Projection cấu hình Sub-Agent thêm danh sách khoảng trống tool: mỗi mục gồm ID skill và các tool còn thiếu, tính bằng `requires_tools` của từng skill trong `allowed_skills` hiệu lực trừ đi `allowed_tools` hiệu lực. Đây là cảnh báo; lưu cấu hình không bị từ chối. Khai báo không cấp quyền.

Ticket này chưa khai báo `requires_tools` cho Built-in Job Skill; ticket 11 làm việc đó.

Xem spec: [Skill Tool Requirement](../spec.md#skill-tool-requirement-adr-0144).

**Blocked by:** 09

**Status:** ready-for-agent

- [ ] Tạo Custom Job Skill với `requires_tools`; đọc lại thấy đúng danh sách; ETag đổi khi danh sách đổi
- [ ] Gán skill đó cho Sub-Agent thiếu một tool: lưu thành công, projection có một mục khoảng trống nêu đúng tool thiếu
- [ ] Cấp thêm tool còn thiếu cho Sub-Agent: mục khoảng trống biến mất
- [ ] Gán skill không làm `allowed_tools` hiệu lực của Sub-Agent thay đổi (ngoài tool `skill` như hiện tại)
- [ ] Skill không khai báo `requires_tools` không bao giờ tạo khoảng trống
- [ ] `requires_tools` chứa tên sai định dạng bị từ chối với mã lỗi riêng
- [ ] Custom Sub-Agent cũng có danh sách khoảng trống
