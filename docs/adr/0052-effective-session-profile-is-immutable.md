# ADR 0052 — Effective Session Profile immutable đến disconnect

## Status

Accepted; affected runtime clauses superseded by [ADR-0071](0071-versioned-provider-runtime-manager.md). Implementation/gates are tracked separately in `.scratch/provider-runtime-manager/`.

Admission materialize đúng một Effective Session Profile và SessionActor không giữ DB row hay live repository reference. Template Switch Catalog snapshot mọi candidate enabled đã validate/load; default invalid fail admission, còn non-default invalid bị exclude kèm diagnostics. Agent không có assignment dùng ServerDefaultProfile với catalog rỗng và không advertise switch tool. Admin mutation của Device, Agent, Template, Provider hoặc External MCP chỉ tác động Voice Session mới; switch chỉ lookup catalog và đổi profile theo command explicit của chính session tại next-turn boundary. Session Profile Revision bắt đầu 1 và chỉ tăng atomically khi switch thành công; không phải DB revision và không persist history. Revoke realtime, nếu cần, là feature `SessionRegistry → RevokeSession → controlled shutdown` riêng.
