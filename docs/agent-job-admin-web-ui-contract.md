# Agent Job Admin Web — three-page integration contract

Status: frontend routes implemented on branch feat/agent-job-admin-pages. Rust Background Engine and Python Isolated Worker backend are NOT implemented by these Web commits.

## Approved scope

Three new top-level Admin pages. Existing AdminShell, PageHeader, Badge, Button, BaseModal, studio-panel, color tokens and Floating Assistant stay unchanged:

| Page | Route | Backend |
|---|---|---|
| Background | /background | GET/PUT /api/admin/background/settings |
| Plugin Store | /plugins | GET /api/admin/plugins, GET /api/admin/plugins/:id, POST /api/admin/plugins/install, POST /api/admin/plugins/:id/{test,activate,disable}, DELETE /api/admin/plugins/:id |
| Jobs | /jobs | GET/POST /api/admin/jobs, GET /api/admin/jobs/:id, POST /api/admin/jobs/:id/{cancel,retry} |

Plugin Detail and Job Detail are modal views inside their pages, NOT new routes.

## Authority

- All existing Agents may submit background jobs. No background_enabled column, Agent Detail switch, per-Agent grant or permission API.
- One global Background switch and exactly two LLM Providers; source-qualified references.
- A Voice-origin Job retains immutable requester Device incarnation / assigned Agent and verifies current Device enabled/assignment and Agent enabled before each dispatch. Admin Jobs use Admin scope. No Background Agent switch.
- Executable Python plugin packages run in isolated workers. External MCP does not execute plugins.
- Backend alone enforces authorization, quotas and the container sandbox; Web requests are untrusted.

## Backend correctness contract

- Backend missing: pages call real Admin APIs and show errors if unavailable. They do not invent statuses or execute plugin code in browser.
- PUT Settings and Plugin mutations send saved revision through If-Match; backend must reject conflicts with 409.
- ZIP upload is multipart under package; server enforces limits, ZIP traversal checks, content digest, dependency installation, sandbox.
- Plugin test is diagnostic. Activate is the only verified disabled/error -> ready action.
- Plugin activation requires Admin authorization and a revision guard, but there is no per-plugin network approval. Runtime workers use Docker's default network directly; the server does not provide an egress proxy or host allowlist.
- Runtime error state is not triggered by legitimate empty search results.
- Retry creates a new Job, never blindly replays unsafe or unknown-outcome operations.
- Recover cancelling Jobs to cancelled after crash, recording unknown_outcome if external execution may have started.
- Allowed plugin operations are snapshot at submit. Replan must not expand choices.
- Artifact download and logs require separate secure backend routes and are not faked in frontend.
- GET /api/admin/background/settings supplies llm_providers [{ ref, name, source }] with canonical source-qualified refs. The frontend never synthesizes refs from catalog keys. PUT settings echoes the list or the client retains the last read options.

## Validation

Run npm run typecheck, npm run build, npm test on the complete repo with dependencies. Verify empty/404 states, revision conflicts, ZIP upload, Job list, details, cancellation, new Job retry, and regression tests for Voice/Reminders/MCP.
