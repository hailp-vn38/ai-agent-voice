# 12: Admin Web hiện yêu cầu tool của skill và cảnh báo khoảng trống

**What to build:** Trang chi tiết skill hiện danh sách tool skill cần; trình soạn skill cho Admin sửa danh sách này với Custom Job Skill và Built-in Job Skill. Trình chỉnh skill của Sub-Agent hiện cảnh báo cạnh mỗi skill có khoảng trống tool, nêu tool còn thiếu. Cảnh báo không chặn nút lưu.

Xem spec: [Skill Tool Requirement](../spec.md#skill-tool-requirement-adr-0144).

**Blocked by:** 10

**Status:** resolved

- [x] Trang chi tiết skill hiện danh sách tool cần; rỗng thì ghi rõ skill không khai báo
- [x] Trình soạn thêm, bớt tool cần và lưu được; lỗi tên sai định dạng hiện đúng ở trường đó
- [x] Sub-Agent có khoảng trống: cảnh báo hiện cạnh đúng skill, nêu đúng tool thiếu
- [x] Cảnh báo cập nhật sau khi lưu cấu hình làm khoảng trống mất đi
- [x] Nút lưu vẫn dùng được khi có cảnh báo
- [x] Có chuỗi i18n và test component

## Comments

- 2026-11-11: Admin Web hiển thị và chỉnh sửa `requires_tools` cho Job Skill. Sub-Agent skill picker hiển thị tool còn thiếu theo từng skill; cảnh báo không chặn lưu và cập nhật từ projection sau khi cấu hình được lưu.
- Validation: targeted `BackgroundPage.test.ts` PASS (9/9); `scripts/agent-test.sh web typecheck` PASS; `scripts/agent-test.sh web test` PASS (100 files / 569 tests); `git diff --check` PASS.
