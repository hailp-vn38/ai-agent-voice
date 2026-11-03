# ADR 0140 — Server-Owned Task Recipes

Status: Accepted, 2026-11-03.
Extends: [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md), [ADR 0133](0133-durable-waiting-retry-and-job-budgets.md), [ADR 0136](0136-collection-ledger-and-bounded-autoflow-batches.md).

Mọi Background Task hiện chạy theo agent loop: Root tự quyết gọi Sub-Agent nào, Sub-Agent tự quyết gọi tool nào. Với việc lặp lại và đã biết trước các bước, như thu thập playlist vào `sources/`, cách này làm mỗi lần chạy đi một đường khác nhau, tốn token cho việc chọn bước và có thể chạm step cap giữa chừng. ADR 0136 đã ghi nhận rằng skill chỉ là lời nhắc, không ép được quy trình.

Đề xuất: thêm **Task Recipe**, một quy trình cố định do server định nghĩa trong code. Recipe chạy như một Root agent bình thường, nhưng "model" của Root là code Rust xác định thay cho LLM. Rust giữ thứ tự bước, còn LLM chỉ làm những bước cần phán đoán. [Spike](../../.scratch/task-recipes/spike-recipe-gateway.md) đã chứng minh cách này chạy được trên runtime hiện có.

## Decisions

- **Recipe là một adapter `ProviderRuntime` xác định.** Mỗi recipe có một Root agent riêng với provider alias `recipe` và `model_id` là `<recipe_id>@<version>`. Khi runtime hỏi bước kế tiếp, adapter gọi code của recipe thay cho model. Seam `ProviderRuntime` đã có sẵn, và adapter được chọn theo `AgentDefinition::provider_id`, nên không cần thêm interface chạy tool bên ngoài Agent Run.
- **Recipe là hàm thuần của tham số và transcript.** Interface của recipe gồm `id`, `version`, `title`, `description`, `params_schema`, `outline` (tiêu đề, loại và có lặp hay không của từng bước), `max_steps`, cùng hai hàm: `next` trả bước kế tiếp, và `progress` trả `RecipeProgress`. Cả hai chỉ đọc tham số và transcript. Recipe không giữ state riêng, nên sau restart transcript chính là con trỏ bước. Recipe không được sinh ra, sửa hay ghép lúc chạy bởi LLM, request hay Admin. Đây là điểm khác cốt lõi so với Plan Proposal của ADR 0094/0098.
- **Có hai loại bước, đều đi qua runtime hiện có.**
  - *Tool Step* là tool call do adapter phát ra. Vì vậy `ToolEffect`, Tool Operation, quyền theo phạm vi Task, ArtifactStore và chính sách không chạy lại mù đều giữ nguyên.
  - *Agent Step* là một lời gọi `subagent` tới Sub-Agent trong catalog, với Delegated Task Prompt dựng từ template của recipe. Sub-Agent kết thúc bằng một tool hoàn tất có schema output. Output sai schema thì recipe resume session con một lần, kèm chẩn đoán có kiểu.
- **Lặp theo lô là nhiều tool call trong một bước.** Một bước của recipe phát tối đa một lô tool call, và runtime chạy hết cả lô. Step cap của Root recipe do recipe khai báo qua `max_steps`, không dùng giới hạn của agent tự do. Danh sách mục và trạng thái từng mục vẫn nằm trong Collection Ledger.
- **Adapter luôn báo usage bằng 0.** Lượt của adapter không tốn token, nên không được để runtime ước lượng và trừ vào Job Budget. Agent Step con vẫn tính token như thường.
- **Recipe được ghim qua `model_id`.** Version nằm trong Agent Run như mọi model khác. Registry chỉ giữ bản hiện tại của mỗi recipe. Nếu version của một Task đang resume không khớp với bản trong binary, adapter trả lỗi vĩnh viễn `RECIPE_VERSION_UNAVAILABLE`, và Task không chạy theo version khác.
- **Orchestrator trộn được hai loại provider.** Alias do `ProviderRuntime` khai báo đi vào runtime, các alias còn lại đi vào `ClientGateway`. Đây là thay đổi duy nhất cần ở `orchestrator-agent`.
- **Chọn recipe một cách tường minh.** Recipe được chọn khi `CreateTaskRequest` có `recipe` (từ Admin hoặc API), hoặc khi `daily_root` gọi tool `recipe_start`. Tool này chỉ nhận `recipe_id` và tham số; Rust validate trước khi tạo Task, rồi đưa tham số đã validate vào user message đầu tiên của Root recipe. Không có recipe thì Task chạy agent loop như hiện tại. Agent loop vẫn là đường mặc định.
- **Voice không có tool recipe riêng.** Voice Task Tool Suite giữ nguyên 9 tool. Yêu cầu bằng giọng nói tạo Task thường, rồi `daily_root` quyết định có gọi `recipe_start` hay không. Model voice vì vậy không phải điền tham số có kiểu, và bề mặt Voice không mở rộng.
- **Bước thất bại thì dừng, không fallback ngầm.** Khi Agent Step sai schema sau một lần sửa, hoặc Tool Step trả lỗi vĩnh viễn, recipe trả kết thúc lỗi kèm `stop_reason` và bước lỗi. Task kết thúc và giữ kết quả từng phần trong transcript và artifact. Runtime không tự chuyển sang agent loop, để mỗi lần chạy recipe đều có cùng hình dạng và đối chiếu được với nhau. Lỗi tạm thời vẫn đi qua `waiting_retry` như mọi Task.
- **Follow-up chạy agent loop tự do.** Task Follow-up trên Task recipe mở một lượt `daily_root`, với `RecipeProgress`, tham số và artifact của recipe làm ngữ cảnh. Root recipe không nhận thêm lượt nào, và lượt recipe đã chạy không bị sửa. Muốn chạy lại recipe với tham số mới thì tạo Task recipe mới. Follow-up cũng là đường để người dùng xử lý tiếp một recipe đã dừng giữa chừng.
- **Recipe đầu tiên là thu thập playlist hoặc danh sách video vào `sources/`.** Recipe mở rộng thêm khi số liệu từ `tasks.db` chỉ ra loại việc lặp nhiều và hay thất bại vì chọn sai bước.

## Considered Options

- **Runner riêng cùng interface chạy tool bên ngoài Agent Run (`Orchestrator::execute_tool`):** từ chối sau spike. Cách này mở rộng interface của `orchestrator-agent` và buộc runner tự làm lại journal, recovery, hủy, budget và timeline mà runtime đã có. Adapter `ProviderRuntime` có được tất cả những thứ đó qua một seam sẵn có.
- **Định nghĩa recipe bằng danh sách bước khai báo có ánh xạ tham số:** từ chối. Ánh xạ "tham số tool lấy từ output bước trước" sẽ thành một DSL viết bằng Rust, và interface gần phức tạp bằng phần implementation. Hàm `next` bằng code thường nhỏ hơn và test được trực tiếp.
- **LLM lập plan rồi Rust chạy DAG:** đã thử ở ADR 0094/0098 và bị thay thế. Model thường sinh plan sai schema (`invalid_plan`).
- **Viết skill chặt hơn:** từ chối với lý do của ADR 0136. Skill không ép được thứ tự bước, không ngăn được step cap, và không cho Rust biết bước nào đã xong.
- **Workflow engine tổng quát hoặc DSL (Temporal, YAML có nhánh):** từ chối. ADR 0095 giữ hệ thống một process, và transcript cùng recovery hiện có đã đủ nền để resume.
- **Registry giữ nhiều version của một recipe:** từ chối khi mới có một recipe. Ghim version và dừng rõ ràng khi không khớp là đủ.
- **Admin tự định nghĩa recipe:** để sau. Admin đã chỉnh được Sub-Agent và skill mà Agent Step tham chiếu.

## Consequences

- Task recipe là Task thường với một Root đặc biệt. Recovery, Waiting Retry, hủy, Job Budget, Tool Operation và timeline không có đường code riêng; chỉ `RecipeProgress` là mới trong Job Read View. Thiết kế giao diện tạo và xem Job recipe nằm ở [task-recipe-ui.md](../task-recipe-ui.md).
- Hàm `next` phải xác định: cùng tham số và cùng transcript thì luôn ra cùng bước. Mọi phán đoán phải nằm trong Agent Step.
- Transcript của Root recipe chứa nguyên tool result. Root recipe không được compaction tóm tắt mất tool result mà `next` cần đọc. Recipe chỉ nên đưa vào transcript tham chiếu ngắn (id mục, đường dẫn artifact), còn dữ liệu lớn để trong Collection Ledger và artifact.
- Thay đổi chạm `orchestrator-agent` (trộn provider) và `orchestrator-flow` (registry, adapter, `RecipeProgress`, follow-up). Phải phát hành theo thứ tự thư viện trong `AGENTS.md`.
- Kết quả của recipe ổn định hơn, nhưng recipe không tự thích ứng. Đầu vào lạ, ví dụ playlist riêng tư, phải thất bại có lý do để người dùng chuyển sang Task thường.
- Một Task có thể gồm lượt recipe rồi đến các lượt `daily_root`. Job Read View phải phân biệt hai loại lượt này, và chỉ lượt recipe được tính khi đo độ ổn định.
- Yêu cầu bằng giọng nói tốn thêm một lượt LLM của `daily_root` trước khi recipe bắt đầu. Đây là cái giá để giữ Voice Task Tool Suite nhỏ.
