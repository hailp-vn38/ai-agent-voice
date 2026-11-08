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
