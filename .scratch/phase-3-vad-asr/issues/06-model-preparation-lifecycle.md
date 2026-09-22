# 06: Model Preparation lifecycle

**What to build:** Thêm startup-only Model Preparation nhận Logical Model Identity, resolve manifest authoritative và trả Resolved Model verified cho Provider Factory trước warmup/bind.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT.

**Status:** claimed

- [ ] Manifest pin source, revision, license, remote artifact, install-relative path, declared transform và provider-facing checksum cho mỗi artifact; reject absolute/traversal/root-escape path.
- [ ] Model Preparation reuse valid artifact hoặc download `.part`, verify source/installed checksum theo manifest, perform transform, rồi atomic install dưới `[deployment.models].root`.
- [ ] `offline = true` cấm network tuyệt đối; missing, corrupt hoặc transform output invalid fail trước bind.
- [ ] `ResolvedModel` expose artifact role required, không expose layout assumption; ModelStore không biết Zipformer và provider không biết HTTP/download mechanics.
- [ ] Tests cover valid reuse, corrupt replacement, interrupted `.part`, transform, offline failure, path rejection và no bind before preparation/warmup success.
