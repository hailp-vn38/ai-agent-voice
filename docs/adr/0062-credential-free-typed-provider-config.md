# ADR 0062 — Credential-free Typed Provider Config

## Status

Accepted

`providers.config_json` chỉ là canonical serialization của discriminator-specific typed non-secret adapter config. Một `ProviderConfigValidator` chung cho Admin mutation và startup loader chạy theo thứ tự raw body bound → UTF-8 `<=64 KiB` → JSON parse → depth `<=16` và aggregate object-key/array-item nodes `<=512` → recursive scan key → typed deserialize. Guard canonicalize key chỉ để exact-match denylist credential (`apikey`, `token`, `accesstoken`, `refreshtoken`, `bearertoken`, `password`, `passwd`, `secret`, `clientsecret`, `authorization`, `proxyauthorization`, `credential`, `credentials`, `privatekey`, `secretref`), rồi deserialize typed config với `deny_unknown_fields` và adapter validation. Không substring-match hoặc heuristic scan value; `max_tokens`, `tokenizer`, `token_budget` vẫn hợp lệ.

Không adapter config nào có plaintext credential, arbitrary `Value`, arbitrary header/options map hoặc escape hatch. Theo ADR 0083, credential được nhập riêng qua Admin API và lưu mã hóa; Secret Resolver chỉ giải mã snapshot tại runtime. `config_json` vẫn không chứa credential. Shape/JSON/protected-field/typed failure trả client `provider_config_invalid`, internal telemetry chỉ bounded reason, không echo config/path/key/value. Generic shape cap là resource boundary, không thay typed adapter semantic limits. Required persisted row invalid fail startup; optional invalid unavailable/exclude dependent candidate, còn unbound invalid unavailable nhưng không block boot. Startup revalidates để không tin Admin API là source duy nhất của DB content.
