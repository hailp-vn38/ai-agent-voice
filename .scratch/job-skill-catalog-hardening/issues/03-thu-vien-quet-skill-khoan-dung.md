# 03: Thư viện orchestrator-agent quét skill khoan dung ở mọi đường

**What to build:** Trong repo thư viện orchestrator-agent:

- Công khai một hàm quét thư mục skill ở chế độ khoan dung, trả về tóm tắt các skill hợp lệ và danh sách mục bị bỏ qua kèm lý do. Build `Orchestrator` và `reload_skills` dùng chính hàm này.
- Build `Orchestrator` không còn thất bại vì một mục skill hỏng, trùng tên, tên sai hay thư mục thiếu `SKILL.md`. Thư mục thiếu `SKILL.md` bị bỏ qua.
- Build và `reconfigure` không còn thất bại khi một agent tham chiếu skill không có trong catalog: quyền được thu hẹp về skill đang có, kèm cảnh báo.

Làm và test trong repo thư viện; thử từ server bằng `[patch.crates-io]` tạm. **Publish lên crates.io cần người duyệt.**

Xem spec: [Một cách quét Skill Catalog](../spec.md#một-cách-quét-skill-catalog-adr-0142); [ADR 0142](../../../docs/adr/0142-single-skill-discovery-and-builtin-skill-upgrades.md).

**Blocked by:** không

**Status:** ready-for-human

- [x] Hàm quét công khai trả cùng tập skill mà một `Orchestrator` build trên cùng thư mục cho phép nạp
- [x] Build thành công trên thư mục có: một `SKILL.md` sai frontmatter, một thư mục rỗng, một thư mục tên sai định dạng, hai skill trùng tên; các skill hợp lệ còn lại nạp được
- [x] Mục symlink bị từ chối (bỏ qua), không bao giờ được nạp
- [x] Build và `reconfigure` với agent tham chiếu skill không tồn tại thành công; ID bị loại khỏi quyền hiệu lực
- [x] Thư mục skill không tồn tại vẫn là lỗi cấu hình lúc build
- [x] Run đang chạy giữ snapshot skill cũ sau `reload_skills`, như hiện tại
- [ ] Publish phiên bản mới và ghi số phiên bản vào ticket này (cần người duyệt)

## Comments

- 2026-11-11: Implementation committed as `f057a99` (`feat: make skill catalog scanning tolerant`) and fast-forwarded to the library integration branch `/tmp/job-skill-catalog-hardening/agent-integration`. `CHECK_MAX_LINES=120 scripts/check.sh fast` passed fmt, clippy, clippy-lib, and all tests. Crates.io publication is pending reviewer approval.
