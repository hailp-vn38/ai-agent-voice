# 04: Tauri shell: command, event và lịch sync

Status: ready-for-agent
Blocked by: 02
Spec: ../spec.md (user stories 2, 3, 4, 10)
Thiết kế: docs/wiki-desktop/git-sync.md#tauri-shell

## Mục tiêu

App tự sync theo lịch và WebView điều khiển được sync.

## Phạm vi

- `Settings` thêm `sync: HashMap<String, SyncSettings { auto, interval_min }>` (`#[serde(default)]`).
- Command `sync_status`, `sync_now` (spawn_blocking), `sync_configure`, `sync_disable`; thêm vào `capabilities/default.json`.
- Event `sync://status` chuyển tiếp từ `VaultSync::subscribe`.
- Task lịch cho mỗi vault (dùng `TaskCancel` sẵn có, hủy khi đổi vault): sync khi mở, debounce 30 s sau `VaultEvent::Changed` origin Ui/Mcp, định kỳ `interval_min`, sync tối đa 10 s khi thoát nếu có thay đổi chưa commit.
- Khi quyền ghi đổi (`vault://lock`) thì bật/tắt lịch theo.
- Mỗi command giữ dưới khoảng 15 dòng; logic nằm trong `wiki-core`.

## Tiêu chí xong

- [ ] Test Rust của shell: settings cũ không có `sync` vẫn đọc được.
- [ ] Tauri `cargo check`, `cargo fmt --check`, `git diff --check` PASS.
- [ ] Ghi log tiến độ nêu rõ phần chưa smoke (để ticket 06).
