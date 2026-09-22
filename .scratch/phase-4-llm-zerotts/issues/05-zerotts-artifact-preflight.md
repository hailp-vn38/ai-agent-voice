# 05: ZeroTTS artifact provenance và startup preflight

**What to build:** Operator có một `zerotts_default` Model Artifact Manifest complete và auditable trước khi native runtime được port. Startup Model Preparation xác minh đủ artifact pack, named voice `maichi`, composite license acknowledgement và deterministic non-user preflight boundary.

**Blocked by:** 01: Mở rộng typed provider foundation Phase 4.

**Status:** resolved

- [x] Manifest pin revision, source/checksum, install-relative path và required roles cho config, tokenizer, voice, graph, codec shared data, metadata và codec license.
- [x] `MIT; bundled-codec=Apache-2.0` acknowledgement phải match exact logical model/revision/license; offline/missing/corrupt/unsafe artifact fails before bind.
- [x] Factory preflight contract rejects missing role, wrong logical model/voice, missing codec shared data hoặc invalid warmup PCM without creating a Voice Session.
- [x] Tests use deterministic acquirer/fixtures to prove artifact and license validation; no model weight, user text or provider secret is committed/logged.

## Comments

- Hoàn tất 2026-09-22: pin `zerotts_default` tại revision `c2bfbd67dc648cac455077333f7cf5c18a2e3bb4` với 13 artifact roles và SHA-256; Model Preparation inject `ResolvedModel` vào `TtsFactory`, factory fail-closed với role/identity/voice sai, và preflight PCM có contract 48 kHz mono finite/non-empty. Ticket 06 sở hữu tokenizer/config/metadata, graph I/O/voice-dimension compatibility và three-graph code parity; Ticket 07 sở hữu codec cùng native warmup thực.
- Xác minh: `cargo fmt --check`, 4 test artifact-preflight, `cargo test --workspace`, `git diff --check` pass. `cargo clippy --workspace --all-targets -- -D warnings` bị chặn bởi hai lint pre-existing ngoài scope: `items_after_test_module` trong `session/speech_output.rs` và `result_unit_err` trong `workers/llm.rs`.

- Audit 2026-09-22: Ticket 05 vẫn resolved. Hai Clippy findings trên nằm trong code Phase 4 của Ticket 04 và 03; Ticket 08 đã được cấp quyền sửa rõ ràng như một phần của Phase Completion Gate. Không ticket nào được dùng ignored real-model test thay cho gate này.
