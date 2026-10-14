# Core Plugins In-Process — Implementation Notes

**Branch:** `feat/core-plugins-in-process` (based on `feat/agent-job-hybrid-structured-tools`)
**Status:** Local Rust source checks verified on 2026-10-14; live acceptance gates remain **NOT_RUN** (see local verification below).
**Domain decision:** [ADR 0114](adr/0114-core-operations-in-process.md) · [Agent Job flow](flows/08-agent-job-hybrid.md)

## Scope

The server ships three Core Operations, independent of Plugin Store/ZIP/images:

| Tool exposed to Root/Child | Rust operation | Output |
|---|---|---|
| `p_core_web_fetch` | `core.web.fetch` | Bounded text, HTTP status, media type, provenance |
| `p_core_data_json_query` | `core.data.json_query` | JSON Pointer-selected value |
| `p_core_artifact_read_text` | `core.artifact.read_text` | Job-owned, text-MIME, UTF-8 content |

These operations are visible only within the frozen Job/Child allowlist. Core-only Jobs do not invoke Docker image preflight or a package runner. External Plugins continue using existing installation and runner code.

## Implementation map

| Responsibility | Source |
|---|---|
| Public seam / catalog and schemas / invocation | `tools/core/mod.rs`, `tools/core/contracts.rs`, `tools/core/execution.rs` |
| Catalog freeze / namespace reservation | `services/background/plugins/mod.rs` |
| Strict LLM tool compilation and schema | `services/background/plugins/contracts.rs` |
| Root/Child frozen discovery | `services/background/agent_loop.rs` |
| Durable Step + Core/External dispatch | `services/background/execution.rs` |
| Scheduler admission | `database/jobs/eligibility.rs` |
| Start authorization | `database/jobs/execution.rs` |
| Dynamic Step authority | `database/jobs/hierarchical.rs` |
| DAG Core Step identity + Job final validation | `database/jobs/orchestration.rs` |
| Domain concepts | `GLOSSARY.md` |

External choice JSON is unchanged. Core Job choices use `kind: "core"`, `plugin_id: "core"`, `operation`, and `contract_revision`. The Core invocation adapter maps the revision onto the existing version field. Installation digest fields stay absent in frozen Core JSON (empty in legacy durable Step columns).

## Test plan

Follow `AGENTS.md`: only use `scripts/agent-test.sh` to run tests and checks.

```bash
scripts/agent-test.sh check voice-agent-server
scripts/agent-test.sh targeted core voice-agent-server
scripts/agent-test.sh targeted hierarchical voice-agent-server
scripts/agent-test.sh targeted admission voice-agent-server
scripts/agent-test.sh targeted execution voice-agent-server
scripts/agent-test.sh package voice-agent-server
scripts/agent-test.sh full
scripts/agent-test.sh run -- cargo fmt --all --check
```

Targeted tests include: frozen revision mismatch, External JSON compatibility, strict native tool projection, Core-only durable JSON query without Docker, foreign/binary artifact rejection, and updated Root allowlist/External pin regressions.

**Still required before merge:** Run formatter, compile, all targeted/full tests and end-to-end Root→Child→Core→`submit_result` with actual configured providers; test a mixed Core/External job, capacity/cancellation/deadline under load, and restart/revision mismatch. No claim of complete acceptance is made without those tests.

## Explicit exception: Risk 2

No new SSRF, DNS/IP, redirect, host-target or filesystem allowlist policy is introduced. Preserve existing checks. Core has Rust-server authority and is not sandboxed. Do not expose `core.web.fetch` to public/untrusted tenants before resolving that separate issue.

## Local review and merge verification (2026-10-14)

The local review found and fixed invalid RFC 6901 escapes and text results that
could exceed the durable Step 65,536-byte serialized JSON budget after escaping.
Regression cases cover invalid/valid escapes, control characters, Unicode,
metadata overhead and unchanged small results. Core tests now live in a separate
file, contracts and execution have separate modules, and operation metadata is
shared. Formatting and unused imports were corrected during integration.

Real-provider Root→Child→Core→`submit_result`, mixed Core/External execution,
capacity/cancellation/deadline under load, and live restart qualification remain
**NOT_RUN**. The local environment has no configured `AGENT_P3_*` qualification
credentials or `AGENT_P2_*` Installer/Runner endpoints. Source verification and a
local Git merge do not establish complete acceptance or deployment readiness.

Source verification (all commands through `scripts/agent-test.sh`):

- `targeted core voice-agent-server`: PASS before integration fixes.
- `run -- cargo test --quiet -p voice-agent-server --lib core`: PASS, 11 tests after fixes.
- `targeted background voice-agent-server`: PASS, 88 passed / 2 ignored.
- `full`: PASS, 831 passed / 0 failed / 14 ignored.
- Strict workspace Clippy: PASS after fixing the initial `collapsible_str_replace` failure.
- Formatting: PASS after fixing the initial formatting failure.
- Diff whitespace: PASS after removing trailing whitespace.

Full logs remain in the local agent-test temporary log directory. These source
checks include deterministic External, DAG, Root/Child, Voice and Reminder
regressions; ignored deployment/model tests are excluded from acceptance claims.
