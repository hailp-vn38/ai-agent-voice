# 02: Worker runtime VAD/ASR có acknowledgement

**What to build:** Server có inference runtime bounded để các Voice Session có thể dùng local VAD/ASR mà không block actor hoặc Tokio. Mỗi stream giữ worker riêng, cancellation chỉ là hoàn tất khi worker acknowledgement, và các failure/timeout có lifecycle có thể kiểm chứng thay vì dựa vào synchronous provider call.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT.

**Status:** resolved

- [x] Provider, worker và SessionActor được tách theo ADR-0042: VAD/ASR mutable runtime chỉ sống trong worker; worker command/event mang session, generation và lease/stream identity; actor không trực tiếp gọi mutable provider stream.
- [x] ASR Open/Push/Finish/Cancel giữ stream pinned; Cancel acknowledgement mới release `AsrStreamLease`. VAD Open/Push/Reset/Close có ResetDone/Closed acknowledgement; worker cleanup timeout quarantine đúng worker, không tái sử dụng mù.
- [x] Runtime có bounded worker/command capacity, ASR-final timeout và cleanup grace timeout; fake provider tests chứng minh stale logical event bị drop nhưng cleanup acknowledgement của stale generation vẫn release/quarantine slot đúng cách.

## Comments

- Implemented worker-owned ASR/VAD runtime with identity-tagged events, acknowledgement-driven cleanup and quarantining; production WebSocket sessions share the application ASR runtime. Verified with `cargo fmt --check`, `cargo check --workspace`, and `cargo test --workspace` (the real-model reference-client smoke remains ignored because local model artifacts are absent).
