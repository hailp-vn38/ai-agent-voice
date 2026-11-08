# Tauri shell: apps/wiki-desktop

Shell là một module **nông có chủ đích**: nó chỉ nối các phần lại với nhau (WebView ↔ `Vault`, `ToolHost` ↔ HTTP, watcher ↔ `Vault`). Mọi logic nghiệp vụ nằm trong `wiki-core` và `wiki-mcp`. Nếu một command Tauri dài quá khoảng 15 dòng thì phần thừa nên chuyển vào core.

## Cấu trúc

```
apps/wiki-desktop/
├─ package.json            # Vite + Vue 3, import knowledge UI (xem ui-reuse.md)
├─ src/                    # main.ts, TauriBackend adapter, Settings view
└─ src-tauri/
   ├─ Cargo.toml           # crate độc lập, bị exclude khỏi workspace gốc; có Cargo.lock riêng
   ├─ tauri.conf.json
   ├─ capabilities/default.json
   └─ src/
      ├─ main.rs           # setup, kích hoạt Vault, chuyển tiếp event, watcher và lock retry
      ├─ mcp_commands.rs   # lệnh MCP và preview/cài cấu hình Claude Desktop
      ├─ mcp_server.rs     # rmcp HTTP + runtime.json
      └─ watcher.rs        # notify → Vault::notice_external
```

## Commands (WebView → Rust)

Chỉ có đúng sáu command, tương ứng với interface `Vault`:

| Command | Gọi tới | Thay cho route admin |
|---|---|---|
| `vault_tree(scope)` | `Vault::tree` | `GET /knowledge/tree`, `/folders` |
| `vault_read(path, window)` | `Vault::read` | `GET /knowledge/pages/:path`, `/sources/:path` |
| `vault_search(query)` | `Vault::search` | `GET /knowledge/index?q=` |
| `vault_graph(focus)` | `Vault::graph` | `GET /knowledge/graph` |
| `vault_status()` | `Vault::status` | — |
| `vault_apply(change)` | `Vault::apply_from(Origin::Ui, …)` | `PUT/POST/DELETE pages`, `/raw/batch`, `/raw/move`, `/raw/import-source`, `/index/rebuild` |

Thêm command của riêng shell: `vault_open(dir)` / `vault_pick()` (hộp thoại chọn thư mục), `vault_is_writable()`, `mcp_info()` (port, trạng thái, đoạn cấu hình client), `mcp_set_allow_write()`, `mcp_set_port()`, `mcp_rotate_token()`, `mcp_claude_config_preview()` và `mcp_claude_config_install()`.

`Change` và `VaultError` derive `serde`, nên frontend nhận lỗi có kiểu (`{ kind: "Conflict", current }`) thay vì chuỗi văn bản.

## Events (Rust → WebView)

- `vault://changed`: `VaultEvent` đã serialize. UI tải lại cây thư mục hoặc trang đang mở. Nếu `origin = Mcp` thì hiện toast "Agent đã cập nhật …".
- `vault://lock`: `{ writable: bool }`, báo quyền ghi hiện tại. Nếu `wiki-mcp` direct giữ lock, app vào chế độ chỉ đọc, hiện banner và thử giành lại lock mỗi giây. UI cũng đọc `vault_is_writable()` lúc mở để không bỏ lỡ event phát trong startup.
- `mcp://status`: số phiên MCP đang kết nối và lần gọi gần nhất. Phiên được đếm theo `Mcp-Session-Id` từ handshake đến request `DELETE`; footer không dùng số request đang xử lý làm số client.

## File watcher <a id="file-watcher"></a>

- Dùng `notify` (FSEvents trên macOS), debounce khoảng 300 ms, bỏ qua `.wiki-desktop.lock`, file SQLite và file tạm của chính `Vault`.
- Gom các path thay đổi rồi gọi `Vault::notice_external(paths)`.
- Thay đổi do `Vault::apply` gây ra thì không phát lại lần nữa. `Vault` giữ một tập "path vừa ghi kèm hash", watcher so với tập đó để lọc.

## Cấu hình và dữ liệu

| Thứ | Vị trí |
|---|---|
| Danh sách vault gần đây, vault đang mở, port MCP | `~/Library/Application Support/<bundle-id>/settings.json` |
| `runtime.json` (port, token cho bridge) | cùng thư mục, quyền `0600`, xóa khi app thoát |
| Token MCP | macOS Keychain (`keyring` crate) |
| Vault | thư mục do người dùng chọn, mặc định `~/Documents/Wiki` |

## Bảo mật <a id="bao-mat"></a>

- MCP HTTP chỉ bind `127.0.0.1` và luôn yêu cầu token, kể cả với loopback. Lý do: trang web độc hại vẫn có thể gửi request tới localhost.
- Kiểm tra header `Origin`/`Host` để chặn DNS rebinding.
- Capabilities Tauri chỉ cấp `dialog:open`, `event` và các command ở trên. Không cấp `fs` hay `shell` cho WebView. WebView không đọc file trực tiếp mà luôn đi qua `Vault`, nhờ đó kiểm tra `DocPath` có hiệu lực ở mọi lối vào.
- CSP cấm script ngoài. Mermaid và chart renderer được bundle sẵn.

## Đóng gói

- `tauri build --target universal-apple-darwin`. `wiki-mcp` là `externalBin` (sidecar) hoặc build vào `Contents/MacOS/`.
- Ký bằng Developer ID, bật hardened runtime và notarize bằng `notarytool`. Binary `wiki-mcp` cũng phải được ký.
- Cập nhật app dùng `tauri-plugin-updater` (có thể để sau v1).

## Menu và phím tắt

`⌘N` trang mới, `⌘P` mở nhanh (search), `⌘⇧F` tìm toàn văn, `⌘G` graph, `⌘,` Settings, `⌘O` mở vault. Menu bar có thêm mục "Copy MCP config".
