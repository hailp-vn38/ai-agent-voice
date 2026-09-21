# ADR 0015 — Một WebSocket là một Voice Session tạm thời

## Status
Accepted

Server không đóng WebSocket chỉ vì `tts:stop`: một Voice Session có thể phục vụ nhiều turn. Session kết thúc khi disconnect, server idle cleanup, lỗi transport/protocol nghiêm trọng hoặc reconnect cùng Device ID; reconnect tạo session ID và Voice Session mới, đồng thời reset dialogue trong RAM. Firmware baseline tự timeout sau 120 giây không nhận dữ liệu, còn timeout server 300 giây chỉ nhằm cleanup tài nguyên.
