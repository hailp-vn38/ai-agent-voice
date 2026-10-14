# Core Plugins In-Process — Implementation Notes

**Branch:** `feat/core-plugins-bundled-catalog` (based on `feat/core-plugins-in-process`)
**Status:** Merged server source tests, strict Clippy and formatting verified on 2026-10-14. Live acceptance remains **NOT_RUN** (see verification below).
**Domain decision:** [ADR 0114](adr/0114-core-operations-in-process.md) · [Agent Job flow](flows/08-agent-job-hybrid.md)

## Scope

The server ships eight Core Operations grouped in four preinstalled Core Plugins, independent of Plugin Store/ZIP/images:

| Tool exposed to Root/Child | Rust operation | Output |
|---|---|---|
| `p_core_web_fetch` | `core.web.fetch` | Bounded HTTP text, status, content type, provenance |
| `p_core_web_extract` | `core.web.extract` | Plain text and title from HTML, best-effort |
| `p_core_document_extract` | `core.document.extract` | Normalized owned TXT/Markdown/JSON content |
| `p_core_document_inspect` | `core.document.inspect` | Text line/word/character statistics |
| `p_core_data_json_query` | `core.data.json_query` | RFC 6901 JSON Pointer value |
| `p_core_data_transform` | `core.data.transform` | Deterministic pick, pluck, filter_eq |
| `p_core_artifact_read_text` | `core.artifact.read_text` | Owned UTF-8 text, bounded |
| `p_core_artifact_inspect` | `core.artifact.inspect` | Verified owned artifact MIME/size/digest |

These operations are visible only within the frozen Job/Child allowlist. The authenticated read-only `GET /api/admin/plugins/builtin` lists the four built-in groups without changing the External Store's list/count. Core-only Jobs do not invoke Docker image preflight or a package runner. External Plugins continue using existing installation and runner code.

## Implementation map

| Responsibility | Source |
|---|---|
| Public seam / catalog / schemas / invocation | `tools/core/mod.rs`, `tools/core/catalog.rs`, `tools/core/contracts.rs`, `tools/core/execution.rs`, `tools/core/expanded/*` |
| Catalog freeze / namespace reservation | `services/background/plugins/mod.rs` |
| Strict LLM tool compilation and schema | `services/background/plugins/contracts.rs` |
| Root/Child frozen discovery | `services/background/agent_loop.rs` |
| Durable Step + Core/External dispatch | `services/background/execution.rs` |
| Scheduler admission | `database/jobs/eligibility.rs` |
| Start authorization | `database/jobs/execution.rs` |
| Dynamic Step authority | `database/jobs/hierarchical.rs` |
| DAG Core Step identity + Job final validation | `database/jobs/orchestration.rs` |
| Domain concepts | `GLOSSARY.md` |
| Built-in catalog HTTP handler | `app/admin/plugins.rs`, `app/admin/mod.rs` |

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

Targeted tests include: frozen revision mismatch, External JSON compatibility, projection of all eight native tool schemas, Core-only durable JSON query without Docker, document extraction and inspection, JSON transformation, artifact metadata/ownership, HTML text extraction, authenticated read-only catalog, and Root allowlist/External pin regressions.

**Remaining live acceptance:** End-to-end Root→Child→Core→`submit_result` with actual configured providers; a mixed Core/External job; capacity/cancellation/deadline under load; and restart/revision mismatch. Local source tests do not establish complete acceptance.

## Explicit exception: Risk 2

No new SSRF, DNS/IP, redirect, host-target or filesystem allowlist policy is introduced. Preserve existing checks. Core has Rust-server authority and is not sandboxed. Do not expose `core.web.fetch` or `core.web.extract` to public/untrusted tenants before resolving that separate issue.

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

## Bundled catalog integration verification (2026-10-14)

Merged `feat/core-plugins-bundled-catalog` into
`feat/agent-job-hybrid-structured-tools`, preserving the target's separate
contracts/execution/tests, RFC 6901 validation and serialized Step result budget.
The eight-operation catalog lives in `tools/core/catalog.rs`; artifact metadata
uses `database/jobs/steps.rs` rather than SQLx under tools.

All tests/checks ran through `scripts/agent-test.sh`:

- Source branch bundled and background tests: PASS (background: 90 passed / 2 ignored).
- Merged `run -- cargo test --quiet -p voice-agent-server --lib core`: PASS, 15 tests.
- Merged `package voice-agent-server`: PASS, 837 passed / 0 failed / 14 ignored
  (`agent-test.package.41hcCR`).
- Merged server Clippy with `--all-targets --all-features -- -D warnings`: PASS
  (`agent-test.run.e0gFPn`).
- Merged formatting and staged whitespace checks: PASS.
- SQLx placement check: FAIL, 24 existing violations outside Core. The original
  target commit `825fe4f8` had 25 violations; Core's query was removed, and no
  new query-location violations were introduced. Logs:
  `agent-test.run.oKLIRw` (integrated Core source) and
  `agent-test.run.XNhUJ0` (original target baseline).

Initial formatter/Clippy failures were fixed and rerun. A helper visibility error
during conflict resolution was fixed before the passing merged Core/package runs.
Live provider, Docker/deployment and hardware qualification remain NOT_RUN.
