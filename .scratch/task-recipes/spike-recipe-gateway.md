# Spike: Task Recipe là một adapter `ProviderRuntime` xác định

Ngày: 2026-11-03. Code spike (vứt đi) nằm ở nhánh `spike/recipe-gateway` của `orchestrator_agent_rs`, file test `spike_recipe_gateway.rs`. Chưa commit.

## Câu hỏi

Có chạy được Task Recipe hoàn toàn trên runtime hiện có không? Cách làm: Root của recipe dùng một provider alias riêng, ví dụ `recipe`. Adapter của provider này đọc transcript và phát tool call kế tiếp bằng code, không gọi model. Nếu được thì không cần thêm interface mới để chạy tool bên ngoài Agent Run (phương án A).

## Kết quả

Cả ba test đều pass khi chạy qua `scripts/agent-test.sh`.

| Kịch bản | Kết quả quan sát |
| --- | --- |
| Chuỗi `fetch_list` → `subagent(classifier)` → `save_item` | Run `Completed`, tool chạy đúng thứ tự, output của Agent Step được truyền sang bước sau. LLM thật chỉ được gọi một lần, cho Agent Step |
| Shutdown khi Tool Step `ReadOnly` đang chạy, rồi build lại | Recovery resume `attempt=1` và chạy tiếp tới `Completed`. Tool `ReadOnly` chạy lại một lần (replay-safe), các bước sau chạy đúng một lần |
| Một bước phát 5 tool call `Mutating`, usage `0/0` | Run `Completed` với `steps=2`, cả 5 lời gọi đều chạy, token ghi nhận `0/0` |

## Kết luận

Phương án B dùng được. Seam `ProviderRuntime` đã có thật (server adapter, test adapter), và chọn adapter theo từng agent qua `AgentDefinition::provider_id`. Transcript chính là con trỏ bước. Recovery, Tool Operation, `ToolEffect`, chính sách không chạy lại mù, `subagent` và timeline đều dùng được ngay, không phải viết thêm.

## Phát hiện cần đưa vào thiết kế

1. **Builder chưa cho trộn provider.** `OrchestratorBuilder` chỉ dùng hoặc `ProviderRuntime`, hoặc `ClientGateway` dựng từ `llm.providers`, không dùng được cả hai. Server hiện chạy bằng `ClientGateway`, và `ClientGateway` không được export để host tự bọc. Cần một thay đổi nhỏ trong `orchestrator-agent`: alias do `ProviderRuntime` khai báo đi vào runtime, các alias còn lại đi vào `ClientGateway`. Phạm vi hẹp hơn nhiều so với một interface chạy tool bên ngoài Agent Run.
2. **Bước recipe bị tính vào step cap.** Mỗi lượt adapter là một step của Agent Run, nên recipe phải đặt `with_steps` theo số bước của nó. Vì vậy bước lặp theo lô nên phát nhiều tool call trong một response (đã chứng minh với 5 lời gọi), thay vì tốn một step cho mỗi mục.
3. **Token bị ước lượng nếu không khai báo usage.** Adapter trả `usage: None` thì runtime ước lượng khoảng 2.000 token input cho mỗi lần chạy, và con số này ăn vào Job Budget. Adapter phải luôn trả `ModelUsage::new(0, 0)`.
4. **Recipe phải là hàm thuần của transcript.** Adapter không được giữ state riêng, vì sau restart chỉ còn transcript. Spike làm đúng như vậy, và nhờ đó resume tự nhiên.
5. **Tham số recipe đi qua user message đầu tiên** dưới dạng JSON, và Rust validate trước khi submit. Tool result của Agent Step là text, nên recipe cần một quy ước đọc output có kiểu (ví dụ child gọi tool kết thúc có schema), thay vì parse text như trong spike.
6. **Chưa kiểm:** Waiting Retry khi Agent Step con bị 429, hủy giữa lô, compaction khi transcript recipe dài, và Tool Step `Mutating` bị gián đoạn (theo chính sách hiện có thì phải chặn, không chạy lại).

## Ảnh hưởng tới spec và ticket

- Thêm ticket đầu tiên: hỗ trợ trộn `ProviderRuntime` với `llm.providers` trong `orchestrator-agent`.
- Ticket 02 không cần runner hay bảng con trỏ bước riêng. Recipe là một adapter cộng một Root agent dành riêng cho recipe, và tiến độ đọc từ transcript.
- Ticket 03 chủ yếu chỉ còn là kiểm chứng, vì phần lớn độ bền đã có sẵn từ runtime.
- Ticket 01 (tổng quát hóa ledger) bỏ được.
