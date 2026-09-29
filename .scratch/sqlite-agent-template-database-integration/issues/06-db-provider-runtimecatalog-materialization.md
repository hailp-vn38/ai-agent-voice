# 06: DB Provider → RuntimeCatalog materialization

**What to build:** Khi database được bật, server có thể materialize Provider Instance từ Database Desired Configuration thành immutable Loaded Runtime trong RuntimeCatalog trước listener bind, có loaded revision/runtime status chính xác và tuyệt đối không lazy-load khi WebSocket admission.

**Blocked by:** 03: Template và Provider desired configuration.

**Status:** resolved

- [x] Một application-owned bridge đọc validated Provider rows, dùng ProviderConfigValidator và SecretResolver, rồi build các factory/worker runtime hiện có vào RuntimeCatalog với metadata provider ID và loaded desired revision.
- [x] Bridge giữ Database Desired Configuration tách Loaded Runtime: runtime status là not-loaded/unavailable/loaded; runtime-match-desired so loaded revision với DB revision; Admin mutation không mutate catalog process hiện hành và trả requires-restart đúng contract.
- [x] Provider runtime construction tái dùng compile-time Provider Registry, existing model preparation/worker ownership và typed adapter config; không tạo parallel provider factory contract, không đọc environment trực tiếp trong adapter và không log config/secret/reference.
- [x] `Database::connect()` chỉ chịu trách nhiệm open/migrate/schema compatibility; không được fail startup bằng một blanket validation của mọi Provider enabled trước khi Provider Load Plan tồn tại. Startup invoke bridge trước listener. Bridge validates every enabled row and returns structured coarse config/secret/runtime outcomes so caller Provider Load Plan can classify required as fail-fast and optional/unbound as unavailable; bridge không tự quyết fallback hoặc lazy load.
- [x] Unit/integration tests prove DB row can materialize each required provider kind into RuntimeCatalog, loaded revision/status reporting, invalid/secret failure classification, no RuntimeCatalog mutation after Admin desired update and no model/runtime build triggered by WebSocket admission.

## Comments

- Implemented startup-only DB provider materialization, immutable runtime-revision snapshot, and Admin runtime/restart reporting. Ticket 07 remains responsible for deriving required/optional/unbound Provider Load Plan policy.
