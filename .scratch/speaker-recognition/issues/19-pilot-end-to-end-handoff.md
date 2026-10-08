# 19: Qualification public và bàn giao pilot V1

**What to build:** Operator có một đường kiểm chứng đầy đủ enrollment web→Observe ESP32→Required guarded, cùng hướng dẫn vận hành vừa đủ và reports rõ software compatibility khác real qualification.

**Blocked by:** 13: Giải quyết conflict bằng discovery theo đợt, 17: Switch Template bằng hot runtime và quyền đúng, 18: Báo cáo calibration và tải pilot có thể tái lập.

**Status:** resolved

- [ ] Scenario qua production process public Admin HTTP/Voice WS bằng Reference Integration Client kiểm các slice hoàn chỉnh, controlled restart và independent wire contracts; không test-only AppState hoặc runtime bypass.
- [ ] Acceptance mốc đầu có enrollment web, Observe ESP32, applicable allowlist/recovery và pilot envelope. Required chỉ bật với đúng exact qualification/tool rights, không từ CI deterministic đơn thuần.
- [ ] Mandatory Qualification không cần tải model hoặc credentials. Real CAM++/ESP32 evidence chạy khi operator cung cấp corpus/máy; nếu chưa có, report NOT_RUN và prerequisite thiếu, giữ preliminary/Required unavailable.
- [ ] Rust/frontend/protocol/admin/runtime checks đạt. API collection, Reference Client và runbook đã được cập nhật cùng các slice; scenario cuối kiểm compatibility giữa chúng.
- [ ] Runbook có operator review Persona/prompt/context/tool results và re-review khi thay nguồn, không workflow eligibility mới; history cách ly và giới hạn replay/privacy rõ ràng.
- [ ] Không thêm hidden bypass, model manager, scheduler, RBAC hoặc eligibility product. Handoff ghi baseline, tests, Mandatory result và Optional Runtime Evidence thật; không deploy Required hoặc claim accuracy khi thiếu evidence.

## Answer

Implemented on branch `speaker/19` (baseline `99cce86`).

**Compile-time Qualification Provider seam (ADR 0068).** New feature
`qualification-providers` on `voice-agent-server` registers a deterministic,
model-free `qualification_speaker` adapter (`providers/speaker/qualification.rs`,
`descriptor.rs`, `factory_registry.rs`, `registry.rs`, `database/provider_config.rs`).
It is compile-time only: the default/release build still exposes only
`campplus_sherpa`. Under the feature the binary skips model download and
deployment-provider materialization (`app/mod.rs`), so it boots with no assets
and no credentials. `qualification_speaker` is treated as non-native in the
runtime factory, so no ONNX fingerprint is required.

**Integration Harness + Reference Integration Client.**
`crates/voice-agent-server/tests/qualification_harness.rs` spawns the
**production binary** (`CARGO_BIN_EXE_voice-agent-server`), waits for the
nonce-bound `VOICE_AGENT_BOUND_ADDRESS_FILE` handshake artifact, drives the
public Admin HTTP enrollment boundary (create provider → create speaker → open
draft → 3 samples → holdout validate → finalize) with its own wire types, then
performs a **controlled SIGTERM restart** and asserts the published voiceprint
persisted. It writes a privacy-safe Qualification Report.

**Docs.** `docs/speaker-pilot-runbook.md` (Required guard, operator review loop,
history/replay/privacy limits, how to run Mandatory + real evidence, rollback)
and `docs/speaker-pilot-handoff.md` (baseline, tests, Mandatory result, honest
NOT_RUN evidence, gaps). `docs/api/00-all-apis.postman_collection.json` gains the
sample-upload, holdout-validate and finalize requests.

Tests:

```
cargo check -p voice-agent-server --tests
cargo check -p voice-agent-server --features qualification-providers
cargo test  -p voice-agent-server --features qualification-providers \
    --test qualification_harness -- --ignored    # 1 passed
cargo test  -p voice-agent-server                 # see handoff for result
```

Acceptance mapping:

- Bullet 1 (public-process scenario, controlled restart, independent wire
  contracts): covered for the enrollment slice by `qualification_harness.rs`.
- Bullet 3 (no model/credentials; NOT_RUN when evidence absent): covered — the
  harness runs model-free, and the report/handoff record Optional Runtime
  Evidence as `NOT_RUN` with missing prerequisites.
- Bullets 5–6 (runbook, no hidden products, honest handoff): covered by the two
  new docs and the unchanged authorization surface.
- Bullet 2 (Observe ESP32 + Required through the production process) and the
  process-level Voice WS slice are **NOT_RUN**: the Observe slice is covered only
  at actor/wire level today. Driving it through the production binary needs the
  remaining ADR 0068 qualification providers (`qualification_vad/asr/llm/tts`),
  which are not implemented; the harness declares real adapters as defaults and
  skips building them under the feature.

Remaining gap: real CAM++/ESP32 Optional Runtime Evidence and frontend checks are
`NOT_RUN`; the handoff records the missing prerequisites rather than claiming
accuracy. The full `cargo test -p voice-agent-server` suite passes except one
pre-existing failure (`session_profile::public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version`,
`200` vs `202`), which reproduces with this ticket's changes stashed.
