# SQLite Agent/Template Database Integration

Status: ready-for-agent

## Problem Statement

Người vận hành Voice Agent cần quản trị Device, Agent, Template, Provider và External MCP một cách persistent, có kiểm soát và an toàn, thay vì chỉ dùng server defaults tĩnh. Đồng thời hệ thống phải giữ Voice semantics realtime: database, history archive, Admin API, provider desired configuration và External MCP không được âm thầm làm đổi Voice Session đang mở, block audio path hoặc làm lộ nội dung/credential. Hiện application chưa có SQLite control plane, typed repository/API surface, Device admission DB-backed, profile snapshot, hay policy rõ ràng cho persistence, migration, secrets và lifecycle vận hành.

## Solution

Thêm SQLite control plane opt-in cho Voice Agent, với schema/migration forward-only, typed repositories và Admin API tách auth; resolve Device → Agent → Template → RuntimeCatalog/MCP thành Effective Session Profile immutable tại WebSocket admission. Database Desired Configuration tách hoàn toàn Loaded Runtime; mọi runtime-affecting change chỉ effective sau restart. External MCP, history archive và operational maintenance đều bounded/fail-soft theo contract, còn Voice realtime path tiếp tục dùng application-owned runtime và RAM Dialogue History.

## User Stories

1. As an operator, I want to enable or disable the database explicitly, so that schema presence never changes Voice behavior by itself.
2. As an operator, I want a SQLite database to open with WAL, foreign keys and bounded lock waiting, so that local persistent control data is reliable.
3. As an operator, I want a fresh database to contain only schema and indexes, so that provisioning is always explicit.
4. As an operator, I want one local Voice Agent process to own each database path, so that V1 avoids unsupported active-active storage behavior.
5. As an operator, I want forward-only migrations with fail-fast incompatibility detection, so that an old binary never runs against a newer schema by accident.
6. As a release owner, I want backup, restore and rollback to remain explicit operational actions, so that the application never performs unsafe schema downgrade or file copying.
7. As an admin, I want authenticated CRUD/query APIs for Agent, Template, Provider, MCP Server and Device, so that I can manage desired configuration without direct SQLite edits.
8. As an admin, I want the Admin API to use a token independent from Voice and OTA auth, so that privileges are separated.
9. As an admin, I want strict Bearer parsing, constant-time credential comparison and redacted errors, so that malformed credentials do not expose security details.
10. As an admin, I want Admin mutation bodies to have a shared size, encoding and JSON boundary, so that large or compressed payloads cannot exhaust the service.
11. As an admin, I want typed pagination, filter and sort limits, so that database queries remain bounded and cannot accept raw SQL expressions.
12. As an admin, I want optimistic concurrency through revisions, so that conflicting updates never silently overwrite another administrator's changes.
13. As an admin, I want PATCH to distinguish an omitted field, a replacement and an explicit clear, so that null behavior is never ambiguous.
14. As an operator, I want Agent, Template, Device and MCP fields to have explicit byte/JSON structure bounds, so that persistent configuration cannot become an unbounded data bag.
15. As an admin, I want configuration resources to be soft-disabled rather than hard-deleted, so that Voice history and live session semantics remain stable.
16. As an operator, I want Admin mutation audit metadata without request/config/secret content, so that administration is traceable without creating a sensitive archive.
17. As an operator, I want transcript capture disabled by default, so that Voice conversations are not persisted without explicit opt-in.
18. As an operator, I want enabled transcript archive to store only final user text and assistant text after normal writer closure, so that partial ASR, deltas, prompts, tools and audio remain private.
19. As an admin, I want transcript retention and explicit scoped purge, so that archive deletion is bounded and intentional.
20. As a Voice Protocol Client, I want unknown or disabled Device admission to fail before WebSocket upgrade when enabled, so that only provisioned devices enter the service.
21. As a developer performing migration, I want optional auto-registration to bind a new Device to one enabled Agent atomically, so that no unassigned or duplicate Device is created.
22. As a Voice Protocol Client, I want Device admission to resolve one Effective Session Profile, so that all turns of an open session have consistent configuration.
23. As an operator, I want sessions already admitted to survive later DB/config mutation or DB degradation, so that control-plane changes do not silently live-reconfigure Voice.
24. As an operator, I want invalid default Template/Provider configuration to fail closed for new sessions, so that server defaults never mask broken intended configuration.
25. As an operator, I want an Agent without any Template assignment to use server defaults, so that legacy Voice behavior remains available.
26. As a Voice Session, I want template switching to use only its admission-time Template Switch Catalog at a next-turn boundary, so that switching cannot query live DB state.
27. As an admin, I want Provider desired configuration to state restart requirements honestly, so that a database update is never represented as a hot-loaded runtime.
28. As an operator, I want required Provider failures to block startup and optional Provider failures to exclude only dependent non-default candidates, so that availability is explicit and bounded.
29. As an operator, I want provider configuration to be typed, credential-free and structurally bounded, so that credentials cannot be hidden in arbitrary JSON.
30. As an operator, I want credentials resolved only through SecretRef and a deployment-owned SecretResolver, so that SQLite and Admin API never store or expose secret values.
31. As an operator, I want provider secrets snapshot at startup and External MCP secrets snapshot at session admission, so that secret rotation has deterministic restart/new-session visibility.
32. As a Voice Session, I want Agent-bound External MCP discovery to fail soft by default, so that an unavailable optional server does not reject Voice admission.
33. As an operator, I want External MCP connections limited by network allowlists, TLS validation, typed authentication and per-server concurrency, so that database configuration cannot become an arbitrary HTTP client.
34. As a Voice Session, I want the Session Tool Catalog to remain immutable after admission, so that transient tool failures do not silently remove capabilities.
35. As a Voice Session, I want Device MCP and External MCP calls to execute sequentially under bounded call, round and time budgets, so that side effects and continuation order remain deterministic.
36. As a Voice Session, I want history persistence and maintenance failures to remain non-authoritative, so that Dialogue History, prompting and turn outcomes stay correct.
37. As an operator, I want `/health` to show process liveness and `/ready` to show ability to accept new connections, so that orchestration reacts to the correct dependency state.
38. As an operator, I want shutdown to stop new admissions and drain existing sessions within a bounded deadline, so that deploys do not wait indefinitely or create new side effects while stopping.

## Implementation Decisions

- Add SQLx/SQLite as the persistent control plane only. The PCM/audio frame path remains database-free. Database activation is explicit; DB-1 and DB-2 must not change WebSocket admission, provider resolution, Agent/Template/MCP behavior or history runtime.
- Use a single local SQLite owner process per database path. Network filesystems, multi-process writers and active-active deployments are unsupported in V1. Migrations create schema/index only; no implicit Agent, Template or Device seed.
- Configure SQLite with WAL, foreign keys, synchronous normal and a validated busy timeout. Use SQLx migration history as the only authoritative migration state. Migrations are forward-only; newer-than-binary schema, pending schema when migration-on-start is disabled, or a migration error fail startup before listener bind. Backup/restore is operator-owned and must be SQLite-consistent under WAL.
- Add typed database models, repositories, services, query helpers and error mapping. SQL stays outside Axum handlers and SessionActor. Repositories own SQL; services own resolution and transaction boundaries.
- Admin API mounts only when explicitly enabled and database is enabled. It has a separate non-empty admin token. Require exactly one valid Bearer header, use constant-time comparison, return 401 for all missing/malformed/wrong credentials, emit no CORS/cookie auth in V1, and leave brute-force throttling to deployment proxy/network controls.
- Use a server-generated request ID for every Admin request and error envelope. Do not trust client correlation IDs. Admin mutation success and audit metadata insert are one transaction; audit insert failure rolls back mutation. Revision conflict remains 409 even if its best-effort audit fails.
- Apply a shared Admin JSON transport boundary: request ID, non-content-parsing transport protection, auth, JSON extraction, then handler/service. Accept only identity/no content encoding and application/json with valid parameters. Reject compressed body, oversized body, invalid content type and invalid JSON with coarse status/code. Do not log body or parser fragments.
- Use typed pagination and typed allowlisted filters/sorts. Default page size is 50, maximum is 200, page starts at one, cursor/page token is bounded, and filter/search/sort predicate counts and values are bounded. Offset is not client-unbounded; future large history should use cursor pagination.
- Use a typed PATCH field intent with Absent, Set and Clear. Clear is valid only for nullable/clearable fields. Immutable or non-nullable clear is a 400; generic JSON Merge Patch is forbidden.
- Bound Resource Key, Device identity, names, descriptions, language, prompt, URL, metadata JSON and static MCP headers JSON as specified in the guide. Metadata JSON has independent depth/node caps. All bounds reject before persistent write/query and do not echo sensitive values.
- Persist Agents, Templates, assignments, Provider instances, Template Provider bindings, MCP servers, Agent MCP bindings, Devices, optional history messages and audit events. Resource keys and Device identity are immutable. Configuration resources soft-disable in V1; transcript purge is the only destructive API operation and requires explicit scope/confirmation.
- Resolve database-backed admission as Device → Agent → Template/Server Default Profile → loaded provider bindings → External MCP bindings. With no assignments, use server defaults. With assignments, default configuration invalidity is coarse 503; valid non-default candidates form the immutable Template Switch Catalog. Resolve exactly one Effective Session Profile per WebSocket and retain no live DB row/repository in SessionActor.
- Auto-registration is explicit dev/migration behavior only. It requires an enabled configured Agent, inserts an enabled Device with bounded safe metadata, uses unique device identity plus transaction/upsert-safe behavior, and never persists auth headers, tokens or raw client hello.
- Split Database Desired Configuration from immutable Loaded Runtime. Provider mutation persists desired state and communicates requires-restart/runtime status; RuntimeCatalog is never hot-reloaded from switch or admission. Required provider load failure blocks startup; optional provider failure leaves runtime unavailable and excludes only dependent non-default candidates; unbound provider skips secret/runtime load.
- Provider config is canonical serialization of a discriminator-specific typed, non-secret adapter configuration. A shared ProviderConfigValidator runs for both Admin write and startup DB validation: raw size cap, JSON parse, depth/node caps, recursive exact protected-key guard, typed deserialize with unknown fields denied, adapter validation, canonical serialize. Provider config has no arbitrary Value, headers/options bag or credential field. Required invalid config fails startup; optional/unbound invalid config is unavailable without blocking boot.
- Treat SecretRef as opaque printable ASCII with redacted Debug and no normalization. SecretResolver is deployment-owned; V1 EnvSecretResolver alone interprets environment-variable syntax. SecretValue is redacted, non-display/non-clone and zeroized on drop. Provider secret resolution occurs at startup/runtime build; External MCP secret resolution occurs once at admission and belongs to ExternalMcpClient, not SessionActor. No Admin endpoint resolves/tests/reads secrets.
- External MCP is distinct from Device MCP. Agent binding publishes all successfully discovered and validated tools in V1. Fresh admission discovery is bounded and optional failure is fail-soft. Tool snapshots use typed ToolOrigin and deterministic namespaces, validated tool-name/schema caps, and no stale discovery catalog.
- External MCP outbound policy enforces hostname/CIDR allowlists after DNS resolution, normal HTTPS certificate-chain and hostname validation, no insecure TLS switch, explicit HTTP LAN exception only, no redirects, no URL userinfo/query/fragment and typed auth injection only. Per-server global call concurrency is bounded; Session Tool Catalog remains immutable even after runtime tool failure.
- Run all Device and External ToolCall through one sequential Tool-round Executor. Validate whole-round cap before call one, round cap before new round and total turn execution budget across calls. A completed call has one matching terminal ToolResult; cancellation, cap or exhausted budget is terminal without retry or late continuation. Tool results are never persisted in optional transcript archive.
- Optional history is disabled by default. When enabled, enqueue final accepted user text and assistant text only after normal writer closure through bounded best-effort HistoryWriter. Queue/writer/database failure drops one record with bounded telemetry, never changes RAM Dialogue History, prompt composition, tool continuation or turn outcome. Retention runs outside realtime path even when capture later disables.
- SQLite busy/locked is distinct from SQLx pool timeout and storage failure. Busy timeout is the only V1 lock-wait mechanism; no application retry, transaction retry or SessionActor database wait. Admin and DB admission map failures to coarse 503; HistoryWriter drops; maintenance aborts one run and waits for its next schedule. Transactions contain only short atomic database work, never external awaits.
- `/health` remains liveness. `/ready` means startup/schema completion, required RuntimeCatalog availability, operational admission resolver and reachable DB whenever active features require it. It does not perform full admission, Device lookup, External MCP discovery or provider/secret reload. Optional MCP failure does not make it non-ready; required DB admission degradation does.
- Shutdown stops new listener/admission/tool calls before draining open sessions for configured grace time. It then controlled-closes remaining sessions. HistoryWriter may flush only within the same deadline; no shutdown extension, retry loop or new external side effect is allowed.
- Admin audit has retention and operator-side forensic purpose only; V1 intentionally exposes no Admin audit read endpoint.
- Roll out in gates: SQLite foundation; typed repositories/query infrastructure; Admin CRUD; Device admission; Template-to-RuntimeCatalog resolution; template switching; External MCP; optional text history. Each gate activates behavior only by its explicit configuration and must preserve existing Voice semantics before that gate.

## Testing Decisions

- The primary regression seam is the existing application/router state boundary exercised through Admin HTTP endpoints and the WebSocket admission/session boundary. Tests must assert client-visible status, profile/runtime selection, tool continuation and Voice lifecycle rather than SQLx call ordering or private field layout.
- Use existing deterministic provider, Device MCP, WebSocket protocol, SessionActor, writer terminal outcome and reference-client test patterns. Real models, remote providers, external MCP servers and hardware remain separate optional gates.
- Database foundation tests cover fresh DB migration, foreign-key/index creation, migration idempotency, old supported schema forward migration, newer-than-binary schema failure, migration failure before listener and migration-on-start disabled against non-current schema.
- Repository/service tests cover typed CRUD, bindings, one enabled default Template, revision concurrency, short transactional mutation/audit behavior, Device uniqueness/auto-registration race safety, pagination/filter/sort allowlists and resource bounds.
- Admin API tests cover disabled route 404, strict authentication 401, request ID, content encoding/type/body size/malformed JSON responses, no secret reference readback, no audit read route, typed PATCH field intent, query limits and coarse database error mapping.
- Provider tests cover desired-versus-loaded runtime reporting, required/optional/unbound load plan, shared ProviderConfigValidator behavior for Admin and startup, protected-key vectors at all nesting levels, allowed token-related non-secret keys, typed unknown-field rejection, secret resolver error redaction and rotation visibility only at restart/new session.
- Admission tests cover database-disabled legacy behavior, enabled unknown/disabled Device 403 before upgrade, DB/profile error 503, auto-registration contract, no-assignment server defaults, invalid assigned default fail-closed, non-default exclusion, immutable Effective Session Profile and Template Switch Catalog behavior across admin mutation.
- External MCP tests cover network/TLS/redirect policy, typed auth redaction, discovery/schema/name/result caps, fail-soft optional discovery, per-server limiter behavior, immutable Session Tool Catalog, typed tool failure, no retry and sequential mixed Device/External tool ordering under turn caps/cancellation.
- History tests cover disabled capture, final-user-only and normal-writer assistant archive conditions, queue/database drop behavior, retention/purge scope, cleanup contention behavior and independence from Dialogue History.
- Lifecycle tests cover `/health` versus `/ready`, optional MCP non-readiness exclusion, DB-admission readiness degradation, no full discovery in readiness, shutdown rejecting new admissions/tool calls, bounded drain and best-effort history writer completion.

## Out of Scope

- PostgreSQL, Redis, clustered/active-active database ownership, NFS/SMB SQLite and migration leader election.
- Application-driven schema downgrade, automatic database backup/restore/rotation, or Admin audit read API.
- Provider hot reload, model instantiation from a DB row during template switch, realtime session revocation as a side effect of config mutation, and per-session config live reload.
- Persistent audio, ASR partials, LLM deltas, prompts, tool arguments/results, or authoritative SQLite conversational state.
- Per-tool Agent MCP allowlists, required External MCP behavior, circuit breakers, stale tool cache reuse, generic external HTTP escape hatches and insecure TLS mode.
- Admin CORS/cookie authentication, in-process brute-force limiter, compressed Admin request bodies and arbitrary/raw SQL API.
- Secret manager/Vault backend implementations beyond the EnvSecretResolver seam, secret value API access, secret preflight/test APIs and secret value pattern scanning.

## Further Notes

- This spec preserves the current RAM Dialogue History and WriterEvent TurnClosed normal-delivery boundary. SQLite history is an optional archive, never conversational authority.
- Existing ADRs for transcript privacy, device admission, desired-versus-loaded runtime, immutable session profiles, External MCP protocol engine/network policy, secret resolution, provider config validation, migrations, contention, Admin transport and runtime lifecycle are authoritative for implementation details.
- The rollout gates are intentionally independent: schema/database availability alone does not activate any new Voice semantic.
