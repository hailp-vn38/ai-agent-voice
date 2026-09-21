# ADR 0011 — Device MCP default-deny trước khi LLM thấy tool

## Status
Accepted

Discovery không phải permission: server lọc Discovered Tool bằng allowlist trước khi tạo LLM-visible Tool. Read và low-risk write chỉ được gọi khi allowlisted; sensitive write mặc định deny; dangerous tool như reboot, firmware upgrade, reset hay command execution bị deny trong V1. V1 không có interactive confirmation, nên tool cần confirmation không được expose cho LLM.
