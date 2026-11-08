# 05: Binary wiki-mcp stdio, chế độ direct

Status: ready-for-human
Blocked by: 03, 04
Spec: ../spec.md (user stories 34, 43, 44)

## Mục tiêu

Binary `wiki-mcp` chạy MCP qua stdio. Binary tự mở kho tri thức, nên Claude Code/Desktop dùng được wiki khi app không chạy.

## Phạm vi

- Tham số: đường dẫn kho, hoặc đọc kho mặc định từ settings của app nếu có; cờ cho phép ghi.
- Mở `Vault`: có lock thì ghi được, không có thì chỉ đọc.
- Phục vụ `ToolHost` qua `rmcp` stdio transport.
- Log chỉ ghi ra stderr, không bao giờ ghi ra stdout.

## Tiêu chí xong

- [ ] Test spawn binary rồi gửi `initialize`, `tools/list`, `wiki_status`: nhận phản hồi đúng.
- [ ] Đang có process khác giữ lock thì `wiki_write_file` trả `ReadOnly`, còn tool đọc vẫn chạy.
- [ ] Smoke test thủ công: thêm vào cấu hình Claude Code, gọi `wiki_query` thành công.

## Cập nhật tiến độ

- Đã thêm binary `wiki-mcp` với `--vault <directory>` (hoặc `WIKI_VAULT_PATH`) và cờ `--allow-write`; mặc định chỉ có tool đọc. Vault tự vào read-only khi process khác đang giữ writer lease.
- PASS: test spawn binary qua rmcp stdio chạy `initialize`, `tools/list`, `wiki_status`; khi writer lease bị giữ, `wiki_status` vẫn chạy và `wiki_write_file` trả lỗi `ReadOnly`. Gói `wiki-mcp` pass 2 integration test với `--locked`; `cargo check --locked -p wiki-mcp` pass.
- Log khởi tạo và lỗi CLI đi ra stderr; stdout dành riêng cho MCP stdio transport.
- Còn smoke test thủ công kết nối Claude Code (`wiki_query`), nên trạng thái là `ready-for-human`.
