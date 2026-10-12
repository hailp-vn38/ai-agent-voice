# ADR 0032 — Fail closed handshake, fail soft application message

## Status
Accepted

Ở `AwaitHello`, chỉ ClientHello Canonical Audio Profile hợp lệ được chấp nhận; binary, malformed/non-hello JSON, missing field, sai version/transport/profile đóng 1002 trước Voice Session. Sau handshake, một malformed/unknown/invalid/wrong-state application message chỉ metric và ignore; frame vượt cap đóng 1009 trước parse. WebSocket framing/UTF-8 fault thuộc transport library, và V1 không invent custom wire error.

## Opt-in request outcomes (proposed)

[ADR-0091](0091-reminder-authorization-and-turn-context.md) đề xuất typed outcomes cho các request Direct TTS/Reminder mới, đã parse hợp lệ, có session identity và feature opt-in. Đây là mở rộng có chủ đích của giới hạn “không custom wire error”; malformed/unknown application messages và client cũ vẫn giữ metric/ignore, handshake và frame cap vẫn fail closed. Chưa áp dụng vào protocol hiện tại.
