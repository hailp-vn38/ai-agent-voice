# 13: Cập nhật GLOSSARY và trạng thái ADR

**What to build:** Thêm ba thuật ngữ vào `GLOSSARY.md` theo đúng định dạng đang dùng (định nghĩa một câu, dòng `_Avoid_`): **Required Job Skill**, **Skill Tool Requirement**, **Skill Base Version**. Sửa các mục Job Skill hiện có nếu chúng mâu thuẫn với hành vi mới (ví dụ Built-in Job Skill giờ tự nhận bản mặc định mới khi Admin chưa sửa).

Sửa ADR 0128: số Built-in Job Skill, và câu nói việc xoá skill không ghi cấu hình Sub-Agent (code hiện tại có gỡ ID skill bị xoá khỏi cấu hình đã lưu). Chuyển ADR 0142, 0143, 0144 từ Proposed sang Accepted với ngày hoàn thành, và ghi vào ADR 0128 rằng nó được ADR 0142 sửa đổi.

Xem spec: [Thuật ngữ](../spec.md#thuật-ngữ), [Further Notes](../spec.md#further-notes).

**Blocked by:** 02, 05, 07, 08, 11, 12

**Status:** ready-for-agent

- [ ] `GLOSSARY.md` có ba thuật ngữ mới, mỗi thuật ngữ có dòng `_Avoid_`
- [ ] Không mục Job Skill nào trong `GLOSSARY.md` mâu thuẫn với hành vi đã triển khai
- [ ] ADR 0128 ghi đúng số Built-in Job Skill và đúng hành vi khi xoá skill, có dòng tham chiếu tới ADR 0142
- [ ] ADR 0142, 0143, 0144 ở trạng thái Accepted
- [ ] Spec này được đánh dấu hoàn thành
