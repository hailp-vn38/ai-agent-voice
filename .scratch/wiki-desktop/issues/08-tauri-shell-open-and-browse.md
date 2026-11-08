# 08: Tauri shell: mở kho và duyệt chỉ đọc

Status: ready-for-human
Blocked by: 01, 07
Spec: ../spec.md (user stories 2–7, 9–21, 52)

## Mục tiêu

Dựng app `apps/wiki-desktop` (Tauri 2 + Vue). App mở được kho tri thức và hiển thị UI Knowledge từ package dùng chung ở chế độ chỉ đọc.

## Phạm vi

- Crate Rust của app **bị exclude khỏi workspace gốc**, có lockfile riêng, tham chiếu `wiki-core` bằng `path`.
- Command: `vault_open`, `vault_pick` (hộp thoại chọn thư mục), `vault_tree`, `vault_read`, `vault_search`, `vault_graph`, `vault_status`.
- `TauriBackend` gọi bằng `invoke`. `capabilities.chat = false`.
- Lần mở đầu tiên có màn chọn kho. Danh sách kho gần đây lưu trong Application Support. Mở lại kho cuối cùng khi khởi động. `⌘O`.
- Capabilities của WebView chỉ gồm dialog, event và các command trên. Không cấp `fs` hay `shell`. CSP không cho script ngoài.
- Cửa sổ dùng overlay title bar, vibrancy, theo dark mode. Phím tắt `⌘P`, `⌘⇧F`, `⌘G`, `⌘⇧N`.

## Tiêu chí xong

- [ ] `cargo` của workspace gốc không build Tauri. Test server không bị ảnh hưởng.
- [ ] Smoke test thủ công: mở một kho có sẵn của `vela-server`, duyệt `raw`/`wiki`, đọc trang có Mermaid, theo Wiki Link, xem Backlink và Graph, tìm không dấu.
- [ ] Lỗi `VaultError` hiện thành thông báo có kiểu trên UI, không phải chuỗi thô.

## Cập nhật tiến độ

- Đã dựng Tauri shell, TauriBackend, mở/chọn/gần đây/kho gần nhất, truy vấn Vault, CSP/capability và giao diện macOS.
- PASS: app `cargo check`, `cargo fmt --check`, Vite build, `scripts/agent-test.sh check vela-server`, Knowledge UI tests, full Admin Web tests (556/556) và `web typecheck`.
- NOT_RUN: smoke test app thật với kho `vela-server` (raw/wiki, Mermaid, Wiki Link, Backlink, Graph, tìm không dấu); chưa xác nhận bằng giao diện các thông báo lỗi có kiểu.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

- [x] Capability quá rộng: spec ghi "Capabilities của WebView chỉ gồm dialog, event và các command trên", nhưng `capabilities/default.json` cấp cả `core:default` (window, webview, app, menu, tray, path…). Thu hẹp lại còn `core:event` + `dialog:allow-open`.
- [x] Thiếu "overlay title bar, vibrancy": `tauri.conf.json` mới đặt `transparent`/`macOSPrivateApi`, chưa áp window effect.
- [x] Shell chưa mỏng (`desktop-shell.md`: command dài hơn ~15 dòng thì đưa logic vào core): `open_vault_state` (`main.rs:96-131`) quản lý danh sách vault gần đây.
- [x] `vault_recent`, `vault_current`, `vault_is_writable` không có trong danh sách command của `desktop-shell.md`. Cập nhật doc hoặc bỏ command.

### Sửa sau review (2026-11-08)

- Capability chỉ còn `core:event:default` + `dialog:allow-open`, áp cho `main` và `wiki-*`. Cửa sổ do Rust tạo (`build_window`), không còn cần quyền tạo cửa sổ từ WebView.
- Cửa sổ có overlay title bar, `transparent` và vibrancy (`Effect::Sidebar`, theo trạng thái active). `tauri.conf.json` không khai báo cửa sổ nữa; `main` được tạo trong `setup`.
- `open_vault_state` gọi `Settings::remember_vault`; lock dùng helper `locked()`.
- `vault_recent` + `vault_current` gộp thành `vault_session()` (vault đang mở, gần đây, quyền ghi); `desktop-shell.md` liệt kê `vault_session`, `vault_is_writable`, `window_new`.
- Sửa thêm: màn welcome dùng `v-else` sau hộp thoại MCP nên hiện cả khi đã mở vault; giờ là `v-if="!current"`.
- NOT_RUN: chưa mở app thật để xác nhận vibrancy và cửa sổ hiển thị đúng.
