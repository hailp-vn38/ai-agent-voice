# 04: wiki-mcp ToolHost và chuyển vela-server sang dùng

Status: ready-for-agent
Blocked by: 02
Spec: ../spec.md (user stories 37–40, 45, 47–48, 50–51)

## Mục tiêu

Tách phần định nghĩa và dispatch tool knowledge MCP khỏi `vela-server` thành crate `wiki-mcp` chạy trên `Vault`, rồi cho `/mcp/knowledge` của server dùng crate này.

## Phạm vi

- `ToolHost::new(vault, policy)`, `list_tools(access)`, `call(ctx, name, args)`.
- Giữ nguyên tên tool, cách phân loại đọc/ghi (`is_read_only_wiki_tool`), các variant lỗi và giới hạn cắt kết quả. Thêm lỗi `ReadOnly`.
- `Policy`: `allow_write`, `max_concurrency`, `call_timeout`, `max_result_chars`.
- Tool ghi gọi `Vault::apply_from(Origin::Mcp, …)`.
- Token và settings vẫn nằm ở `vela-server`, không chuyển vào crate.
- `vela-server` tạo `Vault` từ kho tri thức hiện có và chuyển route `/mcp/knowledge` sang `ToolHost`. Các route `/api/admin/knowledge/*` **không** đổi trong ticket này.

## Tiêu chí xong

- [ ] Bộ test knowledge MCP hiện có chuyển sang `wiki-mcp` và pass.
- [ ] Test: `allow_write = false` thì client ReadWrite chỉ thấy tool đọc; timeout, cancel, giới hạn concurrency; `ReadOnly` khi kho bị process khác khóa.
- [ ] Toàn bộ test của `vela-server` pass. Hành vi `/mcp/knowledge` (tool, token, giới hạn) giữ nguyên.
- [ ] Wiki Auto-Flow vẫn được kích hoạt sau khi ghi qua MCP như trước.

## Cập nhật tiến độ

- Đã tách dispatch knowledge MCP thành crate `wiki-mcp` với `ToolHost`; `/mcp/knowledge` của `vela-server` dùng ToolHost, còn token/settings và Auto-Flow vẫn ở server.
- PASS: `wiki-mcp` (7 tests), `wiki-core` (13 tests), ToolHost subprocess kiểm tra `ReadOnly` khi writer lease bị process khác giữ, chính sách read-only của server, hồi quy query/write, schema validation và `check vela-server`.
- Chưa đạt tiêu chí toàn bộ test `vela-server`: full package dừng tại `tests/vieneu.rs::native_selection_is_typed_registered_and_credential_free`, do fixture `config.toml` local không có `vieneu_default`; test này cũng fail độc lập trong checkout gốc vì cùng thiếu entry.
- Ticket vẫn `ready-for-agent` cho tới khi test full package chạy qua với fixture phù hợp. Chưa chạy E2E MCP từ app vì Tauri shell chưa được triển khai.
