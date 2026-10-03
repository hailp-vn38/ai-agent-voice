# ADR 0061 — Deployment-owned Secret Resolution

## Status

Accepted

SQLite chỉ giữ `SecretRef` opaque, không biết secret backend và không lưu secret value. `SecretRef` là printable ASCII `1..=256` bytes (`0x20..=0x7E`), không empty/whitespace-only/leading-trailing space, không trim/normalize và redacts Debug; domain không áp resolver-specific syntax. Bootstrap inject `Arc<dyn SecretResolver>` vào AppState; Provider loader và External MCP client chỉ phụ thuộc abstraction này. V1 `EnvSecretResolver` diễn giải reference là environment-variable name và trả `secret_invalid` khi cú pháp không đúng, còn domain và Admin API không được làm vậy. `SecretValue` là wrapper zeroize-on-drop, redact `Debug`, không `Display`/`Clone` và chỉ expose value tại điểm dựng provider/request.

Required provider resolve/load failure fail startup trước listener và Provider Runtime giữ credential snapshot đến restart; optional non-default provider failure giữ runtime unavailable và exclude candidate; unbound provider không resolve ở startup. External MCP chỉ resolve một lần ngay trước admission `initialize/tools/list`; `ExternalMcpClient`, không SessionActor, sở hữu credential snapshot đến disconnect. Required MCP trong tương lai sẽ trả `503`. Rotate secret chỉ ảnh hưởng RuntimeCatalog sau restart hoặc WS admission mới; không tools/call nào re-resolve/refresh credential. Remote `401`/`403` là typed `external_tool_auth_failed`, không retry, refresh hay mutate catalog. Không có Admin API resolve/test/read secret; GET chỉ trả `has_secret_ref`, log/metrics chỉ dùng bounded reason (`secret_missing`, `secret_resolver_unavailable`, `secret_invalid`) và resource key, không reference/value/header.
