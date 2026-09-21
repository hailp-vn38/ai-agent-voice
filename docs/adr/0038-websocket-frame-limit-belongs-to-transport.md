# ADR 0038 — WebSocket frame limit thuộc transport, tách khỏi buffer codec

V1 đặt `websocket.max_frame_bytes` ở transport, default 65.536 bytes và validate 4.000–1 MiB; giới hạn này áp dụng cho cả JSON control/MCP và binary ingress. `DownlinkOpusEncoder` luôn dùng scratch 4.000 bytes theo API libopus, rồi kiểm packet thực tế với transport cap; không truyền transport cap làm `max_data_bytes`, vì nó không được phép điều khiển codec bitrate hay giới hạn MCP.

Uplink codec có policy guard riêng `MAX_UPLINK_OPUS_PACKET_BYTES = 4.000`: packet vượt guard trả `Dropped(PacketTooLarge)` trước libopus, giữ session/capture/decoder. Đây là V1 implementation policy cho Canonical Audio Profile, không phải giới hạn hợp lệ chung của Opus. `DOWNLINK_ENCODE_BUFFER_BYTES = 4.000` là encoder scratch khác semantic, không được share constant với uplink guard.
