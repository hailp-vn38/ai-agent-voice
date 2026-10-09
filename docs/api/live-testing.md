# Provider and MCP manual testing

All routes use `/api/admin`, require the Admin bearer token and return the existing bounded error envelope `{error:{code,request_id}}`. Draft tests never save configuration, encrypt credentials, approve tools or change Voice Session catalogs. Results are point-in-time observations held only by the open browser panel.

| Route (POST) | Request | Result |
|---|---|---|
| `/provider-tests/llm` | `{provider:{type,adapter,config_json,api_key?|saved_credential?},input:{text}}` | `test_source:"draft"`, `result.text`, `metrics.elapsed_ms` |
| `/provider-tests/tts` | Same provider; `input:{text,voice?,language?}` | `audio/wav`; `X-Provider-Test-Elapsed-Ms`, `X-Provider-Test-Source:draft` |
| `/provider-tests/asr` | Multipart `provider` JSON and `audio` WAV | Transcript/language; elapsed, duration, RTF |
| `/mcp-tests/connection` | `{server:{key,url,auth,api_key?|saved_credential?,connect_timeout_ms?,request_timeout_ms?}}` | Initialize handshake observation; never `tools/list` |
| `/mcp-tests/discover` | Same server | Complete validated paginated catalog, dropped-name count; never `tools/call` |
| `/mcp-servers/{key}/test/connection` | `{}` | Saved server handshake observation |
| `/mcp-servers/{key}/test/discover` | `{}` | Saved server complete tool catalog |

`api_key` is write-only and mutually exclusive with `saved_credential:{key,expected_revision}`. An unsaved edit can reuse the saved credential only at the exact revision and compatible provider type/adapter or MCP key/auth/header. Conflicts fail before outbound traffic. Provider credential submission from the browser requires HTTPS or loopback development. MCP credential submission allows HTTP for create/edit and draft connection/discovery, including non-loopback Admin endpoints. MCP HTTP/LAN destinations remain permitted under ADR 0056; the UI warns when authentication travels over HTTP.

Provider draft execution requires a configured Provider Runtime Manager and an explicit deployment memory estimate for the adapter. Draft identities have separate logical quotas and use the same global workload, physical resource sharing, memory, admission, terminal acknowledgement and quarantine policies as saved diagnostics. Without that manager/budget, testing fails with a controlled unavailable/configuration error. Existing saved Provider routes and VAD diagnostics are unchanged.

ASR accepts mono PCM16 WAV at an advertised adapter sample rate (currently 16 kHz), up to 30 seconds and 5 MiB. Multipart permits exactly two distinct named parts, with an independently bounded JSON part and total request size. The browser reuses the microphone recorder to downmix/resample and encode WAV, and closes tracks on cancellation/unmount, including late permission grants. LLM input is bounded to 8 KiB, TTS to 4 KiB; runtime output limits remain unchanged.

`api.mcp_tests.max_concurrency` defaults to 2 (range 1–8), `timeout_ms` to 30000 (range 1000–120000). The effective network attempt deadline never exceeds either existing external MCP resolution budget. A bounded attempt owns its permit until transport close even when its HTTP waiter leaves. Shutdown admission uses the application's existing gate. Saved probes can test disabled MCP servers without enabling them. Discovery returns no partial catalog if any page or schema fails.

Provider Create has Test & Review; saved Detail executes real inference; unsaved Edit uses full canonical `config_json` and adapter schema. Runtime status is separate from last test result. MCP forms support draft tests before saving, and `/mcp/:key` exposes manual connection/discovery plus searchable, escaped tool schemas. Discovery remains separate from Agent approval. There is no automatic probe on GET/page mount.

See the [Postman collection](00-all-apis.postman_collection.json), [credential ADR](../adr/0083-admin-managed-resource-credentials.md), [network ADR](../adr/0056-external-mcp-outbound-network-policy.md), and [RMCP ADR](../adr/0069-rmcp-external-mcp-protocol-engine.md).
