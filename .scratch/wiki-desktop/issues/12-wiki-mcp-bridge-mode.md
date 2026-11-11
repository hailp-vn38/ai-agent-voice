# 12: Binary wiki-mcp, chế độ bridge

Status: ready-for-human
Blocked by: 05, 11
Spec: ../spec.md (user stories 34, 43–44)

## Mục tiêu

Khi app đang chạy, binary stdio chuyển tiếp request sang app, để app luôn là writer duy nhất và UI nhận được sự kiện.

## Phạm vi

- Khởi động: đọc file runtime của app. Nếu file có và HTTP phản hồi thì chạy bridge (`rmcp` client chuyển tiếp JSON-RPC qua stdio ↔ HTTP). Nếu không thì chạy direct như ticket 05.
- File runtime cũ (app đã crash) không được làm treo binary: có timeout kết nối rồi rơi về direct.
- App đang mở mà `wiki-mcp` direct giữ lock: app vào chế độ chỉ đọc, hiện banner (`vault://lock`) và định kỳ thử lấy lại quyền ghi.

## Tiêu chí xong

- [ ] Test: dựng một HTTP host giả với `ToolHost`, ghi file runtime, spawn binary. `tools/list` và `wiki_status` đi qua bridge.
- [ ] Test: file runtime trỏ tới port chết → binary rơi về direct trong thời gian giới hạn.
- [ ] Test tích hợp: app (hoặc host giả giữ `Vault`) và `wiki-mcp` direct chạy đồng thời rồi cùng ghi → không hỏng index, đúng một bên nhận `ReadOnly`.

## Cập nhật tiến độ

- Đã thêm bridge stdio ↔ HTTP, timeout và direct fallback; app-generated stdio config truyền `--vault` để chạy direct khi app đóng. App phát lock state, hiện banner read-only và thử lấy lại writer lock mỗi giây.
- PASS: `wiki-mcp` crate tests (stdio/direct/bridge/dead-port/lock), Tauri MCP/lock integration tests và frontend checks.
- NOT_RUN: smoke Claude/app thật và xác nhận banner/reacquire trên hai process thật. Ticket 05 Claude smoke vẫn NOT_RUN.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

- [x] Bridge bỏ qua `--vault`: client cấu hình cho vault A có thể ghi vào vault B đang mở trong app. Đã sửa: runtime file ghi `vaultPath` đã chuẩn hoá; `wiki-mcp` chỉ bridge khi `--vault` trùng vault của app, nếu khác thì chạy direct. Không truyền `--vault` thì vẫn bridge. Test: `bridge_only_serves_the_vault_named_by_vault_flag`.
- [x] Bridge luôn dùng token ghi, bỏ qua `--allow-write`. Đã sửa: chỉ gửi token ghi khi có `--allow-write`, còn lại gửi token đọc. Config stdio app sinh ra tự thêm `--allow-write` khi app bật ghi. Test: `bridge_sends_write_token_only_with_allow_write`. Lưu ý: config Claude Desktop đã cài trước đây (chỉ `--vault`) sẽ chỉ đọc qua bridge cho tới khi cài lại.
- [x] Chưa phải relay JSON-RPC: mỗi `list_tools`/`call_tool` mở một session HTTP mới rồi đóng (`wiki-mcp.rs`). Cancel/progress không được chuyển tiếp, số client ở footer nhảy theo từng call. Giữ một session HTTP suốt vòng đời stdio.
- [x] Test tiêu chí 3 chưa đúng yêu cầu: spec cần "chạy đồng thời rồi cùng ghi → không hỏng index", nhưng `stdio.rs` ghi tuần tự và chỉ kiểm tra `index.md` tồn tại. Cần ghi đồng thời và kiểm tra tính toàn vẹn của index.
- [x] Bundle id `com.vela.wikidesktop` hardcode trong `wiki-mcp.rs` (đường dẫn runtime file); phải khớp tay với `tauri.conf.json`. Xem thêm ticket 13.
- Ngoài phạm vi (giữ hoặc ghi vào spec): flag `--runtime-file`, env `WIKI_DESKTOP_RUNTIME_FILE`, kiểm tra quyền runtime file.

### Sửa sau review (2026-11-08)

- Bridge giữ một session HTTP cho cả vòng đời stdio (dùng lại session probe, tự kết nối lại nếu app khởi động lại server, đóng session khi stdio kết thúc). Cancel từ client stdio gửi `notifications/cancelled` tới app. Progress chưa được chuyển tiếp. Test: `bridge_keeps_one_app_session_for_the_whole_stdio_session`.
- Test `concurrent_writers_keep_the_index_whole_and_only_the_lockless_one_is_read_only`: `Vault` giữ lock và `wiki-mcp` direct ghi đồng thời; mọi lần ghi của `wiki-mcp` nhận read-only, mở lại vault thì index tìm thấy đủ trang và `index.md` liệt kê đủ.
- Bundle id đọc từ `Info.plist` của `Wiki.app` chứa binary (xem ticket 13).
- Sửa thêm: lỗi clippy có sẵn ở `dispatch.rs` (`if` lồng nhau).
