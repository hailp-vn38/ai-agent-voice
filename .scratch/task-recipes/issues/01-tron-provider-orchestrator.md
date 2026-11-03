# 01: Orchestrator trộn `ProviderRuntime` với `llm.providers`

**What to build:** Một Orchestrator phục vụ được cả agent dùng provider của `ProviderRuntime` lẫn agent dùng provider HTTP trong `llm.providers`. Nhờ vậy Root recipe (adapter xác định) và Sub-Agent của Agent Step (LLM thật) chạy chung một cây agent. Đây là thay đổi duy nhất cần ở `orchestrator-agent`.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Builder nhận cùng lúc `ProviderRuntime` và `llm.providers`. Alias do runtime khai báo đi vào runtime, các alias còn lại đi vào `ClientGateway`.
- [ ] Build thất bại với lỗi cấu hình rõ ràng khi một alias được khai báo ở cả hai nơi.
- [ ] Chỉ dùng một trong hai thì hành vi giữ nguyên. Test hiện có về provider runtime và gateway tiếp tục pass.
- [ ] Fingerprint của cả hai loại alias vẫn đọc được, và recovery so fingerprint đúng như trước.
- [ ] Integration test: Root dùng alias runtime gọi `subagent` tới một child dùng alias HTTP giả. Mỗi request đi đúng nơi. Điểm khởi đầu là spike `spike_recipe_gateway` trên nhánh `spike/recipe-gateway`.
- [ ] Làm trên nhánh feature của repo thư viện, test bằng `scripts/agent-test.sh`.
