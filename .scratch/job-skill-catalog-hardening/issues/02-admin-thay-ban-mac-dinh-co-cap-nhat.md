# 02: Admin thấy Built-in Job Skill nào có bản mặc định mới hơn

**What to build:** Tóm tắt skill trong Admin API thêm cờ "bản mặc định có cập nhật": đúng khi skill đang khác bản mặc định và Skill Base Version khác bản mặc định hiện tại. Cờ "đã sửa" giữ nguyên nghĩa. Admin Web hiện hai trạng thái này bằng hai huy hiệu riêng ở trang danh sách và trang chi tiết skill; trang chi tiết đã có sẵn bản mặc định để so sánh và khôi phục.

Xem spec: [Nâng cấp Built-in Job Skill](../spec.md#nâng-cấp-built-in-job-skill-adr-0142).

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] Skill Admin đã sửa, bản mặc định không đổi từ lúc sửa: "đã sửa" bật, "bản mặc định có cập nhật" tắt
- [ ] Skill Admin đã sửa, sau đó bản mặc định đổi: cả hai cờ bật
- [ ] Skill chưa sửa: cả hai cờ tắt, kể cả ngay sau khi được nâng cấp
- [ ] Admin lưu lại skill (giữ nội dung riêng hoặc khôi phục mặc định): "bản mặc định có cập nhật" tắt
- [ ] Custom Job Skill luôn có cả hai cờ tắt
- [ ] Admin Web hiện hai huy hiệu riêng biệt, có chuỗi i18n cho cả hai, và có test component
