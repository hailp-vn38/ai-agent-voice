# 08: Tauri shell: mở kho và duyệt chỉ đọc

Status: ready-for-agent
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
