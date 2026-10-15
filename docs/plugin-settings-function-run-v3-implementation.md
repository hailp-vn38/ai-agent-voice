# Plugin Settings & Manual Function Run V3 — Implementation Record

**Branch:** `feat/plugin-settings-manual-run-v3`, based on `feat/agent-job-v3-opencode-session-runtime` on 2026-10-15.  
**Status:** Code pushed incrementally through GitHub Contents API; **Rust/Web builds and end-to-end tests have not been executed in this environment**.

## Existing modules reused

- `migrations/0119_core_plugin_settings.sql`, `database/core_plugins.rs`, `tools/core/catalog.rs`: Core activation and stable group ownership.
- `database/jobs/execution.rs` and `services/background/execution.rs`: Step-start permission and durable output publication; Core remains in-process.
- `services/background/plugins/dispatch.rs`: existing JSONL-v1 External plugin runner, opt-in settings envelope.
- `services/background/artifacts.rs`: existing Job-owned artifact directory and immutable output publication.
- `services/background/agent_loop/*`: unchanged OpenCode Root/Sub Agent flow.

## Implemented changes

| Area | Files | Contract |
|---|---|---|
| Value Settings persistence | `0120_plugin_value_settings.sql`, `database/plugin_configs.rs` | Current pointer, immutable revisions, Job pins, no activation flag duplication |
| External manifest settings | `plugins/archive.rs` | Optional `[settings] schema`, flat V1 scalar schema, reserved `core.*` namespace, no protocol bump |
| Settings read/write | `plugins/settings.rs`, `app/admin/plugins.rs` | Redacted GET, CAS PUT with independent `If-Match`, secret-set flags, encrypted refs |
| Job settings authority | `database/jobs/submission.rs`, `plugins/dispatch.rs`, `background/execution.rs` | Pin revisions at admission; resolve pinned secret immediately before dispatch; emit `settings` only for declared schemas |
| Manual mode | `0121_plugin_operation_jobs.sql`, `database/jobs/manual.rs`, `background/manual.rs`, `background/scheduler.rs`, `model.rs`, `engine.rs` | `plugin_operation_v3`: one Step, no Agent Runs, no LLM pins, own Job terminal state |
| Admin operation call | `app/admin/mod.rs`, `app/admin/plugins.rs` | Contract lookup and asynchronous POST Run (202 with Job ID) |
| Web | `api/agent-plugins.ts`, `PluginManagePanel.vue`, `PluginDetailPage.vue`, `PluginStorePage.vue` | Core/External Settings, Functions, Runs history and Job Detail navigation |
| Input artifact authority | `engine.rs`, `database/jobs/manual.rs` | Staged, verified copy from terminal Job; register as new Job-owned artifact; max 4 bindings and 20 MiB |
| Idempotency | `engine.rs`, `database/jobs/manual.rs`, Admin/API/Web | Fingerprint original request, replay by `Idempotency-Key`, 409 on mismatched payload |
| Rust SDK and example | `plugins/rust-plugin-sdk`, `plugins/web-extractor`, `scripts/package-rust-plugins.py` | Optional `Request.settings`; web extractor supports default limits from schema; packaging includes settings descriptor |

## HTTP interface

- `GET /api/admin/plugins/{plugin_key}/settings` → `{plugin_key,revision,values,secret_set,needs_reconfiguration,schema,schema_digest,package_digest}`. Unconfigured revision is `0`; no secret plaintext/ciphertext is returned.
- `PUT /api/admin/plugins/{plugin_key}/settings` requires `If-Match: "0"` for initial configuration or current version thereafter. Body `{"values":{...}}`. The response carries the committed new revision. Secret fields not provided retain their previous value; `null` clears an optional secret.
- `GET /api/admin/plugins/{plugin_key}/operations/{operation}` → existing declared input/output contracts.
- `POST /api/admin/plugins/{plugin_key}/operations/{operation}/runs` with `{"arguments":{...},"artifact_bindings":[...]}` → `202 {"job_id":"...","status":"queued","execution_mode":"plugin_operation_v3","replayed":false}`. Header `Idempotency-Key` is optional; same request replays as `200`, mismatched request returns `409`.
- `GET /api/admin/plugins/{plugin_key}/runs?page=1&page_size=20&operation=...` returns filtered Job history.
- Existing `GET /api/admin/jobs/{id}`, Job events, Artifacts, cancel/delete remain in use.
- Existing Core `PATCH /api/admin/plugins/builtin/{id}` retains its activation revision and ADR 0115 start semantics.

## Manual execution state sequence

```text
Admin POST Run -> validate contract -> persist Job(queued) + Step(pending)
Scheduler -> claim(planning) -> running -> reserve Step + Attempt
  -> Core invocation (Rust) OR External invocation (shared worker)
  -> finish Step and Job atomically -> succeeded / failed / cancelled
Restart -> existing recovery marks uncertain attempts without replay
```

Manual mode does not create Root/Child Session records or LLM usage. The scheduler must not claim Agent Jobs on the test harness without models.

## Security and compatibility invariants

- Only declared operations; External must be ready and `safe_read`, Core must be enabled at Start.
- No arbitrary code execution, user-provided entrypoint or worker runtime switch.
- Plugin config versions use separate revisions from activation and installation. Existing Jobs with no pins never gain live secrets.
- Secrets are `sealed:` references with plugin/schema/version/field scope; old values are decrypted and re-sealed when carrying forward to a new revision.
- Existing External fixtures without `[settings]` receive no additional JSONL keys.
- No secrets in `jobs.request_json`, `job_steps.arguments_json`, `job_events` or public settings APIs.

## Implemented but not yet verified against a Rust/Web build

- **Artifact input binding/import**: implemented bounded, hash-checked, Job-owned staging and metadata publication. The initial API accepts at most four bindings, only for Core document/artifact operations, with a 20 MiB aggregate cap and further Job quota bound.
- **Server-side per-plugin Runs filters**: implemented SQL-side filtering and Core operation namespace matching with Web Runs tab/pagination.
- **Idempotency**: manual Run now accepts `Idempotency-Key` and maintains request-fingerprint deduplication under the admission transaction; Web retries the same request with its original key.
- **Plugin upgrade/reconfiguration**: detected incompatible package/schema returns `needs_reconfiguration=true` and a new CAS PUT starts a clean revision without carrying secrets from the old package. Previously pinned revisions are untouched.
- **Rust Plugin SDK/example**: optional one-shot JSONL `settings` payload is accepted by `plugins/rust-plugin-sdk`; `web-extractor` consumes bounded deployment defaults. Rust packager includes the schema file.

## 2026-10-15 implementation corrections (post-Code Review)

- **P0 scoped secrets:** `CredentialCipher::seal` returns an encrypted JSON record, not a `sealed:` reference. The new `plugins/settings/secrets.rs` wraps it with `credentials::reference(scope, record)`, authenticates the saved scope before decrypting, re-seals retained secrets at every settings revision, and accepts authenticated legacy raw records for migration. Unit tests cover roundtrip, cross-plugin rejection, cross-revision rejection, and legacy format.
- **P1 bounded worker settings:** `plugins/settings/validation.rs` enforces a 16 KiB serialized limit on the entire decrypted settings object both at Admin settings update and immediately before dispatch. Tests include escaped UTF-8/JSON and multiple large secrets.
- **P1 terminal checkpoint:** `background/manual.rs` now performs at most three five-second attempts. Persistent SQLite failure stops admission (fail-closed) and leaves startup `recover_jobs()` responsible for the uncertain Job; no unbounded background task.
- **P1 architectural seams:** Manual admission and Job-owned input import moved out of `engine.rs` into `manual_admission.rs`. Settings encryption and budget validation reside in separate tested submodules. The public JobEngine/API contract remains stable.
- **P2 secret clearing:** Admin Web exposes Clear / Undo clear for saved secrets. PUT submits explicit `null` to remove optional secrets, while omitted fields retain the current value.
- **Regression tests:** Added a production-runtime without managed LLM models, SQLite `foreign_key_check`, SDK sealed-reference tests, bounded JSONL tests, web-extractor settings with real operation output/artifacts.

## Outstanding qualification gates

- **Build and CI have not been run**: no Rust toolchain or locally checked-out repository available in this execution container, and the GitHub branch has no reported check runs. Execute the full commands below on a configured developer/CI runner.
- **Forward migration under live data**: apply 0120/0121 to a populated copy of a server database; check FKs, persisted Jobs, Session Run references and existing triggers.
- **External runtime E2E**: run Installer/Runner sockets with the updated bundled Rust plugin, verify its Settings override appears in the result while the old fixture still receives no new envelope key.
- **Safety regression**: test disable-before-start vs disable-after-start, queued run cancellation, restart before/after accepted Step, failed artifact copy cleanup, and a replay racing the same key.
- **Schema-level and UI tests**: exercise empty/required secrets, CAS races, secret rotation, plugin upgrade, browser state and strict existing JSONL clients.

## Suggested verification commands (repository root)

```sh
cargo fmt --all -- --check
cargo check -p voice-agent-server
cargo test -p voice-agent-server plugin_only -- --nocapture
cargo test -p voice-agent-server
npm --prefix apps/admin-web run typecheck
npm --prefix apps/admin-web run test
```

Check actual workspace package IDs and scripts in the repository manifest before executing, and preserve the existing ADR/Flow files and V3 Root/Sub-Agent contract.
