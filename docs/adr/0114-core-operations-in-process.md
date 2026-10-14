# ADR 0114 — Built-in Core Operations share the Agent Job execution seam

- **Status:** Accepted; bundled catalog scope amended (2026-10-14)
- **Reference:** `feat/agent-job-hybrid-structured-tools`
- **Related:** ADR 0103, ADR 0113, `docs/flows/08-agent-job-hybrid.md`

## Context

Agent Job Hybrid V2 freezes installed Plugin Operations in `jobs.plugin_choices_json`, projects their original JSON Schemas into strict native function tools, and records durable Steps/Attempts before calling the installed Plugin Store runner. Common safe-read operations should work without Docker or a separately installed package. Converting them into fake installations would blur deployment ownership and tie system tools to external lifecycle.

## Decision

1. **Core Operation** is shipped with the Rust binary and runs in-process. **External Plugin Operation** keeps the installed package lifecycle and existing runner. Core is not a Plugin Store installation; it cannot be installed, hot-reloaded or disabled through Store.
2. Use one frozen operation catalog for Job admission/Root/Child. Core frozen JSON has `kind=core`, `plugin_id=core`, `operation`, and `contract_revision`; old External JSON remains byte-compatible. `core` is a reserved namespace.
3. The existing `FrozenChoice` is the invocation representation. Decoding Core JSON into an ephemeral choice carries its revision in `version` and uses empty digest fields only to represent absence of an installation at the legacy Step seam. No fake Plugin Store rows, package digests or image digests are created. The original Core revision is authoritative.
4. Keep `compile_child_tools()`, native Function Calling, Root Structured Outputs, schema validation, normalized optional arguments, bounded budgets, cancellation, and Step/Attempt/event persistence. In execution, branch Core directly to a statically linked Rust function; otherwise keep `PluginStore::invoke()`.
5. **Six compile-time Core Operations** are exposed in three Core Plugins: Document (`document.extract`, `document.inspect`), Data (`data.json_query`, `data.transform`), and Artifact (`artifact.read_text`, `artifact.inspect`). Web (`web.fetch`, `web.extract`) is hidden from the catalog and new Job choices; its contracts and execution remain supported for already frozen Jobs. Document parsing remains limited to text/plain, text/markdown and application/json. No PDF/DOCX/OCR, dynamic loading, script execution or general-purpose plugin framework.
6. Scheduler/Job-finalization revalidate Core contract revisions and External installation identities independently. Changes to a frozen Core revision must fail closed.
7. Core returns the same `InvokeResult` form. Output schemas, bounded results, error codes and evidence semantics stay Rust-owned.
8. A read-only authenticated catalog at `GET /api/admin/plugins/builtin` exposes three system-provided Core Plugin groups and their operation contracts. It does not create installation rows or allow install/activate/disable/uninstall. The existing `/api/admin/plugins` remains the External Plugin Store listing.

## Alternatives rejected

- Installing Core operations as immutable fake Plugin Store ZIPs: extra lifecycle, identity and image assumptions.
- A second tool registry or an `OperationExecutor` trait/framework: unnecessary abstraction for exactly two known dispatch variants.
- Dynamic Rust libraries: ABI, deployment and process-safety complexity.
- Rewriting Agent Job Orchestration: unnecessary; Rust already owns accepted actions and Step durability.

## Consequences

- Core-only Jobs require no Docker runtime or plugin installation, but still require configured LLM providers and the normal Job engine.
- Core runs with the server's process authority. A panic, blocking call or heavy workload may affect Voice; operations must stay bounded and cancellation-aware. Tokio timeouts cannot preempt a blocked native task.
- Legacy Step columns describe an External installation. For Core, `plugin_id=core`, `plugin_version=contract_revision`, and empty digest fields encode the absence of a package. The trusted frozen Job choice and source contract remain the authority. A future migration may rename these fields if the persisted model grows.
- Job Artifact **content** reads verify ownership/bytes and accept only text media types; `artifact.inspect` verifies metadata for any artifact MIME type. Core writes and binary generation are not in scope.
- **Risk 2 expressly deferred:** no new network target/SSRF/private-IP/DNS/redirect or filesystem allowlist policy for Core is introduced here. Preserve existing restrictions. Neither `core.web.fetch` nor `core.web.extract` should be exposed to untrusted/public tenants until separately reviewed.
- Re-run existing External, `dag_v1`, Root/Child, voice and reminder regression tests; Core acceptance tests focus on revised frozen contracts and in-process dispatch.

## Verification limitation

Changes were written through the connected GitHub repository because the execution environment has no Rust toolchain and no direct network checkout. `cargo fmt --check`, `cargo check`, and the complete suite must be run by CI or a Rust-equipped checkout before merge; do not consider this ADR a test-pass assertion.
