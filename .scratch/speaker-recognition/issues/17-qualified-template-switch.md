# 17: Switch Template bằng hot runtime và quyền đúng

**What to build:** Speaker đã khóa switch sang Template được grant và có exact-set evidence; cold/denied switch giữ nguyên Template hiện tại.

**Blocked by:** 16: Thu hồi quyền và vô hiệu hóa WS bị ảnh hưởng.

**Status:** done

- [x] Target dùng đúng admission-time DesiredProvider; preparation ngoài actor, không query latest configuration hoặc preload mọi candidate.
- [x] Kiểm locked-speaker grant, epoch, target-space Voiceprint và exact qualification trước arm và apply; không fallback, auto-enroll hoặc downgrade.
- [x] Hot Ready acquire được phép; target cold khi session giữ pipeline trả busy và không build, operator prepare trước session; failure giữ profile cũ.
- [x] Apply tại normal boundary; giữ Prepared Template Profile leases và release profile cũ sau cleanup. Speaker identity giữ nguyên, lượt tiếp theo verify mới trên target.
- [ ] Public switch tests kiểm cùng/khác spaces, concurrent PATCH, revoked target, history/writer boundary, hot/cold outcomes và không có side effect trên switch thất bại.

## Answer

**Mechanism.** A new `session::switch_authority` module carries the ticket-17 gate.
`SwitchSpeakerAuthority { template_id, embedding_space, candidate_set_digest, qualified,
candidates, plan }` is the frozen authority to keep the locked Speaker gated on one target
Template. `SpeakerSwitchGuard` bundles the active policy and embedding space with one authority
per candidate Template and the session's security token. The actor holds the guard as
`SessionActor::speaker_switch: Option<Arc<SpeakerSwitchGuard>>` (`None` = speaker-free, membership
is the whole rule) and calls `authorize_speaker_switch(template_key)`:

- `Unavailable` — no admission-time authority for the target (cold, speaker-free or unknown);
- `Unqualified` — the Agent's exact candidate set has no calibration evidence;
- `NotGranted` — the target does not grant the locked speaker a target-space Voiceprint;
- `IncompatibleSpace` — the target scores in a different embedding space than this runtime;
- a cancelled security token (ticket 16 epoch) also returns `Unavailable`.

**Wiring.** `app::websocket::resolve_speaker_observe` now returns `(observe, guard)`. While it
still holds the database handle it resolves `resolve_observe_plan` and
`admin::speaker_calibration::agent_qualified` for every warm candidate Template and freezes an
authority per candidate; the guard reuses the same `Arc<CancellationToken>` the observe pinned.
`builtin_actions::execute_template_switch` checks the guard at arm (before both the cold prepare
and the warm schedule), and `apply_template_switch` / `drain_managed_switch_boundary` recheck at
the turn boundary. On success `install_switch_speaker` swaps in the target's frozen `ObservePlan`
through `SpeakerObserve::retargeted`, keeping the same runtime, lease and identity so the next
turn verifies anew on the target. A refused switch never touches the profile or the runtimes.

**Tests.** `session::switch_authority::tests` covers granted / not-granted / unqualified /
missing / different-space / cancelled-epoch / unlocked. `actor::tools::tests` covers the apply
path (granted switch applies; not-granted, unqualified, missing, different-space and revoked all
keep revision 1 and leave `llm_runtime` untouched) and the arm path (a locked speaker never arms
a non-granting warm target or a cold target it cannot gate).

**Remaining gap.**

1. A target-template grant/voiceprint revocation that lands *after* admission but *before* arm is
   not caught: the session's security token pins the active Template, not the candidates, and the
   actor cannot re-read the database. The authority is a correct admission-time snapshot; a target
   PATCH that should invalidate it would need the session to pin every candidate Template's
   `SpeakerDeps` (one token per candidate) — deferred to keep admission from fanning out.
2. Only warm candidate Templates get an authority, so a `required`/`observe` session fails closed
   on a cold target instead of preparing it. The ticket's intended flow (operator prewarms, then
   the target is warm at admission) is covered; cold-speaker-target preparation is not.
3. Public HTTP+WS integration tests for concurrent PATCH and the history/writer boundary are not
   written; the authorization is covered by lib unit tests instead.

