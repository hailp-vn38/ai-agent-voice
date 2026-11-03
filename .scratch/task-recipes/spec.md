# Task Recipe: job cố định do server sở hữu

Status: ready-for-agent

Quyết định kiến trúc: [ADR 0140](../../docs/adr/0140-server-owned-task-recipes.md). Thiết kế giao diện: [task-recipe-ui.md](../../docs/task-recipe-ui.md). Tôn trọng ADR 0127, 0133, 0134 và 0136.

## Problem Statement

Mọi Background Task hiện chạy bằng agent loop. Root Orchestrator tự quyết gọi Sub-Agent nào, Sub-Agent tự quyết gọi tool nào. Với việc lặp lại và đã biết trước các bước, như thu thập một playlist vào `sources/`, cùng một yêu cầu có thể đi một đường khác nhau mỗi lần chạy. Job tốn token cho việc chọn bước, có lúc chạm step cap giữa chừng (`STEP_LIMIT_REACHED`), và người vận hành không đoán trước được kết quả.

Job Skill không sửa được việc này: skill chỉ là lời nhắc, không ép thứ tự bước và không cho Rust biết bước nào đã xong. Trên Admin Web, cách duy nhất để tạo job là gõ mục tiêu tự do, nên người dùng phải tự diễn đạt lại cùng một quy trình mỗi lần.

## Solution

Server có một danh mục **Task Recipe**: các quy trình cố định viết trong code Rust, mỗi recipe có version, schema tham số và danh sách bước. Mỗi bước là *Tool Step* (Rust gọi HostTool trực tiếp, không qua model) hoặc *Agent Step* (một Sub-Agent với prompt, tool và skill cố định, output được kiểm theo schema).

Trên Admin Web, modal "Tạo job" mở bằng danh sách recipe. Người dùng chọn một recipe, điền tham số qua form dựng từ schema, xem trước các bước rồi bấm chạy. Thẻ "Tự do" vẫn giữ nguyên hành vi hiện tại. Recipe cũng chạy được qua API, Schedule `spawn_task`, hoặc khi `daily_root` gọi tool `recipe_start`.

Rust giữ thứ tự bước và lưu con trỏ bước bền vững. Restart và Waiting Retry chạy tiếp từ bước dở. Bước lỗi vĩnh viễn làm Task dừng kèm lý do và kết quả từng phần, không tự chuyển sang agent loop. Task Follow-up trên Task recipe chạy một lượt agent loop tự do, lấy kết quả recipe làm ngữ cảnh.

Recipe đầu tiên: **thu thập playlist hoặc danh sách video vào `sources/`**.

## User Stories

### Xem và chọn recipe

1. Là Admin, tôi muốn thấy danh sách recipe ngay khi bấm "Tạo job", để chọn quy trình có sẵn thay vì gõ lại mục tiêu.
2. Là Admin, tôi muốn thẻ "Tự do" luôn đứng đầu danh sách, để việc tạo job theo cách cũ không bị khó hơn.
3. Là Admin, tôi muốn mỗi thẻ recipe có tên, mô tả ngắn và version, để biết recipe làm gì và đang ở bản nào.
4. Là Admin, tôi muốn tìm recipe theo tên hoặc mô tả, để chọn nhanh khi danh mục dài ra.
5. Là Admin, tôi muốn thấy danh sách bước của recipe trước khi chạy, để biết job sẽ làm gì.
6. Là Admin, tôi muốn mỗi bước ghi rõ là Tool hay Agent và có lặp theo lô hay không, để biết bước nào dùng LLM.
7. Là Admin, tôi muốn chỉ thấy các recipe server đang đăng ký, để không chọn phải recipe không chạy được.

### Nhập tham số

8. Là Admin, tôi muốn form tham số được dựng tự động từ schema của recipe, để recipe mới không cần sửa UI.
9. Là Admin, tôi muốn ô URL cho tham số `format: uri`, để nhập link playlist đúng định dạng.
10. Là Admin, tôi muốn ô số có giới hạn min/max, để không nhập số video vượt mức cho phép.
11. Là Admin, tôi muốn checkbox cho tham số boolean, ví dụ "ingest vào Wiki sau khi thu thập".
12. Là Admin, tôi muốn select cho tham số enum, kể cả danh sách động như domain Wiki, để chọn từ giá trị hợp lệ.
13. Là Admin, tôi muốn nhập danh sách URL mỗi dòng một mục, để thu thập nhiều video lẻ trong một job.
14. Là Admin, tôi muốn đính kèm file khi recipe yêu cầu, với giới hạn loại file theo recipe.
15. Là Admin, tôi muốn trường bắt buộc được kiểm ngay trên form, để không gửi request chắc chắn bị từ chối.
16. Là Admin, tôi muốn lỗi từ server hiện đúng cạnh trường bị sai, để sửa nhanh.
17. Là Admin, tôi muốn lỗi không gắn được vào trường nào hiện ở đầu form, để không bị mất thông báo.
18. Là Admin, tôi muốn giá trị mặc định của tham số được điền sẵn, để chỉ phải nhập phần cần thiết.

### Chạy recipe

19. Là Admin, tôi muốn bấm "Chạy job" để tạo Task chạy theo recipe đã chọn với tham số đã nhập.
20. Là Admin, tôi muốn được chuyển sang trang chi tiết Job sau khi tạo, như luồng tạo job hiện tại.
21. Là Admin, tôi muốn bấm đúp hoặc gửi lại request không tạo hai Task, để tránh chạy trùng.
22. Là Admin, tôi muốn tiêu đề Job tự sinh từ recipe và tham số, để danh sách Job dễ đọc.
23. Là người dùng giọng nói, tôi muốn nói "thu thập playlist này" và để `daily_root` chạy recipe phù hợp, để có kết quả ổn định mà không cần biết recipe là gì.
24. Là người vận hành, tôi muốn `daily_root` chỉ truyền `recipe_id` và tham số, còn Rust validate trước khi tạo Task, để LLM không tự soạn hay sửa quy trình.
25. Là người vận hành, tôi muốn tạo Task recipe qua API, để tự động hóa từ ngoài Admin Web.
26. Là người vận hành, tôi muốn yêu cầu không có recipe vẫn chạy agent loop như cũ, để không vỡ luồng hiện tại.

### Thực thi ổn định

27. Là người vận hành, tôi muốn các bước chạy đúng thứ tự định nghĩa, để mỗi lần chạy cùng recipe có cùng hình dạng.
28. Là người vận hành, tôi muốn Tool Step không gọi LLM, để không tốn token và không đổi kết quả giữa các lần chạy.
29. Là người vận hành, tôi muốn Tool Step đi qua cùng pipeline với tool call của agent (phân loại `ToolEffect`, Tool Operation, ArtifactStore, quyền theo phạm vi Task), để không có đường tắt bỏ qua kiểm tra.
30. Là người vận hành, tôi muốn Agent Step dùng Sub-Agent trong catalog với Delegated Task Prompt dựng từ template của recipe, để phần cần phán đoán vẫn do LLM làm trong phạm vi hẹp.
31. Là người vận hành, tôi muốn output của Agent Step được kiểm theo schema, sai thì sửa một lần bằng chẩn đoán có kiểu, để output thất thường không chạy sang bước sau.
32. Là người vận hành, tôi muốn bước lặp theo lô xử lý các mục trong Collection Ledger, để playlist dài không phụ thuộc vào context của LLM.
33. Là người vận hành, tôi muốn mục lỗi trong lô bị cách ly sau số lần cho phép, để một video lỗi không chặn cả playlist.
34. Là người vận hành, tôi muốn bước đã xong được lưu bền trước khi sang bước sau, để restart không làm lại việc đã xong.
35. Là người vận hành, tôi muốn server restart giữa chừng thì Task chạy tiếp từ bước dở, để job dài không bị mất.
36. Là người vận hành, tôi muốn lỗi tạm thời (timeout, 429, mạng) đưa Task vào Waiting Retry như Task thường, để recipe bền như agent loop.
37. Là người vận hành, tôi muốn recipe dùng chung Job Budget, để recipe không chạy vô hạn.
38. Là người vận hành, tôi muốn tool `Mutating` không rõ kết quả không bị chạy lại mù mà đi qua đường gỡ hiện có, để không ghi trùng.
39. Là người vận hành, tôi muốn hủy Task recipe dừng cả bước đang chạy, giống hủy Task thường.

### Dừng và lỗi

40. Là người vận hành, tôi muốn bước lỗi vĩnh viễn làm Task dừng kèm `stop_reason` và bước lỗi, để biết chính xác hỏng ở đâu.
41. Là người vận hành, tôi muốn kết quả từng phần của các bước đã xong được giữ khi Task dừng, để không mất phần đã làm.
42. Là người vận hành, tôi muốn runtime không tự chuyển sang agent loop khi bước lỗi, để thống kê độ ổn định của recipe không bị lẫn.
43. Là người vận hành, tôi muốn đầu vào không hợp lệ (ví dụ playlist riêng tư) thất bại có lý do rõ ràng, để người dùng chuyển sang job tự do.

### Ghim version

44. Là người vận hành, tôi muốn Task lưu `recipe_id`, `version` và tham số đã validate lúc tạo, để đổi code recipe không làm thay đổi Task đang chạy.
45. Là người vận hành, tôi muốn Task kết thúc với lý do rõ ràng khi version đã ghim không còn trong binary sau khi nâng cấp, thay vì lặng lẽ chạy theo version mới.
46. Là nhà phát triển, tôi muốn tăng version khi đổi bước hoặc schema tham số của recipe, để lịch sử Task truy được recipe nào đã chạy.

### Quan sát

47. Là Admin, tôi muốn danh sách Job có nhãn `recipe · <tên> v<version>`, để phân biệt Job recipe với Job tự do.
48. Là Admin, tôi muốn trang chi tiết Job có thanh tiến độ theo bước recipe, để thấy job đang ở bước nào.
49. Là Admin, tôi muốn bước lặp hiển thị số mục đã xong trên tổng số, để ước lượng thời gian còn lại.
50. Là Admin, tôi muốn thấy bước dừng và `stop_reason` trên thanh tiến độ khi Task dừng.
51. Là Admin, tôi muốn timeline hiện có vẫn hiện Agent Run và tool call của các Agent Step, để soi lỗi chi tiết.
52. Là Admin, tôi muốn lượt follow-up tự do được đánh dấu riêng trong timeline, để không nhầm với lượt recipe.
53. Là người vận hành, tôi muốn chỉ lượt đầu được tính là "chạy theo recipe" khi đo độ ổn định.

### Follow-up

54. Là Admin, tôi muốn gửi follow-up trên Task recipe như Task thường, để hỏi thêm hoặc làm tiếp phần recipe chưa xong.
55. Là Admin, tôi muốn follow-up chạy một lượt `daily_root` với kết quả, artifact và bước dừng của recipe làm ngữ cảnh, để agent hiểu việc đã làm.
56. Là người vận hành, tôi muốn follow-up không chạy lại hay sửa recipe đã ghim, để lượt recipe luôn bất biến.
57. Là Admin, tôi muốn chạy lại recipe với tham số mới bằng cách tạo Task recipe mới, để mỗi lần chạy recipe là một Task riêng.

### Voice

58. Là người dùng giọng nói, tôi muốn Voice Task Tool Suite giữ nguyên 9 tool, để model voice không phải điền tham số có kiểu.
59. Là người vận hành, tôi chấp nhận yêu cầu bằng giọng nói tốn thêm một lượt LLM của `daily_root` trước khi recipe bắt đầu, để giữ bề mặt Voice nhỏ.

## Implementation Decisions

### Mô hình recipe (thư viện `orchestrator-flow`)

- Thêm khái niệm Task Recipe vào `orchestrator-flow`. Một recipe là một giá trị Rust do host đăng ký lúc khởi động, cùng chỗ đăng ký flow và WikiAutoFlowBinding. Recipe không được nạp từ file, DB hay request.
- Mỗi recipe có `id`, `version` (số nguyên tăng dần), `title`, `description`, `params_schema` và danh sách bước có thứ tự.
- Có hai loại bước:
  - *Tool Step* nêu tên HostTool và cách dựng tham số tool từ tham số recipe cùng output của bước trước. Đây là ánh xạ cố định trong code, không phải template do LLM sinh.
  - *Agent Step* nêu Sub-Agent trong catalog, template Delegated Task Prompt, tập tool và skill, và schema output.
- Một bước có thể đánh dấu lặp theo lô trên các mục Collection Ledger, với kích thước lô và số lần thử tối đa cho mỗi mục trước khi cách ly. Không có nhánh điều kiện, vòng lặp tùy ý hay DSL.
- Registry từ chối lúc khởi động khi có hai recipe trùng `(id, version)`, khi bước tham chiếu HostTool hoặc Sub-Agent không tồn tại, hoặc khi `params_schema` dùng kiểu ngoài tập con cho phép.
- Registry giữ được nhiều version của cùng một recipe, để Task đã ghim version cũ chạy tiếp sau khi nâng cấp nếu version đó còn được giữ.

### Tập con schema tham số

Chỉ cho phép: `string` (có `format: uri`, `multiline`), `integer` (`minimum`, `maximum`), `boolean`, `enum` (có thể có `x-options-source` để host cung cấp danh sách động), `array<string>` và `artifact` (`accept` theo MIME). Mỗi tham số có thể có `default`, `title`, `description` và `required`. Server validate tham số trước khi tạo Task và trả lỗi kèm `validation_path`.

### Interface của TaskManager

- `CreateTaskRequest` có thêm trường tùy chọn `recipe` gồm `id`, `version` tùy chọn (bỏ trống thì dùng bản mới nhất) và `params`. Khi có `recipe` thì `prompt` không bắt buộc, và tiêu đề được sinh từ recipe khi request không truyền.
- TaskManager có thêm thao tác liệt kê recipe đã đăng ký, trả metadata công khai gồm `id`, `version`, `title`, `description`, `params_schema` và danh sách bước rút gọn (`title`, `kind`, `repeat`).
- TaskView và TaskDetailView có thêm thông tin recipe đã ghim cùng tiến độ bước: bước hiện tại, trạng thái từng bước, số mục đã xong trên tổng số với bước lặp, và bước dừng kèm `stop_reason`.
- Schedule `spawn_task` vẫn chưa nhận recipe ở spec này. Kiểu request được thiết kế sao cho thêm sau không phá vỡ tương thích.

### Thực thi

- Task recipe có một runner riêng trong `orchestrator-flow`, chạy song song với đường agent loop hiện có. Runner dùng chung scheduler slot, Job Budget, Waiting Retry, recovery khi khởi động và cơ chế hủy.
- Con trỏ bước và trạng thái từng bước nằm trong `tasks.db` qua migration forward-only. Bước được commit là hoàn tất trước khi runner sang bước sau.
- Tool Step gọi HostTool qua đúng đường dispatch mà agent dùng, để phân loại `ToolEffect`, Tool Operation, quyền và ArtifactStore giữ nguyên. Tool `Mutating` không rõ kết quả đi qua đường ResolveStuckTool có sẵn.
- Agent Step tạo một Agent Session con cho Sub-Agent. Mỗi lô của bước lặp dùng một session con mới, theo ADR 0136. Output được kiểm theo schema; sai thì gửi một lượt sửa có chẩn đoán, sai tiếp thì bước thất bại.
- Bước lỗi vĩnh viễn thì Task chuyển sang trạng thái kết thúc kèm `stop_reason`, bước lỗi và kết quả từng phần. Lỗi tạm thời đi qua Waiting Retry. Runtime không bao giờ fallback sang agent loop.
- Version đã ghim không còn trong registry lúc resume thì Task kết thúc với mã lỗi riêng, không chạy theo version khác.
- Collection Ledger được tổng quát hóa theo loại mục để bước lặp dùng được cho recipe, đồng thời giữ nguyên hành vi hiện có của playlist.

### Follow-up

Follow-up trên Task recipe đi qua đường Task Follow-up hiện có, vào Agent Session của `daily_root`. Lượt này nhận một tóm tắt có cấu trúc gồm recipe, tham số, trạng thái từng bước, artifact và `stop_reason`. Recipe đã ghim không bị chạy lại. Mỗi Agent Run được đánh dấu là lượt recipe hay lượt tự do.

### Tool `recipe_start` cho `daily_root`

- Một HostTool mới chỉ cấp cho `daily_root`. Tool nhận `recipe_id` và `params`; mô tả tool liệt kê các recipe đang có kèm tóm tắt tham số.
- Tool tạo một Task recipe mới (không chạy recipe bên trong Task hiện tại) và trả về id Task. Tool có tác dụng phụ nên được phân loại là `Mutating`, kèm khóa idempotency theo lượt gọi.
- Voice Task Tool Suite giữ nguyên 9 tool.

### Recipe đầu tiên: thu thập video vào `sources/`

- Tham số: URL playlist hoặc danh sách URL video (bắt buộc có một trong hai), domain Wiki (enum động, có giá trị "agent tự chọn"), số video tối đa (integer, mặc định 50) và có ingest vào Wiki sau khi thu thập hay không (boolean).
- Các bước:
  1. Tool Step lấy danh sách video vào Collection Ledger.
  2. Tool Step lặp theo lô lấy transcript.
  3. Agent Step chọn `<domain>/<topic>` (nếu domain là "agent tự chọn").
  4. Tool Step lặp theo lô gọi `add_to_sources`.
- Định danh nguồn, xử lý trùng và cách ly theo ADR 0136.

### Server và Admin API (`voice-agent-server`)

- Đăng ký recipe đầu tiên lúc khởi động task flow.
- `GET /api/admin/background/recipes` trả danh sách recipe từ TaskManager.
- `POST /api/admin/jobs` nhận thêm `recipe: { id, version?, params }`. Khi có `recipe` thì `goal` không bắt buộc. Idempotency key giữ như hiện tại. Lỗi tham số trả về theo định dạng lỗi admin hiện có, kèm `validation_path`.
- Projection Admin Job có thêm metadata recipe và tiến độ bước.
- Thứ tự phát hành thư viện theo `AGENTS.md`. Thử trước bằng `[patch.crates-io]` trên nhánh feature, rồi thay bằng bản đã phát hành trước khi merge.

### Admin Web

- Modal "Tạo job" có ô tìm kiếm, lưới thẻ (Tự do đứng đầu, sau đó là các recipe) và form tham số dựng từ `params_schema`, theo bảng ánh xạ trong tài liệu UI.
- Form điền sẵn `default`, kiểm `required`, `minimum` và `maximum` phía client, gắn lỗi server vào trường theo `validation_path`, lỗi không khớp trường nào thì hiện ở đầu form.
- Danh sách bước chỉ đọc hiện dưới form.
- Danh sách Job có nhãn recipe. Chi tiết Job có thanh tiến độ bước ở trên timeline, và lượt follow-up tự do được đánh dấu riêng.

## Testing Decisions

Test chỉ kiểm hành vi nhìn thấy từ bên ngoài qua interface công khai: trạng thái Task, TaskDetailView, artifact, file trong `sources/` và response HTTP. Test không kiểm bảng nội bộ, tên hàm hay thứ tự gọi bên trong. Có ba seam, đã được xác nhận.

### Seam 1: interface công khai của TaskManager (seam chính)

Dùng harness hiện có của integration test `orchestrator-flow`: `ScriptedGateway` cho LLM, `yt-dlp` giả bằng script shell, vault tạm. Prior art là test thu thập playlist và Auto-Flow nhiều nguồn, test vòng đời Task (Waiting Retry, resume, hết ngân sách) và test guardrail của `sources/`.

Phủ các trường hợp:

- Recipe playlist chạy hết các bước, tạo đúng file trong `sources/`. Tool Step không tiêu lượt LLM nào: ScriptedGateway chỉ nhận lượt của Agent Step.
- Tham số sai bị từ chối lúc tạo, kèm `validation_path`. Không có Task nào được tạo.
- Registry từ chối recipe trùng `(id, version)`, tham chiếu tool không tồn tại, hoặc schema ngoài tập con.
- Restart giữa bước lặp thì Task chạy tiếp từ bước dở, không lưu trùng nguồn.
- Lỗi tạm thời đưa Task vào Waiting Retry rồi chạy tiếp. Hết ngân sách thì Task dừng và giữ kết quả từng phần.
- Agent Step trả output sai schema hai lần thì Task dừng với bước lỗi và `stop_reason`, và không có lượt agent loop nào tiếp theo.
- Mục lỗi trong lô bị cách ly, các mục khác vẫn xong.
- Version đã ghim bị gỡ khỏi registry thì Task kết thúc với mã lỗi riêng khi resume.
- Hủy Task dừng bước đang chạy.
- Follow-up trên Task recipe chạy một lượt agent loop có ngữ cảnh recipe, và recipe không chạy lại.
- `recipe_start` từ `daily_root` tạo một Task recipe mới. Gọi lại với cùng khóa idempotency không tạo Task thứ hai.
- Task không có recipe vẫn chạy agent loop như cũ (test hiện có phải tiếp tục pass).

### Seam 2: Admin HTTP của server

Chỉ kiểm contract: `GET /api/admin/background/recipes` trả đúng hình dạng, `POST /api/admin/jobs` có `recipe` tạo Task và trả lỗi tham số kèm `validation_path`, projection Job có metadata recipe. Prior art là test của module admin background, ví dụ API resume waiting và validation attachment.

### Seam 3: Vitest của admin-web

Kiểm form dựng từ schema: mỗi kiểu trong tập con render đúng control, `default` được điền sẵn, trường bắt buộc bị chặn khi trống, lỗi server gắn đúng trường, thẻ "Tự do" giữ hành vi cũ. Prior art là test component trong thư mục jobs và test API client agent-jobs.

Chạy test theo `AGENTS.md`: luôn dùng `scripts/agent-test.sh`, chạy test có mục tiêu trước rồi mới chạy toàn workspace. Test thư viện chạy trong repo thư viện.

## Out of Scope

- Admin tạo, sửa hay xóa recipe trên giao diện. Recipe chỉ nằm trong code.
- DSL, nhánh điều kiện, vòng lặp tùy ý hay workflow engine ngoài.
- LLM sinh, sửa hay ghép recipe.
- Tool recipe trong Voice Task Tool Suite.
- Fallback tự động sang agent loop khi bước lỗi.
- Schedule `spawn_task` với recipe, và chọn lịch ngay trong modal.
- Lưu bộ tham số hay dùng thành preset.
- Trang danh mục recipe riêng ngoài modal.
- Recipe thứ hai trở đi, như báo cáo định kỳ hay lô video Muse. Chỉ làm sau khi có số liệu từ `tasks.db`.

## Further Notes

- Trước đây repo từng có "knowledge fixed job" thuộc JobEngine cũ (`.scratch/knowledge-fixed-job/`), đã bị ADR 0127 thay thế. Spec này không dùng lại mã hay bảng của nó.
- Thêm thuật ngữ **Task Recipe** vào `GLOSSARY.md` và đổi ADR 0140 sang Accepted khi bắt đầu triển khai.
- Cần đo độ ổn định trước và sau bằng `tasks.db`: tỉ lệ Task playlist hoàn tất, số lượt LLM mỗi Task, và tần suất `STEP_LIMIT_REACHED`. Số liệu này dùng để quyết định recipe tiếp theo.
