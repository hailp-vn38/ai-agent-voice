# 12: Quan sát và approve Device tool contracts

**What to build:** Admin inspect contract do Device đã admit quảng bá, approve đúng observation và dùng tool trên Agent mà không kế thừa quyền của Device khác.

**Blocked by:** 11: Review và giới hạn External MCP tools.

**Status:** ready-for-agent

- [ ] Bounded completed observations lưu SQLite từ existing admitted MCP discovery; UI có timestamp và cảnh báo observed≠online/behavior guarantee, không schema Admin nhập.
- [ ] API identity device_id + original name, internal Device incarnation FK, no wildcard/display name/Client ID; delete/recreate không kế thừa approvals.
- [ ] Approve CAS incarnation + observed revision + fingerprint + allowlist revision trong transaction; observation đổi trả conflict.
- [ ] Contract drift hoặc conflicting valid observations block entry/close affected WS, no latest-wins/no incomplete merge. Only approved tools advertised + guarded dispatch.
- [ ] Public tests Device/External same-name collision, two same-Device WS observations, concurrent review, stale schema và restart metadata; UI/API docs tích hợp.
