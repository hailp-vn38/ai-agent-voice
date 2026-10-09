# Encrypted Provider and MCP credentials

Provider and MCP create/update requests accept an optional top-level `api_key`. Use the existing `/api/admin/providers` and `/api/admin/mcp-servers` endpoints, authenticated with the Admin bearer token. A missing field preserves the stored credential. An empty or malformed key is rejected. Provider keys belong to remote adapters (`openai`, `chillaudio_ws`); MCP keys belong to bearer/header authentication. Selecting MCP auth `none` removes its stored credential.

Each resource owns an encrypted JSON record outside `config_json`. Its fields are `id`, `encrypted_value` (ciphertext plus AES-GCM authentication tag), `nonce`, `key_version`, and `key_hint`. Base64 is only the binary encoding. GET/list and mutation responses expose `credential: { id, masked_key, key_version, status }` or `null`, never plaintext, nonce or ciphertext. Keys of eight characters or fewer receive a fully masked hint.

Example Provider create body (send over HTTPS, with the Admin bearer token):

```json
{"name":"OpenAI","type":"llm","adapter":"openai","config_json":{"base_url":"https://api.openai.com/v1","model":"your-model"},"api_key":"<new-key>"}
```

Example MCP create body:

```json
{"key":"weather","name":"Weather","url":"https://mcp.example.com/mcp","auth":{"type":"bearer"},"api_key":"<new-token>"}
```

PATCH uses the same optional field and the existing quoted `If-Match` revision. There is no independent key CRUD API. Never save real keys in Postman collections or shared request files.

## Server key provisioning

Inject a cryptographically random 32-byte server key through the deployment secret manager as `VOICE_CREDENTIAL_KEY_1`, Base64 encoded. Set `VOICE_CREDENTIAL_KEY_VERSION=1` (the default). The server key must be stored separately from SQLite and its backups. Encryption uses AES-256-GCM with a fresh random 12-byte nonce per write; AAD binds the Admin-owned resource identity and credential UUID.

For a new version, provision `VOICE_CREDENTIAL_KEY_2` and select version `2`. Retain version `1` while stored records reference it; replacement writes use the current version. Missing keys, unknown versions, wrong keys and invalid authentication tags fail closed. There is no automatic bulk re-encryption or plaintext-read endpoint. Loss of a server key makes its old encrypted records unusable.

Migration 0100 adds encrypted storage without rewriting migration 0099. Existing environment-derived Provider/MCP keys remain fallback sources until a key is submitted through Admin API; no environment secrets are copied automatically. Active Provider runtimes and MCP sessions retain their existing credential snapshots. Prepare a new runtime/restart or open a new MCP session to use replacements. MCP changes revoke existing tool approvals.

Serve Admin Web through HTTPS before sending credentials. Admin Web blocks key submission from non-loopback HTTP pages or to non-loopback HTTP API destinations (including LAN IPs); loopback HTTP is allowed for local development. The backend may sit behind a trusted TLS terminator; configure HTTPS at the gateway. Protect SQLite/WAL/backups with the usual database permissions even though credentials are encrypted. Do not put credentials in URLs, logs, browser persistence or typed Provider configuration.

See [ADR 0083](adr/0083-admin-managed-resource-credentials.md) and [OWASP Secrets Management](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html).
