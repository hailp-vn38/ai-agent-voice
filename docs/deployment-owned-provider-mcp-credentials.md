# Deployment-owned credentials for Provider and External MCP

## Decision and security boundary

Provider API keys and External MCP bearer/header token values **and their lookup references** are no longer accepted from Vue Admin or persisted to SQLite. Admin remains responsible for non-secret Provider configuration and MCP endpoint/auth mode only. Existing provider identities and Agent/Template/MCP bindings remain unchanged.

Before this change, SQLite contained `secret_ref` (an environment variable reference), **not plaintext credential values**. MCP's arbitrary `headers_json` was additionally an unsafe ingress for secrets. The API now rejects `secret_ref` and writable `headers` as unknown fields, even if the values appear innocuous. Provider `config_json` continues to be protected by the typed credential denylist.

| Resource | Admin-owned fields | Deployment-owned credential |
| --- | --- | --- |
| Provider `adapter=openai` | provider name, model, timeout, base_url, enabled, bindings | `VOICE_PROVIDER_<PROVIDER_KEY>_API_KEY` |
| Provider `adapter=chillaudio_ws` | non-secret TTS configuration | `VOICE_PROVIDER_<PROVIDER_KEY>_API_KEY` (injected as its runtime token) |
| External MCP `auth.type=bearer` | URL, timeouts, enabled, auth type | `VOICE_MCP_<SERVER_KEY>_TOKEN` |
| External MCP `auth.type=header` | URL, timeouts, enabled, header_name, auth type | `VOICE_MCP_<SERVER_KEY>_TOKEN` |
| Local provider or MCP `auth.type=none` | normal metadata | none |

Keys are the **immutable resource keys** returned by Admin API, uppercased. A UUID-derived Provider key is only known *after* creation. Example: `key=llm_abc123` maps to `VOICE_PROVIDER_LLM_ABC123_API_KEY`; `key=weather` maps to `VOICE_MCP_WEATHER_TOKEN`. Place these variables in the **server process environment**, not the browser, build-time Vue environment, API payload, DB, logs or version control. Web GET resources expose only `credential_env` (a name, not value); secret readiness is represented indirectly by runtime preparation / MCP admission diagnostics.

Existing `SecretResolver`/`SecretValue` remain the boundary: Provider materialization resolves once when loading; External MCP resolves once at session admission, and active sessions keep their previous immutable snapshot. Changing environment variables requires a process restart or equivalent deployment reload; reopening a session alone does not update the process environment.

## API and UI contract

- `POST/PATCH /api/admin/providers`: no `secret_ref`; `GET`/list include read-only `credential_env: string | null`.
- `POST/PATCH /api/admin/mcp-servers`: `auth` is `{"type":"none"}`, `{"type":"bearer"}` or `{"type":"header","header_name":"x-api-key"}`. No `secret_ref` and no `headers`. `GET`/list include `credential_env: string | null` and legacy read-only empty `headers: {}`.
- Web Provider Create and MCP Edit/Create no longer request credentials or env variable names. Provider Detail and MCP form display the deployment-managed environment variable name.
- Arbitrary static MCP headers **are deliberately no longer configurable**: they previously could contain credentials. Any required non-auth protocol header needs a separately reviewed, typed, secret-free contract (not arbitrary JSON).

## Migration and deployment sequence

1. Back up SQLite and current secret configuration securely, restrict backup access. Enumerate Provider and MCP rows in advance and map each old `secret_ref` to the new deterministic environment name. **Do not print, export, or commit secret values** as part of migration.
2. Provision the new names in the server deployment environment or secret manager before switching binaries. For fresh Providers, create the Provider, obtain its generated key, provision its environment variable, and trigger runtime prepare/restart as applicable.
3. Deploy backend and web together; let migration `0099_deployment_owned_credentials.sql` clear `mcp_servers.headers_json` and remove both `secret_ref` columns. Never attempt to reuse old Admin request payloads.
4. Restart and check Provider runtime state / diagnostics and MCP admission. Re-review previously approved External MCP tools if source fingerprints change.
5. Once proven stable, scrub exposed credentials (if any), retire old secret environment names, and apply normal credential rotation policy. **Dropping columns does not securely erase old bytes in WAL files, old SQLite page copies, snapshots or backups**; manage those copies according to retention and SQLite maintenance procedures.

## Failure behavior / non-goals

Missing Provider credentials follow existing startup/load policy (required binding fails startup; optional unavailable). Missing MCP credentials exclude only that MCP server during admission with bounded, redacted errors. Admin cannot read/test/rotate secret values. No user-facing API accepts a raw API key or a user-chosen environment variable name.
