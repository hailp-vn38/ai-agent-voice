# 11: MCP HTTP trong app và màn Settings MCP

Status: ready-for-human
Blocked by: 04, 09
Spec: ../spec.md (user stories 35–42, 46)

## Mục tiêu

Khi app đang mở, agent kết nối được vào kho qua streamable HTTP. UI thấy agent ghi gì ngay lập tức.

## Phạm vi

- `rmcp` streamable HTTP server phục vụ `ToolHost`, chỉ bind `127.0.0.1`, port cấu hình được.
- Luôn yêu cầu bearer token. Token đọc và token ghi tách riêng, sinh ngẫu nhiên, lưu trong Keychain. Có `mcp_rotate_token`.
- Kiểm tra `Origin`/`Host` để chặn DNS rebinding.
- Ghi file runtime (port, token, quyền 0600) vào Application Support khi khởi động, xóa khi thoát.
- Event `mcp://status` (số client, lần gọi gần nhất) hiện ở footer. Toast "Agent đã cập nhật <trang>" khi có `Changed` với `origin = Mcp`.
- Settings → MCP:
  - bật/tắt quyền ghi;
  - copy URL kèm token;
  - hiện đoạn cấu hình stdio;
  - nút "Cài vào Claude Desktop": hiện diff và hỏi xác nhận trước khi ghi `claude_desktop_config.json`.

## Tiêu chí xong

- [ ] Test tích hợp: `rmcp` client gọi server trên port ngẫu nhiên. Thiếu token thì bị 401, `Origin` lạ thì bị từ chối, token đọc không gọi được tool ghi.
- [ ] Smoke test thủ công: agent gọi `wiki_write_file` thì toast hiện trong vòng 1 giây và trang mở sẵn tự cập nhật.
- [ ] File runtime có quyền 0600 và bị xóa khi app thoát bình thường.

## Cập nhật tiến độ

- Đã thêm HTTP MCP loopback, bearer token riêng, Host/Origin validation, Keychain, runtime file 0600, chọn port, token rotation, Claude config preview/confirmation, toast và trạng thái session.
- PASS: RMCP integration test (401/403/read-only/write-disabled/session lifecycle/runtime permissions); Tauri `cargo check`/test/format và Vite build.
- NOT_RUN: app smoke, agent write toast trên UI thật, hộp thoại cài Claude và runtime cleanup qua đóng app thật.
- Ticket 04 vẫn `ready-for-agent`: full test `vela-server` còn lỗi fixture local `vieneu_default`; giữ blocker này cho tới khi lượt kiểm tra đó pass.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

Spec:
- [x] "hiện diff": UI đang hiện hai khối before/after đầy đủ (`App.vue:238`), chưa phải diff.
- [x] Nút cài tính lại config (`mcp_commands.rs:213`) thay vì ghi đúng nội dung đã preview. Cần ghi đúng thứ người dùng đã xác nhận (hoặc từ chối nếu file đã đổi).
- [x] Port cấu hình bị chiếm/sai làm `vault_open` fail (`main.rs:222`); lúc khởi động còn xoá `current_vault` (`main.rs:442-445`). Lỗi MCP không được chặn việc mở vault.

Standards:
- [x] Shell chưa mỏng (`desktop-shell.md`): `mcp_set_port` (~45 dòng, có rollback), `mcp_rotate_token`, `mcp_claude_config_install` + helper `claude_config` (merge JSON) nên chuyển logic ra khỏi command.
- [x] `mcp.md` ghi config client `{ "command": ".../wiki-mcp" }`, code sinh thêm `args` (`--vault`, và từ nay `--allow-write`). Cập nhật `mcp.md`.

Smell (nhận định, không bắt buộc):
- Map lỗi "lock poisoned" lặp ~15 lần trong `main.rs`/`mcp_commands.rs`; chuỗi "Open a vault before enabling MCP" lặp 5 lần; nhiều `map_err(|e| CommandError::Io { … })` nên có `From`.
- Danh sách Host/Origin khai báo hai nơi (`mcp_server.rs:299-301` và middleware `authenticate`). `write_config` dựng lại URL thay vì dùng `base`.
- `mcp_rotate_token(access: String)` so khớp `"read"`/`"write"` dù đã có `wiki_mcp::Access`; `mcp_server.rs` trả `Result<_, String>` khắp nơi.
- Lỗi port/access trả `CommandError::InvalidPath` dù không liên quan đường dẫn.
- `mcp_server::start(app, vault, port, allow_write, runtime_file, vault_path)`: nhóm tham số chủ yếu lấy từ `Settings`.
- Pattern "lấy cái cũ rồi cancel" trên `watch::Sender` lặp ở các hàm `restart_*`; `vault://lock` emit ở 3 chỗ (`main.rs:178,200,248`).

### Sửa sau review (2026-11-08)

- Module mới `claude_config.rs`: merge giữ server khác, diff theo dòng (LCS), lưu preview trong state; `mcp_claude_config_install` ghi đúng bản đã preview và trả `Conflict` nếu file đổi sau preview. UI hiện diff +/−. 3 unit test.
- Mở vault không còn phụ thuộc MCP: port đã lưu bị chiếm thì dùng port trống khác; nếu vẫn lỗi thì vault vẫn mở, `mcp_info` báo lý do. `mcp_set_port` bind port mới trước khi dừng server cũ, nên port bận không làm mất server đang chạy.
- Command MCP mỏng lại (`running()`/`info()` helper); `McpLaunch` gom tham số khởi động; Host/Origin allow-list dùng chung; `mcp_rotate_token` nhận `Access` (`read_only`/`read_write`); lỗi input dùng `CommandError::InvalidInput`; thêm `From<io::Error>`/`From<serde_json::Error>`; `TaskCancel` (drop là cancel) thay pattern "lấy cũ rồi cancel"; `vault://lock` emit qua một helper. `McpInfo` thay `readUrl`/`writeUrl` bằng `url`.
- UI đọc message từ `CommandError` thay vì hiện `[object Object]`.
- `mcp.md` cập nhật config client có `args`.
- NOT_RUN: hộp thoại cài Claude và đổi port trên app thật.
