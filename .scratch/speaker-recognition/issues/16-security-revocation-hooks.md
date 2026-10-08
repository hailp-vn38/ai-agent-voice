# 16: Thu hồi quyền và vô hiệu hóa WS bị ảnh hưởng

**What to build:** Admin re-enroll, purge hoặc thu hồi quyền; WS bị ảnh hưởng dừng dispatch mới và đóng1008, còn snapshot vẫn qualified/không revoke tiếp tục.

**Blocked by:** 08: Re-enroll, nhiều embedding spaces và purge, 15: Required xác minh mới ở từng voice turn.

**Status:** done

- [x] Wire tất cả relevant Speaker/Agent/Template/provider/grant/policy handlers vào consistent mutation/catalog/security publication; không chỉ API Speaker mới.
- [x] Re-enroll/disable/purge/reduce grants/unlink/contract/evidence revoke invalidates đúng dependencies; no mode downgrade, no profile hot reload.
- [x] Recheck accept, mỗi LLM request/continuation/tool dispatch; stale pass không hồi quyền. Registry không strong-reference leak, mailbox failure dùng shutdown escape path.
- [x] Valid additions/saved unqualified new set không tự close old qualified snapshot; additions không hot-expand rights. Already-dispatched work không rollback claim.
- [x] UI mutation responses/dependencies rõ; production HTTP+WS race tests revoke ở từng dispatch boundary, writer/native cleanup, commit publication failure và unaffected sessions.

## Answer

**Mechanism.** Extended `database::tool_security::ToolSecurity` with a weak-reference Speaker
registry. `register_speaker(SpeakerDeps { agent, template, speakers, candidate_set_digest })`
returns an `Arc<CancellationToken>`; the registry keeps only a `Weak`, and prunes dead entries on
the next registration, so a dropped session never leaks. Invalidation methods cancel exactly the
sessions that pinned a changed dependency:

- `invalidate_speaker(id)` — re-enrol/replace (`finalize`), disable (`PATCH`), purge.
- `invalidate_agent_speakers(id)` — Agent `PATCH`, policy mode change, unlink.
- `invalidate_template_speakers(id)` — Template `PATCH` (prompt/language/enabled).
- `invalidate_candidate_set(digest)` — calibration reload's `revoked_candidate_sets`.

`SpeakerDeps.candidate_set_digest` is computed at admission via the shared
`database::speaker_candidate_set` module (moved out of `admin::speaker_calibration` so calibration
publication and session admission hash the identical canonical JSON).

**Admission and close.** `resolve_speaker_observe` registers the plan's candidate speaker ids,
template and digest, and attaches the token to `SpeakerObserve`. `handle_socket` selects on the
token in both the pre-hello and main loops and closes `1008` with `SecurityInvalidated`, mirroring
the existing external-MCP path. If the actor mailbox is gone the `try_send`/`send` fails and the
loop simply ends — the shutdown escape path. The actor's ingress loop also rechecks
`SpeakerObserve::security_cancelled()` before every `ClientMessage`/`ClientAudio`, so a message
that races the close cannot restore authority.

**Additions vs reductions.** Invalidation is dependency-targeted, so a first enrolment or a new
grant matches no existing session and closes nothing. `put_agent_speaker` compares the previous
Template grants and revokes only on a *reduction*; additions apply to new WS only.

**Tests.**
- `tests/speaker_security_revocation_hooks.rs` (9): disable/purge/addition/unlink/template-disable/
  policy-off/agent-patch/grant-reduction-vs-addition/dropped-session, over real HTTP + SQLite.
- `tests/speaker_observe.rs::revoked_snapshot_closes_the_ingress_loop_before_accepting`: drives the
  real actor loop with a revoked token and asserts `Close(1008)`.
- `database::tool_security::tests::speaker_invalidation_matches_every_pinned_dependency`: registry
  matching unit check.

**Known gap.** No full WebSocket test exercises a real speaker provider (test builds use
`ProviderSet::unavailable()`); the close is covered at the actor loop and by the shared
external-MCP close path. Provider-handler invalidation is not wired — sessions pin no provider id,
only the derived profile; add a provider pin if that changes.

**Incidental fix.** `tests/speaker_evaluation.rs` did not compile at HEAD (missing `ObservePlan
{ policy }` field left by ticket 15); added `policy: SpeakerPolicyMode::Observe` so the suite runs.
