# 13: Cập nhật GLOSSARY và trạng thái ADR

**What to build:** Thêm ba thuật ngữ vào `GLOSSARY.md` theo đúng định dạng đang dùng (định nghĩa một câu, dòng `_Avoid_`): **Required Job Skill**, **Skill Tool Requirement**, **Skill Base Version**. Sửa các mục Job Skill hiện có nếu chúng mâu thuẫn với hành vi mới (ví dụ Built-in Job Skill giờ tự nhận bản mặc định mới khi Admin chưa sửa).

Sửa ADR 0128: số Built-in Job Skill, và câu nói việc xoá skill không ghi cấu hình Sub-Agent (code hiện tại có gỡ ID skill bị xoá khỏi cấu hình đã lưu). Chuyển ADR 0142, 0143, 0144 từ Proposed sang Accepted với ngày hoàn thành, và ghi vào ADR 0128 rằng nó được ADR 0142 sửa đổi.

Xem spec: [Thuật ngữ](../spec.md#thuật-ngữ), [Further Notes](../spec.md#further-notes).

**Blocked by:** 02, 05, 07, 08, 11, 12

**Status:** resolved

- [x] `GLOSSARY.md` có ba thuật ngữ mới, mỗi thuật ngữ có dòng `_Avoid_`
- [x] Không mục Job Skill nào trong `GLOSSARY.md` mâu thuẫn với hành vi đã triển khai
- [x] ADR 0128 ghi đúng số Built-in Job Skill và đúng hành vi khi xoá skill, có dòng tham chiếu tới ADR 0142
- [x] ADR 0142, 0143, 0144 ở trạng thái Accepted
- [ ] Spec này được đánh dấu hoàn thành sau khi các thư viện cần thiết được phát hành và thay patch tạm bằng phiên bản đã phát hành

## Answer

Đã cập nhật glossary và ADR theo hành vi đã tích hợp. Spec vẫn `ready-for-human` vì phát hành các crate lên crates.io cần người duyệt theo `AGENTS.md` và spec.

- 2026-11-12: Hoàn tất chỉnh sửa tài liệu; còn cổng phát hành thư viện trước khi đánh dấu spec hoàn thành.
