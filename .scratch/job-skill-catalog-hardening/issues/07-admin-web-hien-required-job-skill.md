# 07: Admin Web hiện Required Job Skill là không bỏ gán được

**What to build:** Trong trình chỉnh skill của một Sub-Agent, Required Job Skill hiện là đã chọn và bị khoá, kèm chú thích rằng nội dung luôn nằm trong system prompt của Sub-Agent này. Admin vẫn bấm vào để mở trang chi tiết và sửa nội dung. Trang chi tiết skill ghi Sub-Agent nào coi nó là Required Job Skill.

Xem spec: [Required Job Skill](../spec.md#required-job-skill-adr-0143).

**Blocked by:** 06

**Status:** resolved

- [x] Required Job Skill hiện đã chọn, không bỏ chọn được, có chú thích
- [x] Lưu cấu hình Sub-Agent từ Admin Web không gửi thay đổi nào làm mất Required Job Skill
- [x] Trang chi tiết skill liệt kê các Sub-Agent yêu cầu skill đó
- [x] Skill thường vẫn chọn và bỏ chọn được như cũ
- [x] Có chuỗi i18n và test component
