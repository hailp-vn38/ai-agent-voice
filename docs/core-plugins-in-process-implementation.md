# Core Plugins In-Process — Implementation Notes

**Branch:** `feat/core-plugins-bundled-catalog` (based on `feat/core-plugins-in-process`)  
**Status:** Implemented in branch; Rust compilation and acceptance tests are **not yet verified** by this environment.  
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
| Static Core contract and invocation | `tools/core/mod.rs` and `tools/core/expanded/*` |
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

**Still required before merge:** Run formatter, compile, all targeted/full tests and end-to-end Root→Child→Core→`submit_result` with actual configured providers; test a mixed Core/External job, capacity/cancellation/deadline under load, and restart/revision mismatch. No claim of complete acceptance is made without those tests.

## Explicit exception: Risk 2

No new SSRF, DNS/IP, redirect, host-target or filesystem allowlist policy is introduced. Preserve existing checks. Core has Rust-server authority and is not sandboxed. Do not expose `core.web.fetch` or `core.web.extract` to public/untrusted tenants before resolving that separate issue.
