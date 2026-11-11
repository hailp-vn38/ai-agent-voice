# 05: Admin thấy và sửa được Custom Job Skill hỏng

**What to build:** Danh sách skill trong Admin API trả thêm các mục bị thư viện bỏ qua, với trạng thái `invalid` và lý do; mục hợp lệ có trạng thái `ok`. Admin xoá được một Custom Job Skill `invalid`, hoặc ghi đè nó bằng nội dung hợp lệ, với `If-Match` là ETag tính trên nội dung file thô. `If-None-Match: *` không ghi đè được mục `invalid`.

Admin Web hiện mục `invalid` trong danh sách kèm lý do, cho xoá, và cho mở trình soạn để ghi đè.

Xem spec: [Custom Job Skill hỏng trong Admin API](../spec.md#custom-job-skill-hỏng-trong-admin-api).

**Blocked by:** 04

**Status:** ready-for-human

- [x] Custom Job Skill sai frontmatter hiện trong danh sách với trạng thái `invalid`, lý do và ETag
- [x] Thư mục rỗng và thư mục tên sai định dạng hiện là `invalid` với lý do tương ứng
- [x] Xoá mục `invalid` bằng ETag đúng thành công và mục biến mất; ETag sai trả `revision_conflict`
- [x] Ghi đè mục `invalid` bằng nội dung hợp lệ với `If-Match` đúng: skill thành `ok` và Sub-Agent được gán nó nạp được
- [x] Tạo mới bằng `If-None-Match: *` trên ID đang `invalid` trả `already_exists`
- [x] Không Built-in Job Skill nào từng hiện là `invalid`
- [x] Mục `invalid` không gán được cho Sub-Agent (`invalid_arguments`)
- [x] Admin Web hiện lý do, có nút xoá và ghi đè, có chuỗi i18n và test component

## Comments

- 2026-11-12: API integration, Admin Web component, typecheck, full web suite, and server check pass. Pending human review/publication gate inherited from ticket 04.
