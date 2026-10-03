# 03: Template và Provider desired configuration

**What to build:** Admin có thể quản trị Template, Template assignment và Provider desired configuration có typed non-secret config, provider binding và restart semantics rõ ràng mà chưa hot-reload runtime đang chạy.

**Blocked by:** 02: Admin control-plane shell và Agent/Device CRUD.

**Status:** resolved

- [x] Template CRUD, Agent Template assignment và Template Provider binding enforce one enabled default per Agent, immutable keys, provider type completeness, resource bounds, revision/audit transaction và soft-disable semantics.
- [x] Provider CRUD chỉ persist Database Desired Configuration; response phân biệt runtime status, runtime-match-determined state và requires-restart, không instantiate/hot-load provider từ Admin request.
- [x] Provider config uses shared validator for Admin and startup: raw byte/JSON depth/node caps, recursive exact protected-key guard, typed discriminator config with unknown-field rejection, adapter semantic limits and canonical persistence.
- [x] Provider credential path is SecretRef only: opaque bounded reference, deployment-owned resolver, redacted/zeroized SecretValue, no plaintext credential in config/header/options escape hatch and no resolve/test/read secret API.
- [x] Public Admin/startup tests prove assignment/default invariants, protected-key vectors, allowed non-secret token-named keys, secret redaction, typed validation failures, revision/audit behavior and honest requires-restart response.

## Comments

- 2026-09-29: Runtime status/revision reporting and required-versus-optional provider startup classification belong to Ticket 06. Ticket 03 must not return hard-coded runtime fields or fail startup for enabled but unbound/optional desired rows.
