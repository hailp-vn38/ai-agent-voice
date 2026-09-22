# 06: Model Preparation lifecycle

**What to build:** Thêm startup-only Model Preparation nhận Logical Model Identity, resolve manifest authoritative và trả Resolved Model verified cho Provider Factory trước warmup/bind.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT.

**Status:** resolved

- [x] Manifest pin source, revision, license, remote artifact, install-relative path, declared transform và provider-facing checksum cho mỗi artifact; reject absolute/traversal/root-escape path.
- [x] Model Preparation reuse valid artifact hoặc download `.part`, verify source/installed checksum theo manifest, perform transform, rồi atomic install dưới `[deployment.models].root`.
- [x] `offline = true` cấm network tuyệt đối; missing, corrupt hoặc transform output invalid fail trước bind.
- [x] `ResolvedModel` expose artifact role required, không expose layout assumption; ModelStore không biết Zipformer và provider không biết HTTP/download mechanics.
- [x] Tests cover valid reuse, corrupt replacement, interrupted `.part`, transform, offline failure, path rejection và no bind before preparation/warmup success.

## Comments

- Implemented startup-only `ModelPreparation` with role-addressed `ResolvedModel`, SHA-256 source/output verification, atomic install, offline fail-fast, and symlink-safe model-root containment. The manifest now pins verified upstream URLs; Zipformer normalizes the pinned SentencePiece artifact into the provider `tokens.txt` contract.
- Verification: `cargo test --workspace`, `cargo clippy -p voice-agent-server --tests -- -D warnings`, `cargo fmt --check`, and `git diff --check` passed. The real-model Reference Client smoke remains ignored because it needs the large locally prepared artifacts; it is not a Phase Completion Gate substitute.
