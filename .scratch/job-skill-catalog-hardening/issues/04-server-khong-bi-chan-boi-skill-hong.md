# 04: Server khởi động và lưu cấu hình được khi thư mục skill có mục hỏng

**What to build:** Server dùng phiên bản thư viện của ticket 03 và bỏ phần quét skill riêng. Thành viên Skill Catalog (kiểm tra khi lưu cấu hình, `allowed_skills` hiệu lực, danh sách cho Admin) lấy từ hàm quét của thư viện; bản nhúng không còn được coi là skill tồn tại khi file trên đĩa hỏng.

Built-in Job Skill được tự phục hồi lúc khởi động và trước mỗi lần ghi skill: thiếu thì ghi bản mặc định, không parse được thì giữ nội dung cũ ở một file ẩn cạnh nó rồi ghi bản mặc định. Lần lưu skill thất bại dọn thư mục nó vừa tạo. Deployment không cấu hình thư mục skill có Skill Catalog rỗng ở mọi route; Wiki Ask vẫn lấy `wiki-chat` từ bản nhúng.

Thay `[patch.crates-io]` bằng phiên bản đã publish trước khi merge.

Xem spec: [Một cách quét Skill Catalog](../spec.md#một-cách-quét-skill-catalog-adr-0142).

**Blocked by:** 03

**Status:** ready-for-human

- [x] Dựng app thành công khi thư mục skill chứa một Custom Job Skill sai frontmatter, một thư mục rỗng và một thư mục tên sai định dạng
- [x] Dựng app với một Built-in Job Skill không parse được: sau đó file bằng bản mặc định, nội dung cũ còn nguyên ở file ẩn cạnh nó, và file ẩn không có trong danh sách file liên quan
- [x] Với một Custom Job Skill hỏng trên đĩa, lưu cấu hình Sub-Agent và lưu LLM Provider đều thành công
- [x] Sub-Agent được gán một Custom Job Skill đang hỏng: cấu hình hiệu lực không còn skill đó, các skill khác giữ nguyên
- [x] Danh sách skill của Admin API trùng với tập skill hợp lệ mà thư viện quét được
- [x] Không cấu hình thư mục skill: route danh sách trả rỗng và route chi tiết trả `not_found` cho mọi ID; Wiki Ask vẫn có nội dung `wiki-chat`
- [x] Mô phỏng lưu thất bại sau khi tạo thư mục: không còn thư mục của skill đó
- [x] Hành vi nâng cấp của ticket 01 không đổi nếu ticket 01 đã merge

## Comments

- 2026-11-11: Server implementation and targeted tests are committed on `ticket/job-skill-04`. The temporary `[patch.crates-io]` dependency on ticket 03 is still pending publication approval, so this ticket is ready for human release coordination.
