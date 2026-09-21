# ADR 0032 — Fail closed handshake, fail soft application message

## Status
Accepted

Ở `AwaitHello`, chỉ ClientHello Canonical Audio Profile hợp lệ được chấp nhận; binary, malformed/non-hello JSON, missing field, sai version/transport/profile đóng 1002 trước Voice Session. Sau handshake, một malformed/unknown/invalid/wrong-state application message chỉ metric và ignore; frame vượt cap đóng 1009 trước parse. WebSocket framing/UTF-8 fault thuộc transport library, và V1 không invent custom wire error.
