# ADR 0044 — Model Preparation đã pin tại startup

## Status
Accepted

Typed provider configuration chỉ chọn Logical Model Identity; Model Artifact Manifest authoritative source, revision, remote artifact, install-relative path, declared transform và provider-facing checksum. Trước bind, Model Preparation resolve identity, reuse hoặc acquire artifact đã pin, verify, transform, verify output và atomic install dưới configured model root, rồi inject Resolved Model theo artifact role vào Provider Factory để build/warmup. Offline Model Preparation cấm network và fail trước bind khi không thể chứng minh artifact hợp lệ.

## Consequences

Provider không nhận direct filesystem path từ config, không tự download và không đoán tên upstream. Manifest path tuyệt đối, traversal hoặc escape model root bị reject.
