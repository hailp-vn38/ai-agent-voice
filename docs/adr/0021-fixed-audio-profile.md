# ADR 0021 — Canonical Audio Profile cố định cho V1

## Status
Accepted

V1 validate thay vì negotiate audio: client phải dùng raw Opus WebSocket v1, 16 kHz mono 60 ms; server luôn gửi Opus 24 kHz mono 60 ms. Client hello/header mismatch đóng WS với 1002 trước Ready; server không đoán hoặc resample uplink. TTS provider vẫn có thể normalize/resample nội bộ trước canonical downlink.
