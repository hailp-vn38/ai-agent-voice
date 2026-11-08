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
