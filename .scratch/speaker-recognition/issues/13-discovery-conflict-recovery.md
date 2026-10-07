# 13: Giải quyết conflict bằng discovery theo đợt

**What to build:** Admin khởi động đợt discovery recovery cho Device và chỉ review lại khi mọi observation cần thiết hoàn tất, nhất quán.

**Blocked by:** 12: Quan sát và approve Device tool contracts.

**Status:** ready-for-agent

- [ ] Reuse MCP tools/list, scoped incarnation + batch identity/deadline/states; không protocol mới hoặc product wizard.
- [ ] Chỉ complete observations current batch được xét; stale prior-batch bị loại. Conflict/timeout/incomplete tiếp tục blocked, không bỏ failed members để claim consistent.
- [ ] Observations cũ superseded theo retention bounded; successful batch chỉ reviewable, vẫn conditional approve mới, không tự restore allowlist.
- [ ] UI/API diễn đạt batch outcome/time và review state; WS/control vẫn theo admission hiện có.
- [ ] Public multi-WS/HTTP races kiểm late results, mixed batches, disconnect/timeout, consistency, revision conflicts và bounded retention.
