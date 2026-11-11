# 02: Admin thấy Built-in Job Skill nào có bản mặc định mới hơn

**What to build:** Tóm tắt skill trong Admin API thêm cờ "bản mặc định có cập nhật": đúng khi skill đang khác bản mặc định và Skill Base Version khác bản mặc định hiện tại. Cờ "đã sửa" giữ nguyên nghĩa. Admin Web hiện hai trạng thái này bằng hai huy hiệu riêng ở trang danh sách và trang chi tiết skill; trang chi tiết đã có sẵn bản mặc định để so sánh và khôi phục.

Xem spec: [Nâng cấp Built-in Job Skill](../spec.md#nâng-cấp-built-in-job-skill-adr-0142).

**Blocked by:** 01

**Status:** resolved

- [x] Skill Admin đã sửa, bản mặc định không đổi từ lúc sửa: "đã sửa" bật, "bản mặc định có cập nhật" tắt
- [x] Skill Admin đã sửa, sau đó bản mặc định đổi: cả hai cờ bật
- [x] Skill chưa sửa: cả hai cờ tắt, kể cả ngay sau khi được nâng cấp
- [x] Admin lưu lại skill (giữ nội dung riêng hoặc khôi phục mặc định): "bản mặc định có cập nhật" tắt
- [x] Custom Job Skill luôn có cả hai cờ tắt
- [x] Admin Web hiện hai huy hiệu riêng biệt, có chuỗi i18n cho cả hai, và có test component

## Comments

- 2026-11-11: Added `default_has_update` to the Admin skill summary, derived from the normalized current built-in hash and the stored Skill Base Version, and showed a separate badge on the catalog and detail pages. Tests passed: `scripts/agent-test.sh targeted background_skill_summary_reports_default_updates_separately_from_edits vela-server` (1/1), `scripts/agent-test.sh targeted background_skills_crud_etag_preconditions_and_agent_visibility vela-server` (1/1), targeted `BackgroundPage.test.ts` (8/8), and `scripts/agent-test.sh web typecheck`. Scoped rustfmt and `git diff --check` passed. Workspace-wide `cargo fmt --all -- --check` reports formatting differences in unrelated existing files; they were left untouched.
