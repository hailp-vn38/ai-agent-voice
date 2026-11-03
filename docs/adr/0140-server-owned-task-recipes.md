# ADR 0140 — Server-Owned Task Recipes

Status: Proposed, 2026-11-03.
Extends: [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md), [ADR 0133](0133-durable-waiting-retry-and-job-budgets.md), [ADR 0136](0136-collection-ledger-and-bounded-autoflow-batches.md).

Mọi Background Task hiện chạy theo agent loop: Root tự quyết gọi Sub-Agent nào, Sub-Agent tự quyết gọi tool nào. Với việc lặp lại và đã biết trước các bước, như thu thập playlist vào `sources/`, cách này làm mỗi lần chạy đi một đường khác nhau, tốn token cho việc chọn bước và có thể chạm step cap giữa chừng. ADR 0136 đã ghi nhận rằng skill chỉ là lời nhắc, không ép được quy trình. Đề xuất: thêm **Task Recipe**, một quy trình cố định do server định nghĩa trong code. Rust giữ thứ tự bước, còn LLM chỉ làm những bước cần phán đoán.

## Decisions

- **Recipe do server sở hữu, nằm trong code Rust.** Mỗi recipe có `recipe_id`, `version`, schema tham số và danh sách bước. Recipe không được sinh ra, sửa hay ghép lúc chạy bởi LLM, request hay Admin. Đây là điểm khác cốt lõi so với Plan Proposal của ADR 0094/0098.
- **Có hai loại bước.**
  - *Tool Step* gọi HostTool trực tiếp, không qua model. Bước này đi qua cùng pipeline với tool call của agent: phân loại `ToolEffect`, Tool Operation, ArtifactStore và quyền theo phạm vi Task.
  - *Agent Step* chạy một Sub-Agent có sẵn trong catalog, với Delegated Task Prompt dựng từ template của recipe, tập tool và skill cố định, và output được Rust kiểm theo schema. Output sai schema được sửa một lần bằng chẩn đoán có kiểu, sai tiếp thì bước thất bại.
- **Cấu trúc chỉ có chuỗi tuần tự và fan-out theo ledger.** Một bước có thể lặp qua các mục trong Collection Ledger theo lô giới hạn, với cách ly mục lỗi như Auto-Flow. Recipe không có nhánh điều kiện, vòng lặp tùy ý hay DSL. Việc cần rẽ nhánh theo nội dung thì để agent loop làm.
- **Recipe được ghim lúc tạo Task.** Task lưu `(recipe_id, version, params)` đã validate. Đổi code recipe không ảnh hưởng Task đang chạy. Nếu version đã ghim không còn trong binary sau khi nâng cấp, Task kết thúc với lý do rõ ràng thay vì chạy theo version mới.
- **Tiến độ là con trỏ bước lưu bền.** Bước hoàn tất được commit vào `tasks.db` trước khi sang bước sau. Restart, `waiting_retry` và resume chạy tiếp từ bước dở. Recipe dùng chung Job Budget với Task thường. Tool `Mutating` không rõ kết quả vẫn không chạy lại mù mà đi qua đường gỡ của ADR 0136.
- **Chọn recipe một cách tường minh.** Recipe được chọn khi `CreateTaskRequest` có `recipe` (từ Admin, API hoặc Schedule `spawn_task`), hoặc khi `daily_root` gọi tool `recipe_start`. Tool này chỉ nhận `recipe_id` và tham số; Rust validate trước khi tạo Task. Không có recipe thì Task chạy agent loop như hiện tại. Agent loop vẫn là đường mặc định.
- **Voice không có tool recipe riêng.** Voice Task Tool Suite giữ nguyên 9 tool. Yêu cầu bằng giọng nói tạo Task thường, rồi `daily_root` quyết định có gọi `recipe_start` hay không. Model voice vì vậy không phải điền tham số có kiểu, và bề mặt Voice không mở rộng.
- **Bước thất bại thì dừng, không fallback ngầm.** Agent Step sai schema sau một lần sửa, hoặc Tool Step lỗi vĩnh viễn, làm Task kết thúc kèm `stop_reason`, bước lỗi và kết quả từng phần. Runtime không tự chuyển sang agent loop, để mỗi lần chạy recipe đều có cùng hình dạng và đối chiếu được với nhau. Lỗi tạm thời vẫn đi qua `waiting_retry` như mọi Task.
- **Follow-up chạy agent loop tự do.** Task Follow-up trên Task recipe mở một lượt `daily_root` bình thường, với kết quả, artifact và bước dừng của recipe làm ngữ cảnh. Recipe đã ghim không bị chạy lại hay sửa. Muốn chạy lại recipe với tham số mới thì tạo Task recipe mới. Follow-up cũng là đường để người dùng xử lý tiếp một recipe đã dừng giữa chừng.
- **Recipe đầu tiên là thu thập playlist hoặc danh sách video vào `sources/`.** Recipe mở rộng thêm khi số liệu từ `tasks.db` chỉ ra loại việc lặp nhiều và hay thất bại vì chọn sai bước.

## Considered Options

- **LLM lập plan rồi Rust chạy DAG:** đã thử ở ADR 0094/0098 và bị thay thế. Model thường sinh plan sai schema (`invalid_plan`). Recipe giữ lợi ích "bước cố định" nhưng bỏ phần model tự soạn plan.
- **Viết skill chặt hơn:** từ chối với lý do của ADR 0136. Skill không ép được thứ tự bước, không ngăn được step cap, và không cho Rust biết bước nào đã xong.
- **Workflow engine tổng quát hoặc DSL (Temporal, YAML có nhánh):** từ chối. ADR 0095 giữ hệ thống một process, còn transcript, ledger và Tool Operation đã đủ nền để resume. Một DSL sẽ thành ngôn ngữ thứ hai phải test và version.
- **Admin tự định nghĩa recipe:** để sau. Admin đã chỉnh được Sub-Agent và skill mà Agent Step tham chiếu. Cho Admin sửa cả cấu trúc bước sẽ cần DSL, schema và migration riêng.
- **Viết worker riêng cho từng use case:** từ chối. Mỗi worker sẽ tự làm lại resume, budget và Read View. Một runner dùng chung giữ các phần đó ở một chỗ.

## Consequences

- Có hai đường thực thi Task để test và quan sát. Job Read View cần hiện danh sách bước của recipe và bước hiện tại, bên cạnh Agent Run. Thiết kế giao diện tạo và xem Job recipe nằm ở [task-recipe-ui.md](../task-recipe-ui.md).
- Code nằm ở `orchestrator_task_rs` (runner, con trỏ bước, migration `tasks.db`), còn recipe cụ thể do `voice-agent-server` đăng ký. Thay đổi phải phát hành theo thứ tự thư viện trong `AGENTS.md`.
- Collection Ledger hiện chỉ ghi video của playlist. Muốn fan-out cho recipe khác thì phải tổng quát hóa ledger theo loại mục.
- Kết quả của recipe ổn định hơn, nhưng recipe không tự thích ứng. Đầu vào lạ, ví dụ playlist riêng tư, phải thất bại có lý do để người dùng chuyển sang Task thường.
- Một Task có thể gồm lượt recipe rồi đến các lượt agent loop. Job Read View phải phân biệt hai loại lượt này, và chỉ lượt đầu được tính là "chạy theo recipe" khi đo độ ổn định.
- Yêu cầu bằng giọng nói tốn thêm một lượt LLM của `daily_root` trước khi recipe bắt đầu. Đây là cái giá để giữ Voice Task Tool Suite nhỏ.
