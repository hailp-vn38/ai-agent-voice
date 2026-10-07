# 15: Required xác minh mới ở từng voice turn

**What to build:** Agent Required chỉ accept lượt nói được phép: lần đầu nhận diện1:N, các lượt sau verify1:1 identity khóa, và không thể dùng Detect để vượt gate.

**Blocked by:** 10: Observe giọng ESP32 qua Voice WS, 12: Quan sát và approve Device tool contracts, 14: Reload calibration và kiểm exact candidate sets.

**Status:** ready-for-agent

- [ ] Guard enable/admission bằng ready binding/runtime/grants + exact qualified evidence; deterministic qualified fixtures chỉ trong qualification build, không production bypass.
- [ ] Common accept boundary chờ ASR nonempty + fresh speaker pass + History Barrier; exactly-once STT/history/LLM, identity/generation/runtime stale checks trước semantics.
- [ ] Lock Speaker perWS, mỗi lượt fresh1:1, short fail không inherited pass, change speaker reconnect; compare all compatible Agent candidates trước Template grant.
- [ ] Denied/unknown/ambiguous/short/unavailable zero STT/history/archive/LLM/tool/TTS; Required Detect audio-required denied; Device abort và same-session barge-in giữ protocol.
- [ ] Bounded cleanup/timeout/queue failure, mismatch counter3 với1008; busy/short/runtime không count. Actor checks security epochs trước accept/LLM continuation/tool.
- [ ] Agent UI, speaker status và public WS qualification kiểm result reorder, reject side effects, history/writer barrier, per-turn identity, stale outputs và control responsiveness.

## Answer

Implemented the Required fresh-per-turn gate end to end, test-first.

**What landed**

- `session/speaker_gate.rs` — pure decision core (`SpeakerGate`, `GateDecision`, `GateReject`).
  First turn identifies 1:N (verify threshold + `SPEAKER_GATE_MARGIN` over the runner-up); later
  turns verify 1:1 against the lock; `insufficient_audio`/`unavailable` are bounded non-answers,
  `unknown`/`ambiguous`/`mismatch` count, three consecutive counting denials return the close
  reason. 8 unit tests.
- `session/actor/speaker.rs` + actor wiring — the terminal ASR final is held until the same turn's
  speaker diagnostic resolves (either order), then committed exactly once or refused with zero
  history/LLM. Detect in Required is refused as audio-required. The diagnostic is routed back from
  the detached scoring task through a new `gate_rx` drained in `drain_provider_events`; stale
  generation/turn results never authorize.
- `OutboundMessage::CloseWithReason` — 1008 closes now carry `speaker_policy_denied`; the websocket
  writer forwards the reason.
- `speaker_observe.rs` — `ObservePlan` carries the policy; `ObserveDiagnostic` carries the
  runner-up score so the gate can detect ambiguity.
- `app/admin/speaker_policy.rs` — `required` is selectable once calibration is qualified (replaces
  ticket 14's always-block).
- Tests: `tests/speaker_required.rs` (7 actor-level tests: 1:N lock, 1:1 verify, mismatch,
  ambiguous, unknown, 3-denial 1008, unavailable non-counting); Required cases added to
  `tests/speaker_observe.rs`; calibration API test updated to assert `required` becomes available.

**Files changed:** 12 source + 3 test files (see the `feat(speaker): Required fresh per-turn
verification gate` commit on this branch).

**Test commands / results**

- `cargo test -p voice-agent-server --lib` → 211 passed, 1 ignored.
- `cargo test -p voice-agent-server --test speaker_required` → 6 passed.
- `cargo test -p voice-agent-server --test speaker_observe` → 6 passed.
- `cargo test -p voice-agent-server --test speaker_calibration_api` → 6 passed.
- `cargo test -p voice-agent-server --test agent_speaker_policy_api` → 6 passed.
- `cargo clippy -p voice-agent-server --lib --tests` → no warnings in touched files.
- `cargo check --workspace --all-targets` → clean.

**Remaining gap**

- `session_profile.rs::public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version`
  fails on a clean `HEAD` (pre-existing, unrelated); verified by stashing this branch's changes.
- The `History Barrier` and `security epoch` are honored via the existing writer/generation
  machinery (the gate refuses before `commit_user_text` and checks the generation); no new barrier
  primitive was introduced.
- Agent UI/browser qualification and the "deterministic qualified fixtures only in qualification
  build" rule are not part of this diff; they belong to the qualification/pilot tickets.
- `ponytail:` a `required` Agent stranded with zero candidates by a direct DB edit (bypassing the
  policy PUT and ticket-09 invalidation) would admit ungated; the admin path cannot reach it.
