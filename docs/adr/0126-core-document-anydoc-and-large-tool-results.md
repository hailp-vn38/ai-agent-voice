# ADR 0126 — Native document conversion and durable large Tool Results

Status: Accepted for implementation, 2026-10-19.

## Context

Agent Job V3 needs to turn verified DOCX/PDF/spreadsheet/presentation Job Artifacts into Markdown and allow larger Plugin Results without bloating the LLM context. Existing Core Document can only extract text/Markdown/JSON, while external plugin responses are restricted by JSONL stdout and an 80 KiB private RPC. Job Artifacts already provide verified immutable storage, ownership, quota and atomic publication.

## Decision

1. Extend the **existing** `core.document` group with `document.convert` and `document.read`; do not create a separately installable AnyDoc package or a second Job engine. AnyDoc is a local parser only, not OCR, LLM or Knowledge publisher.
2. A conversion consumes a server-verified Job Artifact ID and publishes a Markdown Job Artifact using the **existing** `InvokeResult`, hash/staging and SQLite Step publisher. The result JSON contains only stable metadata. Final artifact UUIDs are derived from the committed Step ledger, never invented before publication.
3. The pinned Agent Job V3 transport contract supports a server-issued `delivery=artifact` envelope for schema-validated JSON that cannot fit the effective inline/model budget. The publisher retains all raw bytes in `tool-output.json` and exposes only committed `artifact_id` and `read_job_output` continuation; the original plugin output schema is still checked before this storage transformation.
4. A **Durable Step Result** and a **Model-visible Tool Observation** are distinct domain concepts. Tool output may be bounded inline or file-backed, while the observation seen by Root/Child is a smaller, honestly truncated excerpt with a deterministic read continuation. Do not duplicate 1–8 MiB result JSON in the transcript or final coverage summary.
5. Reuse existing `[background.job_output]` pinned limits, rather than introducing overlapping `[background.tool_results]` settings. Keep legacy frozen Core operations and JSONL-v1 input contracts valid.
6. Keep the private runner RPC small. Write larger stdout into a fixed server-derived attempt file, return byte count and SHA-256, and validate that file through the trusted attempt-root. Large plugin-created files continue through the existing artifact publisher. Add one streaming SDK helper, `Outcome::attach_file`.
7. Manual operation source bindings still require an authorized completed Job. Conversion does not promote content into Knowledge or bypass Wiki proposal/review.

## Trade-offs

- Local Rust AnyDoc is simpler and private but cannot OCR scanned PDFs. `needs_ocr` remains a meaningful failure.
- The in-process parser shares server resources. Native conversion holds its capacity slot through synchronous completion; async cancellation alone cannot forcibly terminate the parser. Hard-kill process isolation is deferred until a measured need.
- Larger JSON increases storage/transport requirements, so model-visible observations remain budgeted. The durable result is authoritative, not the preview.
- Rather than migrating existing `document.extract` and `document.inspect`, the new operations preserve historical frozen behavior at the cost of two public operation names.

## Invariants

- No arbitrary source pathname, URL or provider-selected artifact ID is accepted by Core Document.
- All published Artifact IDs, media types, sizes and digests are server-verifiable and Job-scoped.
- Source and output bytes remain available after a successful Step; a failed Step does not expose a fabricated successful Artifact reference.
- An observation carries sufficient information to continue paginated reading without claiming the entire output was inspected.
- Voice Session, Device MCP, Core enable/disable at Step start and prior plugin installations are unchanged.

## Related

- [Implementation guide](../core-document-anydoc-large-tool-results-v3-implementation.md)
- [ADR 0114 — Agent sessions](0114-opencode-agent-session-tool-delegation.md)
- [ADR 0115 — Core activation](0115-core-plugin-activation-policy.md)
- [ADR 0115 — Job artifacts](0115-job-owned-local-artifacts-v3.md)
- [ADR 0117 — Knowledge adapter](0117-knowledge-core-plugin-adapter.md)
