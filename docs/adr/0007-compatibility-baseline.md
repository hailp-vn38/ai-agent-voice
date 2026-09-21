# ADR 0007 — Pin firmware compatibility baseline

## Status
Accepted

V1 dùng Compatibility Profile `xiaozhi-fw-v2.5.0-ws-v1`: firmware `78/xiaozhi-esp32` release v2.5.0 tại commit `ac6deed3d8e75348475364bf40ad953c6cd48054`, raw Opus WebSocket v1 và MCP `type:"mcp"` với JSON-RPC 2.0. Source pinned commit là nguồn chân lý cao nhất; `main` chỉ theo dõi upstream và không được dùng sinh fixture. HIL Reference Profile là `bread-compact-wifi`.
