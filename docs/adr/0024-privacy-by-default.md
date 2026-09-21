# ADR 0024 — Telemetry privacy-by-default

## Status
Accepted

V1 chỉ telemetry metadata vận hành qua Trace Session ID ngẫu nhiên; không persist hoặc log audio, transcript, prompt/response, tool arguments/results, Device/Client ID nguyên bản, auth/provider secrets hay body OTA/provider. Dialogue chỉ ở RAM Voice Session và bị drop khi disconnect/reconnect; sensitive debug capture ngoài V1.
