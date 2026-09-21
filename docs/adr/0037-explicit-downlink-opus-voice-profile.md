# ADR 0037 — Downlink Opus dùng profile thoại explicit trong V1

`DownlinkOpusEncoder` cố định 24 kHz mono 60 ms, application VoIP, 32 kbps, VBR và constrained VBR bật, DTX/FEC tắt, packet-loss percent 0, complexity 10. Các controls này không vào `config.toml` ở Phase 2: raw WebSocket V1 chạy trên TCP và chưa có loss-recovery policy; profile explicit tránh phụ thuộc libopus default và sẽ chỉ được xem lại sau TTS/HIL của Phase 4.
