# 05: Binary wiki-mcp stdio, chế độ direct

Status: ready-for-agent
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
