# 04: External MCP desired configuration

**What to build:** Admin có thể cấu hình External MCP Server và Agent MCP binding an toàn, có typed auth/network policy và optimistic concurrency, nhưng chưa làm thay đổi Voice Session runtime.

**Blocked by:** 02: Admin control-plane shell và Agent/Device CRUD.

**Status:** resolved

- [x] MCP Server và Agent MCP binding CRUD are authenticated, revisioned, audited and soft-disable-only; V1 binding publishes all validated server tools once runtime integration exists and `required` remains unsupported/false.
- [x] URL, static-header JSON, timeout, typed none/bearer/header auth, SecretRef redaction and protected header rules are validated without plaintext credential storage.
- [x] Outbound policy data enforces hostname/CIDR allowlist model, HTTPS certificate/hostname verification, explicit LAN HTTP exception only, no redirects and no URL userinfo/query/fragment.
- [x] GET is redacted and never reveals secret reference/value; Admin mutation does not resolve or test credentials.
- [x] HTTP/repository tests demonstrate malformed config rejection, protected header/auth constraints, network-policy validation, revision conflict and safe audit output.

## Answer

Đã bổ sung Admin HTTP CRUD cho `mcp_servers`, Agent MCP binding revisioned và External MCP network policy desired configuration. V1 chỉ persist desired state, không resolve/test secret hoặc khởi tạo External MCP runtime từ Admin request. `required=true` bị reject; GET chỉ trả auth đã redact. Runtime có seam DNS resolution riêng, kiểm tra mọi địa chỉ đích trước outbound dial.

Validation: `cargo fmt --check`, test ticket `external_mcp_configuration_is_redacted_validated_and_revisioned` và Admin API suite pass. `cargo test --workspace` có ba failure timing/backpressure có sẵn tại `speechoutput_tracer`, ngoài phạm vi ticket 04.
