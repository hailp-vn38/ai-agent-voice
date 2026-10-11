# Provider configuration sources and Template bindings

`GET /api/admin/providers` lists database and deployment-configured Providers together. Every item has `source: "database" | "config"`. Filtering (`q`, `type`, `enabled`), sorting (`key`, `-key`, `name`, `-name`), counts and pagination operate on that union. Facets honor search/enabled filters and ignore the selected type. `speaker` is also a supported filter and Template slot. The existing maximum page size remains 200.

Database keys retain their existing format. Config references use `config:<kind>:<raw_key>`; references retain their origin even when both origins have the same raw key. Config keys must be nonempty, at most 128 ASCII bytes, and contain only letters, digits, `_` or `-`. Kinds are `vad`, `asr`, `llm`, `tts`, and `speaker`; vision is outside this binding contract.

Config items use `id=0`, `revision=1`, zero timestamps, and `config_json="{}"`. These are read-only metadata conventions, not SQLite identities. Credentials, deployment endpoints, paths and private runtime configuration are not returned. Runtime state is inspected without preparing a model; membership in this catalog does not establish readiness.

`GET /providers/{key}` and `GET /providers/{key}/templates` support qualified config references. `PATCH` and `DELETE` return **409 `config_provider_readonly`** for config references before any database mutation. Saved-provider inference tests and capabilities return **409 `config_provider_operation_unsupported`** for config references. `POST /providers/{key}/prepare` with `{}` uses the existing deployment acquisition path in managed runtime mode and returns the existing `provider_key`, `desired_revision`, and `runtime` preparation shape. It uses the manager's ordinary deadline, resource budget and error mapping.

`PUT /templates/{template_key}/providers/{kind}` retains `{ "provider_key": "..." }` and the required quoted `If-Match: "<revision>"`. It accepts a database key or a qualified config reference of the matching kind. Missing/malformed/mismatched references return `invalid_provider` without advancing revision. Bind/unlink mutations retain revision CAS and audit atomicity.

Config bindings require managed runtime mode. Legacy mode rejects a new config binding or explicit prepare with **409 `config_provider_requires_managed_runtime`**. A persisted config binding cannot silently fall back in legacy mode. Enable the deployment's existing `provider_runtime` configuration before binding config instances.

Template relationship reads retain qualified keys; a binding whose config instance has disappeared returns `enabled=false`. The binding is preserved until explicitly removed or replaced. New admissions fail closed for missing explicit bindings. Only unbound core slots inherit deployment defaults; an unbound optional Speaker slot remains absent.

Managed startup indexes every bindable config instance's immutable metadata. Only existing defaults/preload instances are acquired at startup; other instances are acquired on demand. Exact database/deployment Provider Versions and Resource Leases are retained through admission and Template switching. Template prewarm uses the same source resolution and bounded speculative manager path.

Admin Web retains its components and layouts. Its store fetches all provider pages (100 items each) and preserves qualified keys, including Speaker bindings. Existing edit/delete buttons can still appear for config items; the server enforces read-only behavior.

Migration `0101_template_config_provider_bindings.sql` adds source-aware bindings without creating config rows in `providers`. It preserves existing IDs, timestamps and AUTOINCREMENT high-water marks, checks foreign keys, and retains database Provider delete restrictions. As with other forward migrations, an older binary requires a compatible backup to roll back the schema.
