# 11: MCP HTTP trong app và màn Settings MCP

Status: ready-for-agent
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
