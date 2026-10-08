# Speaker Pilot V1 — Handoff

**Branch:** `speaker/19` · **Baseline:** `99cce86` (on top of tickets 01–17)
**Scope:** the final ticket of the Speaker Recognition V1 spec — public
qualification and the V1 pilot handoff.

## What ships

1. **Compile-time Qualification Provider seam** (ADR 0068), feature
   `qualification-providers` on `voice-agent-server`:
   - `qualification_speaker`, a deterministic, model-free Speaker Provider.
   - It is registered only under the feature; the default and release builds are
     unchanged and still expose only `campplus_sherpa`.
   - Under the feature the binary skips model download and deployment-provider
     materialization, so it boots with no assets and no credentials.
2. **Integration Harness / Reference Integration Client** —
   `crates/voice-agent-server/tests/qualification_harness.rs`. It spawns the
   **production binary**, waits for the nonce-bound startup handshake artifact,
   drives the public Admin HTTP enrollment boundary with its own wire types, and
   performs a **controlled SIGTERM restart** to prove durability.
3. **Qualification Report** —
   `.scratch/speaker-recognition/evidence/pilot-handoff-qualification.json`
   (privacy-safe: no audio, no embeddings, no credentials).
4. **API collection** — sample upload, holdout validation, and finalize added to
   the `Enrollment drafts` folder in `docs/api/00-all-apis.postman_collection.json`.
5. **Operator runbook** — `docs/speaker-pilot-runbook.md`.

## Evidence

| Check | Command | Result |
| --- | --- | --- |
| Default build | `cargo check -p voice-agent-server --tests` | PASS |
| Qualification build | `cargo check -p voice-agent-server --features qualification-providers` | PASS |
| Mandatory Qualification | `cargo test -p voice-agent-server --features qualification-providers --test qualification_harness -- --ignored` | PASS (1/1) |
| Existing server suite | `cargo test -p voice-agent-server` | PASS except 1 pre-existing failure (below) |

The one failing test,
`session_profile::public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version`,
asserts `200` but gets `202` at `session_profile.rs:1785`. It **reproduces with
this ticket's changes stashed** (verified), so it is pre-existing and unrelated
to ticket 19. It exercises real-provider materialization over a live WS session,
not the enrollment/qualification path.

### Mandatory Qualification — result

Deterministic `qualification-providers` run: **PASS**. It proves, through the
production process boundary:

- the startup handshake artifact is nonce-bound and machine-readable;
- admin enrollment is reachable over public HTTP with bearer auth;
- the deterministic speaker loads on demand without a model;
- holdout validation gates finalize;
- the published voiceprint survives a controlled process restart.

### Optional Runtime Evidence — result

**NOT_RUN.** Missing prerequisites:

- real CAM++ model assets and a recorded speaker corpus;
- ESP32 Observe hardware on the target machine.

No accuracy claim is made. `Required` activation is **not** deployed on the
strength of the deterministic run.

## Deliberately not built

No hidden bypass, model manager, scheduler, RBAC, or eligibility product. The
only authorization surfaces remain the Agent Tool Allowlist and the speaker
grant.

## Known gaps / follow-ups

- **Process-level Voice WS Observe is NOT_RUN.** The harness covers the
  enrollment slice through the production process; the Observe slice is covered
  today only at the actor/wire level (`speaker_observe.rs`, `protocol_e2e.rs`).
  Driving it through the production binary needs the remaining ADR 0068
  qualification providers (`qualification_vad/asr/llm/tts`), which are not
  implemented yet. The harness currently declares real adapters as defaults and
  skips building them under the feature.
- **Frontend checks NOT_RUN** — no frontend changes were made in this ticket.
- **Real-model gate NOT_RUN** — requires the corpus/hardware above.
