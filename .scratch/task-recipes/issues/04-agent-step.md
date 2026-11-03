# 04: Agent Step có kiểm schema và playlist tự chọn domain/topic

**What to build:** Recipe chứa được bước cần phán đoán do một Sub-Agent làm trong phạm vi hẹp. Với recipe playlist, khi người dùng chọn domain "agent tự chọn", một Agent Step chọn `<domain>/<topic>` trước khi lưu nguồn. Output sai schema được sửa một lần, sai tiếp thì Task dừng.

**Blocked by:** 02

**Status:** resolved

- [x] Agent Step là lời gọi `subagent` do `next` phát ra, với Delegated Task Prompt dựng từ template của recipe. Registry từ chối Sub-Agent không tồn tại.
- [x] Sub-Agent kết thúc bằng một tool hoàn tất có schema output. `next` đọc output có kiểu từ tool result, không parse text tự do.
- [x] Output sai schema thì `next` resume session con đúng một lần, kèm chẩn đoán có kiểu. Sai tiếp thì Task kết thúc với `RECIPE_STEP_FAILED` và bước lỗi, không fallback sang agent loop.
- [x] Tham số domain của recipe playlist có giá trị "agent tự chọn" và danh sách domain Wiki động qua `x-options-source`.
- [x] Có test TaskManager với ScriptedGateway cho ba trường hợp: output đúng, sửa một lần thành công, và sai hai lần làm Task dừng. Có test hàm thuần `next` cho cùng ba trường hợp.

## Answer

Đã merge vào integration branch `feat/task-recipes` qua server `97978dd` và flow `c5ceb15`. Agent Step dùng Sub-Agent catalog với prompt template; completion HostTool có JSON Schema, output được kiểm strict vào `TopicSelection`, một lần resume sửa lỗi có kiểu, lỗi lần hai dừng với `RECIPE_STEP_FAILED`. Domain "agent tự chọn" dùng enum Wiki động `x-options-source`. Test pure recipe và TaskManager/Admin HTTP ScriptedGateway cho output đúng, sửa một lần và dừng sau hai lần đều pass.

Giới hạn interface hiện có: Orchestrator chỉ đưa final output của child vào kết quả `subagent` ở parent, không đưa child transcript/structured payload riêng. Vì vậy completion tool trả JSON canonical, Sub-Agent chỉ được phép kết thúc bằng payload JSON đó, và parent deserializes strict JSON (từ chối prose/field ngoài schema); không có parse văn bản tự do.

Kiểm tra rộng: server package suite còn lỗi ngoài ticket ở `tests/vieneu.rs:66` vì thiếu config file tại workspace; flow `scripts/check.sh fast` dừng ở clippy cảnh báo nền `src/tools/pdf.rs:142`, ngoài diff.
