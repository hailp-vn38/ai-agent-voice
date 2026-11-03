# 04: Agent Step có kiểm schema và playlist tự chọn domain/topic

**What to build:** Recipe chứa được bước cần phán đoán do một Sub-Agent làm trong phạm vi hẹp. Với recipe playlist, khi người dùng chọn domain "agent tự chọn", một Agent Step chọn `<domain>/<topic>` trước khi lưu nguồn. Output sai schema được sửa một lần, sai tiếp thì Task dừng.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Agent Step nêu Sub-Agent trong catalog, template Delegated Task Prompt, tập tool, skill và schema output. Registry từ chối Sub-Agent không tồn tại.
- [ ] Agent Step chạy trong Agent Session con. Bước lặp theo lô dùng một session con mới cho mỗi lô.
- [ ] Output đúng schema thì được truyền sang bước sau. Sai schema thì có đúng một lượt sửa kèm chẩn đoán có kiểu. Sai tiếp thì Task dừng kèm bước lỗi và `stop_reason`, không fallback sang agent loop.
- [ ] Tham số domain của recipe playlist có giá trị "agent tự chọn" và danh sách domain Wiki động qua `x-options-source`.
- [ ] Test TaskManager với ScriptedGateway phủ ba trường hợp: output đúng, sửa một lần thành công, và sai hai lần làm Task dừng.
