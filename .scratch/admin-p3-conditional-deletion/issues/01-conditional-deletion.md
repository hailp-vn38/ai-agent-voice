# Conditional Admin deletion và MCP unlink

Type: task
Status: resolved

## Scope

Thay thế phần hard-delete của ADR-0055 theo ADR-0070. Thêm `DELETE` có `If-Match` cho Agent,
Device, Template, Provider và MCP Server; chặn `409 *_in_use` khi còn active
relationship/history; không thay đổi dữ liệu nếu bị chặn. Agent Template đã soft-unlink
(`enabled=false`) không còn là relationship active. Thêm DELETE Agent MCP binding, bump Agent
revision và audit trong cùng transaction.

## Acceptance

- Public Admin API tests bao phủ delete thành công, `If-Match` thiếu/stale, in-use và MCP unlink.
- Delete thành công trả 204, audit cùng transaction, không cascade active relationship/history.
- `cargo fmt --check`, `git diff --check`, focused Admin tests và workspace tests xanh.

## Answer

Đã triển khai tại commit `06e7e84`: DELETE Agent, Device, Template, Provider và MCP Server
dùng `If-Match`, chặn active relationship/history bằng `409 *_in_use`, và audit delete trong
cùng transaction. DELETE MCP binding xóa binding, bump Agent revision và audit. Focused Admin
test, full Admin suite, library suite và workspace suite đều xanh; hai review cuối không còn
finding.
