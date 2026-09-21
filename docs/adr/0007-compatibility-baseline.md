# ADR 0007 — Pin reference implementation provenance

## Status
Accepted

V1 dùng Compatibility Profile `voice-ws-v1-baseline`: raw Opus WebSocket v1 và MCP `type:"mcp"` với JSON-RPC 2.0. Reference source là firmware `78/xiaozhi-esp32` release v2.5.0 tại commit `ac6deed3d8e75348475364bf40ad953c6cd48054`; nó cung cấp provenance cho fixture và không định nghĩa loại client được hỗ trợ. `main` chỉ theo dõi upstream và không được dùng sinh fixture. HIL Reference Profile là `bread-compact-wifi`.
