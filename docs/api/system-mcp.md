# System MCP

System MCP exposes bounded, secret-free system and External MCP management information through the same core used by Studio Assistant and `/mcp/system`. Device MCP and External MCP execution remain separate authorities.

## Studio Assistant grants

`[system_mcp] enabled = false` is the deployment ceiling. Enable it to opt in. One revisioned database record controls `system_read`, `mcp_read`, `mcp_discover`, and `mcp_manage_request`. On first use, the two read grants inherit the deployment flag; discovery and proposals default to false. Later deployment changes never overwrite this record.

System → Studio Assistant → System MCP edits these grants through authenticated Admin HTTP. Revoking a grant closes affected Voice Sessions and prevents further calls. New grants require reconnecting to receive a new catalog. Physical Devices never receive these tools.

- `GET /api/admin/system-mcp/grants`: `{enabled, grants, revision}`.
- `PUT /api/admin/system-mcp/grants`: the four grant booleans, with `If-Match: "revision"`, Admin bearer and a trusted browser Origin.

The read tools are `system_get_status`, `system_get_resources`, `mcp_list_servers`, `mcp_get_server`, `mcp_list_tools`, and `mcp_get_agent_bindings`. `mcp_list_tools` returns `observed_revision`, `fingerprint`, and `review_revision`; use the latter as `expected_revision` when requesting a tool review. Remote callers must supply `agent_key` for `mcp_list_tools`; Voice callers default to their admitted Agent.

`mcp_discover_tools` requires `mcp:discover`. It shares the existing External MCP diagnostic concurrency, timeout, network policy and catalog limits. Complete validated catalogs update observations and revoke approvals on drift. Discovery never approves or executes an external tool. Its output contains only the server key, completion flag and observed tool count.

## Change requests

`mcp_request_server_change`, `mcp_request_agent_binding_change`, and `mcp_request_tool_review` require `mcp:manage:request`. They create proposals without changing resources. Spoken agreement and model-generated fields cannot authorize confirmation.

The RAM registry holds at most 256 records and 16 per caller, including terminal records, for five minutes. Restart removes them. It is valid only in the existing single-process SQLite ownership deployment.

- `GET /api/admin/system-mcp/change-requests`: bounded safe proposals and terminal states. Requires Admin bearer and trusted Origin (or same-origin Referer for browser GET).
- `POST /api/admin/system-mcp/change-requests/{id}/confirm`: atomically claims one fixed command before dispatch. Rechecks caller, grants, shutdown, expiry, revisions and review fingerprint under the existing publication guard. The existing DB mutation and success audit commit together.
- `POST /api/admin/system-mcp/change-requests/{id}/reject`: consumes a pending proposal.

Both POST routes require Admin bearer and trusted Origin before touching the registry. Responses contain `{id, command, expires_at, status}`. Status is `pending`, `claimed`, `applied`, `failed`, or `rejected`. Conflicts and DB failures terminalize the request; the same ID never retries. If an HTTP response is lost, read the resource and revision before requesting another change. Registry state and DB commit are deliberately not a distributed transaction.

## Dedicated HTTP JWT authorization

The HTTP route is absent by default. Opt in with a dedicated key supplied through `VOICE_SYSTEM_MCP_JWT_KEY` (at least 32 random bytes encoded as a secret string, distinct from Admin and Voice tokens):

```toml
[system_mcp]
enabled = true

[system_mcp.http]
enabled = true
issuer = "https://auth.example.com"
audience = "https://agent.example.com/mcp/system"
allowed_origins = ["https://admin.example.com"]
```

The server's plaintext listener must bind loopback. Put it behind a trusted HTTPS reverse proxy, preserve the public Host authority, and expose only the intended routes. The resource URL must use HTTPS and the fixed `/mcp/system` path. Empty `allowed_origins` permits clients without Origin and rejects all supplied Origins. SDK Host/Origin validation remains enabled. Do not expose the loopback HTTP listener directly or configure the proxy to rewrite untrusted origins.

Clients send `Authorization: Bearer <JWT>`. This intentionally uses pre-issued dedicated tokens; it does not provide OAuth discovery, token issuance or interactive login. Configure a client that supports an explicit bearer token. The JWT header must be `{ "alg": "HS256", "typ": "JWT" }`. The signature uses the UTF-8 bytes of the environment key. Required claims:

```json
{
  "iss": "https://auth.example.com",
  "aud": "https://agent.example.com/mcp/system",
  "sub": "client-identity",
  "jti": "unique-token-id",
  "iat": 1791734400,
  "exp": 1791734700,
  "scope": "system:read mcp:read"
}
```

`iat` and `exp` are integer Unix seconds; lifetime is at most one hour. Tokens must be unexpired, issued in the past hour, and not issued in the future. Optional `nbf` is enforced. `sub` and `jti` must be nonempty printable ASCII identities of at most 128 bytes. Audience and issuer are exact strings, and audience is a single string. Supported scopes are `system:read`, `mcp:read`, `mcp:discover`, and `mcp:manage:request`; at least one is required. JWT grants are independent of the Studio Assistant grant record, under the same deployment ceiling. Request arguments cannot supply authority.

`POST /api/admin/system-mcp/token-revocations` accepts `{jti, expires_at}` with Admin bearer and trusted Origin. Supply the token expiry, within the next hour. Tombstones are retained at least an hour from revocation regardless of supplied expiry, so cleanup cannot reactivate a still-valid token. The revocation table is bounded to 4096 live tombstones. Every HTTP request and every tool dispatch rechecks expiry/revocation; proposal confirmation also rechecks them. Rotate the key and restart to invalidate all tokens. Never reuse a `jti` for a new token.

The HTTP adapter uses pinned `rmcp = 3.5.0` server and Streamable HTTP server features. The SDK owns protocol negotiation and HTTP JSON-RPC framing. Transport is stateless with JSON responses, 16 KiB request bodies, 16 simultaneous HTTP requests and a 35-second handler bound; tool calls have a 30-second deadline and existing result limits. Admin tokens, Voice tokens and WS tickets are not accepted or forwarded. Errors expose safe fixed codes; tool failures use MCP `isError`.

Tests exercise the Rust SDK client and an independently constructed HTTP reference client, protocol negotiation, tools listing/calling, output parity with the Voice core, route absence, bad JWTs, Host/Origin denial, revocation, proposals, discovery drift and timeout. Production client/auth-service integration and HTTPS proxy deployment remain environment-specific.

Feature names are taken from the [pinned SDK manifest](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.5.0/crates/rmcp/Cargo.toml).
