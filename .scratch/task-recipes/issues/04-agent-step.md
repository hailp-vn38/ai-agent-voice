# 04: Agent Step có kiểm schema và playlist tự chọn domain/topic

**What to build:** Recipe chứa được bước cần phán đoán do một Sub-Agent làm trong phạm vi hẹp. Với recipe playlist, khi người dùng chọn domain "agent tự chọn", một Agent Step chọn `<domain>/<topic>` trước khi lưu nguồn. Output sai schema được sửa một lần, sai tiếp thì Task dừng.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Agent Step là lời gọi `subagent` do `next` phát ra, với Delegated Task Prompt dựng từ template của recipe. Registry từ chối Sub-Agent không tồn tại.
- [ ] Sub-Agent kết thúc bằng một tool hoàn tất có schema output. `next` đọc output có kiểu từ tool result, không parse text tự do.
- [ ] Output sai schema thì `next` resume session con đúng một lần, kèm chẩn đoán có kiểu. Sai tiếp thì Task kết thúc với `RECIPE_STEP_FAILED` và bước lỗi, không fallback sang agent loop.
- [ ] Tham số domain của recipe playlist có giá trị "agent tự chọn" và danh sách domain Wiki động qua `x-options-source`.
- [ ] Có test TaskManager với ScriptedGateway cho ba trường hợp: output đúng, sửa một lần thành công, và sai hai lần làm Task dừng. Có test hàm thuần `next` cho cùng ba trường hợp.
