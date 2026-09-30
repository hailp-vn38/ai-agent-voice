# ADR 0059 — External MCP tool schema dùng bounded supported subset

## Status

Accepted

External MCP tool schema là untrusted input: V1 chỉ nhận JSON Schema object subset mà LLM conversion hỗ trợ, với allowlist keyword và depth/node/property/enum bounds. Unsupported reference/composition/recursive shape hoặc invalid relation reject toàn bộ server catalog fail-soft; không passthrough, truncate hay rewrite schema.
