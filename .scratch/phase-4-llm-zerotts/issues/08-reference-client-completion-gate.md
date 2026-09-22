# 08: Reference Client Phase 4 completion gate

**What to build:** Phase 4 is qualified from an independent Voice Protocol Client: fake streaming LLM plus real ZeroTTS create a real assistant-audio turn compatible with canonical downlink Opus. Live OpenAI is documented and run only as optional deployment smoke.

**Blocked by:** 03: OpenAI adapter và LLM Operation runtime; 07: ZeroTTS worker canonical delivery.

**Status:** resolved

- [x] With the verified installed ZeroTTS pack and ONNX Runtime provisioned, the mandatory public-boundary gate proves valid ASR final, first real audio before fake LLM Finished, `tts:start` before audio, Drained then one stop, correct Active Turn release and Delivered Assistant Response commit. The Reference Client decodes the actual canonical 24 kHz Opus packets.
- [x] The same real-model gate proves cancel and Unexpected Tool Call invalidate the generation before stop, block queued/stale audio after stop, avoid MCP/retry, and do not commit an undelivered response. Fake LLM is allowed only to make stream timing/tool outcomes deterministic; fake TTS/Opus/pacer is not allowed in this gate.
- [x] Close the pre-audited Phase 4 repository hygiene debt: relocate the SpeechOutput test module so Clippy has no `items_after_test_module`, and replace LlmRuntime's unit error with a typed, non-secret error. Then `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `git diff --check` must pass. Any unavailable real-model gate is reported as unavailable, never as a pass.
- [x] Report the mandatory real-model Reference Client command/result separately from deterministic fake-provider tests and the opt-in live OpenAI smoke. Live smoke redacts credentials/content and reports network/provider failure as deployment information rather than Phase 4 CI proof.

## Comments

- Audit 2026-09-22: Ticket 08 explicitly owns the two known Phase 4 Clippy repairs, so an implementing agent does not need new permission to make the final quality gate green. The current checkout has no installed ZeroTTS pack, therefore this ticket cannot be marked resolved until its provisioned real-model command has run successfully.

- Hoan tat 2026-09-22: `scripts/test-phase4-reference-gate.sh` validates installed non-user ZeroTTS pack va ONNX Runtime without download, emits real canonical Opus, va `voice-reference-client` decode packet 24 kHz mono. Public router gate inject fake ASR/streaming LLM only de khoa timing, dung ZeroTTS that; gate passed normal delivery (first audio truoc LLM Finished, `tts:start` truoc audio, Drained mot stop), cancellation-after-Started, va Unexpected Tool Call (invalidate truoc stop, khong stale audio/commit/MCP/retry). `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check` passed. Live OpenAI smoke van opt-in deployment smoke, khong phai Phase 4 CI proof.
