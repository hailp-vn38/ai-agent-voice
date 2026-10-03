# ADR 0055 — History Purge là destructive operation explicit có scope

## Status

Accepted

V1 không hard-delete Agent, Template, Provider, MCP Server hay Device; configuration chỉ soft-disable với optimistic concurrency. Chỉ `POST /api/admin/history/purge` xóa Persistent Transcript, bắt buộc exactly một scope Device, Voice Session hoặc `all`; scope `all` cần `confirm: "PURGE_ALL_HISTORY"`. Purge không tác động DialogueHistory hay Effective Session Profile đang chạy.
