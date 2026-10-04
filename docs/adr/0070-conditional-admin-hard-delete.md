# ADR 0070 — Admin hard-delete có điều kiện và explicit unlink

## Status

Accepted — thay thế phần "V1 không hard-delete configuration" của ADR-0055.

## Decision

Admin API hỗ trợ hard-delete các resource desired-configuration qua `DELETE` và `If-Match`.
Một delete chỉ thành công khi resource không còn active relationship hoặc history reference
nào. Thành công trả `204 No Content`, ghi audit action `delete` trong cùng transaction, và
không materialize runtime mới hoặc tác động Voice Session đã admit.

Mọi active relationship phải được unlink tường minh trước. Nếu còn reference, API trả `409`
với code resource-specific `*_in_use`; không có cascade-unlink active relationship, revision
bump ngầm của owner, hay history purge ngầm. History vẫn chỉ bị xóa bởi
`POST /api/admin/history/purge`.

- Provider bị chặn bởi Template Provider Binding.
- MCP Server bị chặn bởi Agent MCP Binding.
- Template bị chặn bởi active Agent Template Assignment, Template Provider Binding, Device
  Template Override hoặc History Message.
- Agent bị chặn bởi Device, Agent MCP Binding hoặc History Message. Agent Template Assignment
  không chặn xóa: FK cascade chỉ xóa row liên kết của Agent, còn Template global vẫn giữ nguyên và
  tiếp tục dùng được bởi Agent khác.
- Device bị chặn bởi History Message.

`DELETE /api/admin/agents/{key}/mcp-bindings/{server_key}` là unlink tường minh; mutation
increment revision của Agent và audit action `unlink_mcp_binding`.

Agent Template unlink kế thừa P0 semantics: row assignment được giữ với `enabled=false`, nên
không còn là active relationship. Xóa Agent cũng được phép khi còn assignment active: foreign key
chỉ dọn các row assignment của Agent, không xóa Template hay history. Xóa Template vẫn cần unlink
mọi assignment active trước; sau unlink, foreign key có thể dọn row inert đó.

## Consequences

Quản trị viên có luồng xóa có thể dự đoán được và không vô tình mất transcript. Đổi lại, UI
phải hiển thị relationship usage và hướng dẫn unlink/purge trước khi retry delete. Runtime
Catalog và Voice Session đã admit vẫn immutable; delete chỉ đổi desired state, vì vậy caller
phải restart trước khi tin rằng runtime process đã đổi.
