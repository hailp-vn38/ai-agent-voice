# ADR 0083 — Admin-managed Resource Credentials

## Status

Accepted

Provider and External MCP credentials are managed through the authenticated Admin API and persisted in SQLite so operators can configure them from Admin Web without provisioning an environment variable for every resource. This replaces the deployment-only credential ownership in ADR 0061 and the deployment-owned credential document; typed Provider configuration remains credential-free under ADR 0062.

Credential values are write-only API inputs, separate from `config_json` and arbitrary MCP headers. Resource reads expose masked credential metadata, never plaintext. An optional write-only `api_key` accompanies the existing Provider/MCP POST or PATCH payload; there is no separate key endpoint. Omitted fields preserve an existing value; replacement follows resource revision checks. Switching MCP authentication to `none` removes its stored credential. Error responses, audit records, tool fingerprints and browser persistence must not contain credential values.

Persisted credentials remain usable after process restart. Provider runtime preparation and MCP session admission take credential snapshots; updating a credential does not replace the value in an already active runtime or session. MCP credential changes invalidate existing tool approvals. New forward-only migrations add storage without modifying migration 0099 or restoring references it already removed.

The operational trade-off is that the SQLite database and its backups now contain credential material and require protection. Values are encrypted with AES-256-GCM using a fresh cryptographically random 12-byte nonce for every write. AAD binds the immutable resource identity and credential UUID. Each encrypted record stores ciphertext with its authentication tag, nonce, key version and a masked hint. Server encryption keys are supplied separately from SQLite; Base64 encodes binary fields and does not provide encryption.

## Module and verification seams

The credential module owns validation, nonce generation, AES-GCM encryption/decryption, masking and versioned server-key selection behind the existing Secret Resolver interface. Database reads capture encrypted credential snapshots before runtime construction; the synchronous runtime resolver performs no SQL access. The agreed test seams are the Admin HTTP interface, Provider runtime after restart, the observable MCP request, and the web forms. Tests advance one failing behavior at a time.

Existing deployment-environment credentials remain a fallback for resources that have no stored credential. A submitted key takes precedence; migration does not copy environment values into SQLite. Provider typed configuration and arbitrary-header rejection remain unchanged. Production credential submission requires HTTPS termination. Missing encryption keys, unknown versions and authentication-tag failures fail closed with bounded errors. Retain old version keys until their records have been replaced; key loss makes those records unusable.

References: [OWASP Secrets Management](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html).
