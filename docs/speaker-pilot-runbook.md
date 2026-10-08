# Speaker Pilot V1 — Operator Runbook

This runbook covers the web-enrollment + ESP32 Observe pilot. It is an operator
procedure, **not** a new eligibility workflow: there is no RBAC, scheduler, or
model-manager product here. The only authorization surface is the existing Agent
Tool Allowlist and the speaker grant described below.

## 1. What the pilot can and cannot do

| Capability | State |
| --- | --- |
| Web enrollment (draft → samples → holdout → publish) | Shipped |
| ESP32 Observe (identify, no unlock) | Shipped |
| `Required` speaker enforcement | Shipped, **off unless the guard below is satisfied** |
| Real CAM++ accuracy claim | **Not made** — requires Optional Runtime Evidence |

## 2. Enabling `Required`

`Required` is honored for an agent only when **all** of these hold:

1. The agent is granted the speaker with the exact `Required` mode.
2. A voiceprint is published and its `calibration_revision` matches a catalog
   revision that was explicitly published for that agent.
3. The turn runs on a connection that saw the activation (`new_connections`
   effective, or an explicit re-open).

Deterministic CI (the Mandatory Qualification build) is **preliminary evidence
only**. Do not enable `Required` on CI alone: it uses the model-free
`qualification_speaker` adapter and proves contracts, not accuracy.

To turn enforcement off, remove the `Required` grant (or switch the agent back
to `Observe`). No restart is required for new connections.

## 3. Operator review loop

The operator reviews, and re-reviews whenever a source changes:

- **Persona** — the agent's persona text.
- **Prompt** — the prompt template that wraps the persona.
- **Context** — the context assembly sources that feed a turn.
- **Tool results** — what the allowlisted tools returned for a turn.

A source change is a manual re-review trigger. There is no automated eligibility
gate and no approval workflow to add one.

## 4. History isolation and replay/privacy limits

- Observe history is scoped to a single session and a single connection. It is
  never replayed across sessions or devices.
- Qualification reports are privacy-safe: they contain **no audio, no
  embeddings, and no credentials** — only results, check names, and revisions.
- Enrollment drafts are bounded (quota + TTL) and expire; an expired draft is
  not recoverable and must be restarted.

## 5. Running the Mandatory Qualification harness

The harness spawns the production entrypoint with compile-time deterministic
providers, waits for the startup handshake artifact, drives the public Admin
HTTP boundary, and restarts the process to prove durability.

```bash
cargo test -p voice-agent-server --features qualification-providers \
    --test qualification_harness -- --ignored --nocapture
```

It writes `./.scratch/speaker-recognition/evidence/pilot-handoff-qualification.json`.
A `pass` there means the contracts hold; it is not an accuracy measurement.

## 6. Running real Optional Runtime Evidence

Real evidence is operator-supplied and machine-specific. It needs:

- real CAM++ model assets, and a recorded speaker corpus (same/different
  speakers), and
- ESP32 Observe hardware on the target machine.

Run the provider bench with the real model, capture the corpus results, and
record them next to the qualification report. **If the corpus or hardware is
absent, record `not_run` and the missing prerequisite** — do not infer accuracy
from the deterministic run.

## 7. Rollback

1. Remove the `Required` grant (immediate for new connections).
2. Optionally delete the published voiceprint (`DELETE` with `If-Match`).
3. Optionally disable the speaker provider revision.

No schema migration or process restart is required to roll back enforcement.
