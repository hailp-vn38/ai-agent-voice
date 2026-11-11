# 12: Admin Web hiện yêu cầu tool của skill và cảnh báo khoảng trống

**What to build:** Trang chi tiết skill hiện danh sách tool skill cần; trình soạn skill cho Admin sửa danh sách này với Custom Job Skill và Built-in Job Skill. Trình chỉnh skill của Sub-Agent hiện cảnh báo cạnh mỗi skill có khoảng trống tool, nêu tool còn thiếu. Cảnh báo không chặn nút lưu.

Xem spec: [Skill Tool Requirement](../spec.md#skill-tool-requirement-adr-0144).

**Blocked by:** 10

**Status:** ready-for-agent

- [ ] Trang chi tiết skill hiện danh sách tool cần; rỗng thì ghi rõ skill không khai báo
- [ ] Trình soạn thêm, bớt tool cần và lưu được; lỗi tên sai định dạng hiện đúng ở trường đó
- [ ] Sub-Agent có khoảng trống: cảnh báo hiện cạnh đúng skill, nêu đúng tool thiếu
- [ ] Cảnh báo cập nhật sau khi lưu cấu hình làm khoảng trống mất đi
- [ ] Nút lưu vẫn dùng được khi có cảnh báo
- [ ] Có chuỗi i18n và test component
