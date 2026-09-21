# ADR 0004 — WebSocket binary protocol v1 trước

## Status
Accepted for V1

## Decision
V1 chỉ hỗ trợ raw Opus binary frame theo voice protocol version 1.

## Rationale
Firmware mặc định dùng version 1; v2 chủ yếu cần metadata/timestamp cho AEC và tăng độ phức tạp.

## Future
Thêm parser/encoder v2/v3 sau bằng enum protocol version mà không đổi pipeline AI.
