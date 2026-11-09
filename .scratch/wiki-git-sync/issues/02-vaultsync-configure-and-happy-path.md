# 02: `VaultSync`: configure và sync không xung đột

Status: resolved
Blocked by: 01
Spec: ../spec.md (user stories 1, 2, 3, 7, 8, 9, 11)
Thiết kế: docs/wiki-desktop/git-sync.md

## Mục tiêu

Có module `wiki-core::sync::VaultSync` chạy được một vòng commit → fetch → merge → push khi không có xung đột.

## Phạm vi

- Kiểu `SyncConfig`, `SyncStatus`, `SyncReport`, `SyncError` (derive `serde`) theo thiết kế.
- `GitAuth::System` (desktop) và `GitAuth::SshKey { key, known_hosts }` (server, headless: `GIT_SSH_COMMAND` với `-i`, `IdentitiesOnly=yes`, `UserKnownHostsFile`, `StrictHostKeyChecking=yes`; `GIT_CONFIG_GLOBAL=/dev/null`; `-c safe.directory=<vault>`; tác giả lấy từ `SyncConfig`).
- `SyncStatus::Deferred` có trong kiểu dữ liệu (server dùng ở ticket 08).
- Một runner git nội bộ: `std::process::Command`, env `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=`, `LC_ALL=C`, timeout có kill, lọc chuỗi giống credential khỏi stderr.
- `configure`: phát hiện thiếu git (`Disabled { NoGit }`), `git init -b <branch>` nếu chưa có repo, bổ sung `.gitignore`/`.gitattributes` mà không xóa dòng có sẵn, đặt `origin`, từ chối URL có credential (`UnsafeRemote`), `git ls-remote` để báo `Auth`/`Network` sớm.
- `sync_now`: kiểm tra file quá `max_file_bytes`, commit (message có hostname và origin), fetch, merge `--no-edit`, trả path đổi qua `exclusive_external`, push, thử lại một lần khi push bị từ chối, tính ahead/behind.
- Remote trống (repo mới tạo) thì push lần đầu với `-u`.
- `subscribe` phát `SyncStatus` mỗi khi đổi.

## Tiêu chí xong

- [x] Test với bare repo + hai clone: ghi ở A, sync A, sync B → B đọc và search thấy, nhận `External`.
- [x] Test: `.wiki/index.sqlite` và `.wiki-desktop.lock` không có trong `git ls-files`.
- [x] Test: file lớn hơn giới hạn → `FileTooLarge`, không có commit mới.
- [x] Test: URL `https://u:t@host/r.git` → `UnsafeRemote`.
- [x] Test: vault reader → `ReadOnly`; không tìm thấy git trên PATH → `Disabled { NoGit }`.
- [x] Test: `GitAuth::SshKey` sinh đúng env/args; runner chạy được khi `HOME` không có.
- [x] Test bỏ qua có in `SKIPPED` khi máy không có git.
- [x] `scripts/agent-test.sh package wiki-core` PASS.
