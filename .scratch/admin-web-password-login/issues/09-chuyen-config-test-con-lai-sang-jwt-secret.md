# 09: Chuyển các config test còn lại sang `jwt_secret` (lô 3)

Status: resolved

**What to build:** Bước *migrate* cuối. Nhiều test không gọi Admin API nhưng dựng config có `api.enabled` và `admin_token` (device admission, enrollment, CORS, database bootstrap, external MCP admission, lifecycle, provider, session profile, speaker, speech output, tool round, config provider bindings, wiki-mcp). Sau ticket 10 các config đó sẽ làm khởi động thất bại, nên ở đây thêm `jwt_secret` vào tất cả, và test nào có gửi Bearer thì chuyển sang helper.

Sau ticket này không còn test nào **phụ thuộc** vào `admin_token`; khoá vẫn nằm trong config để CI xanh cho đến ticket 10.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Mọi config test có `api.enabled = true` đều có `jwt_secret` hợp lệ
- [ ] Không test nào ngoài phạm vi ticket 07 và 08 còn gửi giá trị `admin_token` làm Bearer
- [ ] Tìm trong toàn bộ test: mọi chỗ `admin_token` còn lại chỉ là dòng config, không phải credential được gửi đi
- [ ] `scripts/agent-test.sh full` qua
