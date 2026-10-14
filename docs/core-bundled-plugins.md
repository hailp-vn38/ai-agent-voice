# Bundled Core Plugins — Server Catalog and Usage

Branch: feat/core-plugins-bundled-catalog  
Status: source implementation committed; Rust CI checks remain to be verified.

## Deployment

The Rust server bundles four Core Plugins with eight safe-read operations at build time. No ZIP package, Docker image, installer transaction, or external worker is needed. Operations are frozen into Job choices on submission and exposed to Child agents only when Root permits them. External installations keep their existing Store/runtime behavior.

Read-only admin catalog: GET /api/admin/plugins/builtin (Bearer Admin token).

The response contains items with id, name, kind=built_in, status=ready, runtime.kind=in_process, runtime.language=rust and operation entries with name, description, replay=safe_read, contract_revision. Top-level total=4 and operation_total=8. The existing GET /api/admin/plugins lists External installations only; Core groups cannot be activated, uninstalled, disabled or updated via Store.

## Operation catalog

| Group | Job operation name | Arguments | Output |
|---|---|---|---|
| Core Web | core.web.fetch | url, max_chars (optional) | Bounded HTTP text/status/type and provenance |
| Core Web | core.web.extract | url, max_chars (optional) | Best-effort HTML title/plain text, URL/status and provenance |
| Core Document | core.document.extract | artifact_id, max_chars (optional) | Normalized text, MIME/size/truncation and provenance |
| Core Document | core.document.inspect | artifact_id | Text line/word/Unicode-character counts and MIME/size |
| Core Data | core.data.json_query | json_text, RFC 6901 pointer | Selected JSON value |
| Core Data | core.data.transform | json_text, mode, fields/field/equals | Transformed JSON value |
| Core Artifact | core.artifact.read_text | artifact_id, max_chars (optional) | Original UTF-8 content and size |
| Core Artifact | core.artifact.inspect | artifact_id | Verified MIME type, byte size and SHA-256 |

LLM function names are compiled by existing strict tool projection, e.g. p_core_document_extract. The LLM never chooses an executor or bypasses the frozen binding.

## JSON transforms

Pick object fields:

~~~json
{"mode":"pick","json_text":"{\"name\":\"Ada\",\"role\":\"research\"}","fields":["name"]}
~~~

Result: {"value":{"name":"Ada"}}.

Pluck one field from an array:

~~~json
{"mode":"pluck","json_text":"[{\"id\":1},{\"id\":2}]","field":"id"}
~~~

Result: {"value":[1,2]}. Missing keys are represented as null.

Filter array of objects by string field:

~~~json
{"mode":"filter_eq","json_text":"[{\"type\":\"a\"},{\"type\":\"b\"}]","field":"type","equals":"a"}
~~~

Result: {"value":[{"type":"a"}]}.

## Document and artifact constraints

All artifact inputs are server-owned IDs, not arbitrary filesystem paths. Existing job ownership/size/SHA-256 verification applies.

- Document extract/inspect and Artifact read_text require text/plain, text/markdown or application/json with valid UTF-8.
- Artifact inspect accepts any artifact media type, but verifies bytes and metadata.
- Document extract removes simple Markdown headers/emphasis/link markup and pretty-prints JSON. It is not a full Markdown parser.
- Web extract strips script/style/noscript blocks and HTML tags, decodes common entities, but does not execute JavaScript or render SPA pages.
- PDF, DOCX, OCR, images, audio and video are not parsed in this phase.

## Limits and safety

Web body: 65,536 bytes maximum; text response: 16,000 Unicode characters maximum. JSON result: 16,000 serialized bytes. Transform pluck/filter processing: 128 array entries; pick fields: 16. The existing bounded Core execution semaphore, Job cancellation, timeout and durable Attempt tracking remain in effect.

Risk 2 is explicitly deferred: no new SSRF/private-IP/DNS/redirect/host policy was added. Both Web operations must not be offered to untrusted/public tenants until this is separately resolved. Existing schema, ownership and resource limits remain required.

## Tests

Use repository test wrapper only:

~~~bash
scripts/agent-test.sh check voice-agent-server
scripts/agent-test.sh targeted core voice-agent-server
scripts/agent-test.sh targeted bundled voice-agent-server
scripts/agent-test.sh targeted hierarchical voice-agent-server
scripts/agent-test.sh package voice-agent-server
scripts/agent-test.sh full
scripts/agent-test.sh run -- cargo fmt --all --check
~~~

Before merge, complete Root -> Child -> Core -> submit_result integration against configured LLM providers, mixed Core/External execution, restart/revision mismatch, and Voice load/cancellation testing. See ADR 0114 and docs/flows/08-agent-job-hybrid.md.
