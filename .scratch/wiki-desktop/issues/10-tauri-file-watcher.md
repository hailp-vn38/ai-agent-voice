# 10: Tauri shell: file watcher

Status: ready-for-human
Blocked by: 03, 09
Spec: ../spec.md (user stories 31–33)

## Mục tiêu

App nhận thay đổi do Obsidian, Git hay iCloud tạo ra mà không cần khởi động lại.

## Phạm vi

- Dùng `notify` (FSEvents) trên thư mục kho, debounce khoảng 300 ms, bỏ qua lock file, file SQLite và file tạm của `Vault`.
- Gom các path thay đổi rồi gọi `Vault::notice_external`. UI nhận `External` qua `vault://changed`.
- Đổi kho thì dừng watcher cũ và khởi động watcher mới.

## Tiêu chí xong

- [ ] Smoke test thủ công: sửa trang trong Obsidian thì trong vòng khoảng 1 giây cây, trang và kết quả tìm kiếm cập nhật; `git checkout` một nhánh khác thì app phản ánh đúng.
- [ ] Lưu trong app không gây nhấp nháy hay báo lặp "thay đổi bên ngoài".

## Cập nhật tiến độ

- Đã thêm notify watcher đệ quy, debounce 300 ms, lọc lock/SQLite/temp và wiki/index.md, wiki/log.md; đổi kho thì dừng watcher cũ và mở watcher mới.
- PASS: Tauri `cargo check`, `cargo fmt --check`, `git diff --check`.
- NOT_RUN: smoke Obsidian/Git checkout và xác nhận app write không phát External lặp.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

- [x] Khi app ở chế độ chỉ đọc (`wiki-mcp` direct giữ lock), `notice_external` trả `ReadOnly` (`wiki-core/src/events.rs:18`). Thay đổi từ ngoài chỉ được ghi log, không tới UI. Reader vẫn cần refresh index/cache và phát `vault://changed`.
- [x] Ngoài phạm vi: watcher bỏ qua cả `wiki/index.md` và `wiki/log.md` (`watcher.rs:105`). Spec chỉ liệt kê lock, file SQLite và file tạm. Xác nhận có chủ ý hay bỏ.

### Sửa sau review (2026-11-08)

- `Vault::notice_external` không còn trả `ReadOnly`: reader vẫn sync index riêng và phát `VaultEvent::External`, chỉ không ghi lại `wiki/index.md`. Test: `a_read_only_vault_reports_external_changes_without_rewriting_the_index` (wiki-core).
- Giữ việc bỏ qua `wiki/index.md`/`wiki/log.md` (file dẫn xuất do `Vault` tự ghi) và ghi lý do vào `desktop-shell.md`.
