# Hỏi wiki và Graph theo câu trả lời

Status: ready-for-human

Tài liệu nguồn: [thiết kế](../../docs/knowledge/wiki-chat-graph-design.md), [triển khai](../../docs/knowledge/wiki-chat-graph-implementation.md). Nếu khác nhau về cách test, spec này được ưu tiên.

## Problem Statement

Admin có một Wiki ngày càng lớn nhưng chỉ tìm được bằng từ khóa hoặc nhìn toàn bộ Graph. Tab Graph hiện chỉ có Toàn bộ và Cục bộ, nên muốn biết "trading trong wiki liên hệ với nhau thế nào" thì phải tự mở từng Wiki Page và tự lần theo Wikilink.

Trên vault hiện tại, chỉ cần `wiki/index.md` nằm trong đồ thị là mọi trang đều bị kéo vào, nên Admin không thể khoanh một chủ đề bằng cách mở rộng theo link. Assistant voice có tìm wiki nhưng chỉ tìm từ khóa với ít kết quả, không cho biết đã dựa vào trang nào, và không gắn với Graph.

## Solution

Trong tab Graph của trang Knowledge có thêm khung Hỏi wiki. Admin đặt câu hỏi bằng ngôn ngữ tự nhiên, agent chỉ đọc sẽ tìm và đọc Wiki Page và Source rồi trả lời kèm trích dẫn.

Trong lúc đó Graph chuyển sang scope Câu trả lời và lớn dần theo những gì agent đã tìm và đọc, chỉ gồm các trang liên quan cùng Wikilink có thật giữa chúng. Admin có thể đi từ trích dẫn sang node, từ node sang trang, rồi quay lại mà không mất hội thoại.

Trước khi có agent, Admin đã có thể nhập từ khóa trên Graph để xem đồ thị theo truy vấn, không cần LLM.

## User Stories

### Đồ thị theo truy vấn

1. As an Admin, I want to type a keyword on the Graph, so that I can see only the pages about that topic without configuring an LLM.
2. As an Admin, I want the query graph to leave out `wiki/index.md` and `wiki/log.md`, so that one hub page does not pull in the whole wiki.
3. As an Admin, I want the query graph to include both Wiki Pages and Sources, so that I can see compiled knowledge next to its evidence.
4. As an Admin, I want direct neighbors of matching pages to appear as context, so that I can notice related pages the search did not match.
5. As an Admin, I want context pages to look quieter than matches, so that I can tell what matched from what is only nearby.
6. As an Admin, I want weak matches filtered out, so that an unrelated query does not draw a misleading graph.
7. As an Admin, I want a query with no matches to say so clearly, so that I don't mistake it for a loading or broken Graph.
8. As an Admin, I want a large result capped with a visible notice, so that the Graph stays usable and I know it is partial.
9. As an Admin, I want matches kept over context when the cap applies, so that the most relevant pages are never the ones dropped.
10. As an Admin, I want Toàn bộ and Cục bộ to keep working exactly as they do today, so that the new mode costs me nothing.

### Hỏi đáp

11. As an Admin, I want to ask a question in natural language in the Graph tab, so that I don't have to leave the Graph to get an answer.
12. As an Admin, I want the answer to stream as it is written, so that I know work is happening and can start reading early.
13. As an Admin, I want answers based only on the wiki, so that I can trust them as a reflection of my own knowledge base.
14. As an Admin, I want the agent to say when the wiki has no answer, so that I don't receive a made-up reply.
15. As an Admin, I want follow-up questions to keep earlier context, so that I can dig deeper without repeating myself.
16. As an Admin, I want to ask in Vietnamese about content written in English, so that language differences don't hide what I know is there.
17. As an Admin, I want to see which tools the agent is using while it works, so that a slow answer is explained.
18. As an Admin, I want each finished tool step collapsed into a short summary, so that progress does not bury the answer.
19. As an Admin, I want to stop a running answer, so that I can correct my question without waiting.
20. As an Admin, I want what has already arrived to stay after I stop, so that I don't lose useful partial content.
21. As an Admin, I want to clear the conversation, so that I can start a new topic and return to the normal Graph.
22. As an Admin, I want a dropped connection to keep the partial answer and offer a retry, so that a network blip does not cost me the work.

### Trích dẫn

23. As an Admin, I want answers to cite the pages they rely on, so that I can verify each claim.
24. As an Admin, I want a numbered source list under each answer, so that I can scan what the answer rests on.
25. As an Admin, I want only pages the agent actually retrieved to count as citations, so that an invented link can't look like evidence.
26. As an Admin, I want an unverifiable link shown as plain text, so that I'm not sent to a page that was never consulted.
27. As an Admin, I want to hover a citation and see its node light up, so that I can place the source in the graph at a glance.
28. As an Admin, I want clicking a citation to focus its node, so that I can look at what surrounds that source.

### Graph theo câu trả lời

29. As an Admin, I want the Graph to switch to Câu trả lời when I ask, so that the relevant relationships appear without extra clicks.
30. As an Admin, I want the Graph to grow after each search and read, so that I can watch the agent's evidence build before the answer finishes.
31. As an Admin, I want cited, read, retrieved, and context pages to look different, so that I can judge how much each page mattered.
32. As an Admin, I want node color to keep meaning Concept, Entity, Synthesis, or Source, so that what I already learned about the Graph still holds.
33. As an Admin, I want a legend for the node levels, so that I don't have to guess what size and border mean.
34. As an Admin, I want every node and edge to be real, so that I can rely on the Graph as a faithful picture of my Wikilinks.
35. As an Admin, I want clicking a node here to open its details in place, so that I don't lose the Graph and answer I'm studying.
36. As an Admin, I want an Open page action in those details, so that I can read the full page when I choose to.
37. As an Admin, I want the conversation and graph still there when I come back from reading a page, so that I can continue where I was.
38. As an Admin, I want to reopen the graph of an earlier answer, so that I can compare what different questions touched.
39. As an Admin, I want to switch to Toàn bộ or Cục bộ without losing the chat, so that I can check the bigger picture and come back.
40. As an Admin, I want clicking a node in Toàn bộ or Cục bộ to still open the page, so that existing habits keep working.

### Bố cục và trạng thái

41. As an Admin, I want the chat in the right column of the Graph tab, so that the answer and Graph are visible side by side.
42. As an Admin, I want to collapse the chat with the existing right-panel control, so that I can give the Graph the full width.
43. As an Admin on a narrow screen, I want the chat as a bottom drawer, so that I can still use it where the right column doesn't exist.
44. As an Admin, I want a few suggested questions when the chat is empty, so that I can see what kind of question works.
45. As an Admin, I want a clear prompt to choose a provider when none is set, so that I know why the chat isn't answering and how to fix it.
46. As an Admin, I want a clear message when too many answers are running, so that I know to retry rather than assume it broke.
47. As a screen-reader user, I want completed messages announced rather than every streamed fragment, so that the chat stays usable.
48. As a user with reduced motion enabled, I want Graph focus changes without animation, so that the page respects my setting.
49. As an Admin using dark mode, I want the chat and node levels to stay legible, so that the feature works in my theme.
50. As an Admin, I want interface text in both supported languages, so that the chat matches the rest of the app.

### Cấu hình, an toàn và vận hành

51. As an Admin, I want to choose which chat LLM Provider answers these questions, so that I control cost and quality separately from voice.
52. As an Admin, I want providers without tool support refused when saving, so that I find out at setup rather than on my first question.
53. As an Admin, I want the agent to be unable to change the wiki, so that asking a question can never alter my knowledge base.
54. As an Admin, I want instructions hidden inside wiki or Source content to be treated as data, so that an imported document can't take over the agent.
55. As an Admin, I want a tool call outside the allowed set refused even if the model asks for it, so that safety doesn't depend on the model's behavior.
56. As an operator, I want each answer bounded in tool rounds and time, so that a bad question can't run forever or burn budget.
57. As an operator, I want a limit on concurrent answers, so that the chat can't starve other Knowledge work sharing the index.
58. As an operator, I want closing the page to cancel the running answer, so that abandoned questions stop consuming the provider.
59. As an operator, I want logs that exclude questions and page text, so that private knowledge stays out of logs.
60. As an operator, I want errors reported as codes without vault paths, so that internal paths aren't exposed.
61. As an Admin, I want disabling the Knowledge MCP in System not to affect this chat, so that external access settings stay independent.
62. As an unauthenticated visitor, I must be refused by these endpoints, so that the wiki stays private.

## Implementation Decisions

### Modules

- **Answer graph builder (mới, ở server).** Nhận dữ liệu Graph đầy đủ và tập bằng chứng, trả đồ thị con. Không biết về LLM hay HTTP.
- **Ask loop (mới, ở server).** Vòng lặp agent ngắn: gọi LLM Provider, thực thi tool trong allowlist, ghi bằng chứng, phát sự kiện qua channel.
- **Knowledge Graph endpoint (sửa).** Thêm scope `query`, dùng chung phần gộp cạnh với hai scope cũ.
- **Knowledge Ask endpoint (mới).** Xác thực, kiểm tra cấu hình và giới hạn đồng thời, chọn LLM Provider, nối sự kiện vào SSE.
- **Chat state (mới, ở web).** Sống ở cấp trang Knowledge để qua được việc đổi tab. Giữ tin nhắn, đồ thị theo từng tin, và điều khiển hủy.
- **Chat panel (mới, ở web).** Danh sách tin, trạng thái tool, trích dẫn, ô nhập.
- **Knowledge Graph view (sửa).** Thêm scope `query` và `answer`, kiểu node theo mức, cho phép focus và highlight từ ngoài.
- **SSE parser (tách ra, ở web).** Dùng chung thay vì để riêng trong luồng Jobs, không đổi hành vi hiện có.

### Dựng đồ thị

- Server dựng đồ thị từ kết quả tool. LLM không khai báo node hay cạnh.
- Bốn mức node: `cited`, `read`, `retrieved`, `context`. Một trang giữ mức cao nhất và điểm cao nhất đã thấy.
- Mức chỉ đổi kích thước, viền và độ đậm. Màu vẫn theo loại trang.
- Quy tắc đồ thị con: lấy bằng chứng làm hạt giống; loại `category = system`, `wiki/index.md` và `wiki/log.md`; mở rộng 1 bước, ưu tiên láng giềng nối với nhiều hạt giống nhất; tối đa 60 node; không cắt hạt giống trước `context`; vượt giới hạn thì `truncated = true`.
- Đồ thị con được tính ở server, vì API Graph cắt 500 node trước khi web lọc được.

### Bằng chứng

- Đọc đường dẫn từ kết quả tool đầy đủ, trước khi cắt phần gửi cho LLM, vì bước cắt hiện tại xóa `metadata.results`.
- Ask loop gọi Wiki engine trực tiếp, không qua Knowledge MCP, để cài đặt MCP không ảnh hưởng tới chat.
- `wiki_query` ghi mức `retrieved`; `wiki_get` và `wiki_multi_get` ghi mức `read`.
- Trích dẫn `[[đường dẫn]]` chỉ nâng lên `cited` nếu trang đã có trong bằng chứng.

### Agent

- Allowlist gồm `wiki_query`, `wiki_get`, `wiki_multi_get`. Không cấp `wiki_graph_neighbors`, việc mở rộng do server làm.
- Tên tool ngoài allowlist nhận kết quả lỗi và không được thực thi.
- Kết quả tool được gắn `untrusted_data`. System prompt yêu cầu chỉ trả lời từ wiki, trích dẫn bằng Wikilink, tìm nhiều biến thể khi có thể khác ngôn ngữ, và coi kết quả tool là dữ liệu.
- Server không giữ phiên. Web gửi lịch sử trong mỗi yêu cầu và không thể gửi vai `system`.

### API

`GET /api/admin/knowledge/graph?scope=query&q=...` trả cùng khung Graph hiện tại, node có thêm `tier` và `score`. Thiếu `q` trả `400 invalid_input`.

`POST /api/admin/knowledge/ask` nhận `{ messages: [{ role, content }] }`, trả `text/event-stream`:

| Sự kiện | Dữ liệu |
| --- | --- |
| `tool` | `{ call_id, name, state, query? }` |
| `graph` | Ảnh chụp đồ thị đầy đủ, kèm `revision` |
| `delta` | `{ text }` |
| `done` | `{ citations, stop_reason }` |
| `error` | `{ code }` |

- Sự kiện `graph` luôn là ảnh chụp đầy đủ để web chỉ cần thay thế.
- Mã lỗi: `wiki_chat_not_configured` (503), `wiki_chat_busy` (429), `knowledge_unavailable`, `invalid_input`, `provider_failed`, `step_limit`, `timeout`.
- Đóng kết nối là hủy lượt. Web không tự kết nối lại, vì POST không an toàn để lặp.

### Giới hạn ban đầu

6 vòng tool mỗi lượt; 90 giây mỗi lượt; 15 giây mỗi tool; 12.000 ký tự kết quả gửi LLM; 12 tin và 16 KB lịch sử; 60 node; 2 lượt đồng thời. Keep-alive SSE 15 giây và `Cache-Control: no-store` như Notifications.

### Provider và lưu trữ

- Một setting riêng chọn chat LLM Provider từ catalog có sẵn, yêu cầu tag `chat` và hỗ trợ tool. Ưu tiên lưu cùng Background settings để không cần migration.
- Chưa cấu hình thì trả `wiki_chat_not_configured`, không tự chọn model.
- Không có bảng mới và không có migration.

### UI

- Khung chat nằm trong tab Graph, rộng 380px, thay chỗ cột Outline & links khi ở tab này. Nút thu gọn cột phải sẵn có điều khiển chat.
- Dưới 950px, chat là ngăn kéo từ dưới lên, cao 55%.
- Hành vi bấm node ở scope `answer`: mở inspector tại chỗ, có nút mở trang. Ở Toàn bộ và Cục bộ không đổi.
- Gửi câu hỏi chuyển Graph sang scope `answer`. Không có kết quả thì giữ nguyên Graph.

### Thứ tự thực hiện

1. Graph theo truy vấn, không cần LLM. Đo `min_score` trên vault thật và xác nhận embedder có chạy khi truy vấn.
2. Ask loop qua SSE. Việc đầu tiên là xác nhận lấy được `LlmProvider` có tool từ `AppState` ở cả chế độ có và không có provider runtime manager. Nếu việc này đòi đổi lifecycle của runtime manager thì dừng và trao đổi.
3. Khung chat.
4. Liên kết hai chiều giữa trích dẫn và node, gợi ý câu hỏi, đo thời gian thật.

### Quyết định kiến trúc đi kèm

Thêm ADR ngắn ghi nhận chat chỉ đọc dành cho Admin và việc server mở rộng đồ thị, tham chiếu ADR 0124. Không thay đổi quyết định của ADR đó: Wikilink vẫn là nguồn duy nhất của quan hệ.

## Testing Decisions

Test tốt chỉ kiểm tra hành vi nhìn thấy từ bên ngoài: phản hồi HTTP, sự kiện SSE, nội dung vault, và DOM. Không kiểm tra cấu trúc nội bộ. Refactor bên trong không được làm vỡ test.

Có đúng hai seam đã được xác nhận với người dùng:

### Seam server: HTTP thật

Gọi `POST /api/admin/knowledge/ask` và `GET /api/admin/knowledge/graph?scope=query` qua router thật, với vault tạm và LLM Provider giả phát sẵn kịch bản sự kiện. Mẫu có sẵn: fixture server của test Knowledge dùng `router_with_state`, gồm `knowledge_graph_filters_dangling_edges_aggregates_lines_and_bounds_local_neighborhood`; test tool wiki của Voice Session cho các ca allowlist và cắt kết quả.

Các hành vi phải phủ:

- Đồ thị truy vấn không có trang hệ thống và chỉ chứa node, cạnh có thật.
- Giới hạn 60 node không cắt bằng chứng trước `context`, và láng giềng nối nhiều hạt giống được ưu tiên.
- Thiếu `q`, thiếu xác thực, thiếu cấu hình provider, hết chỗ đồng thời đều trả đúng mã.
- Sự kiện `graph` đến trước câu trả lời cuối.
- Tool ngoài allowlist, kể cả tool ghi, bị từ chối và vault không đổi.
- Kết quả tool rất dài vẫn đủ node dù phần gửi LLM bị cắt.
- Trích dẫn ngoài bằng chứng không thành `cited`.
- Ngắt kết nối dừng được provider giả; vượt số vòng kết thúc bằng `step_limit`.
- Lỗi không chứa đường dẫn vault.
- Scope Global và Local trả đúng như trước.

Giới hạn và thứ tự ưu tiên của đồ thị con cũng test qua seam này, không mở seam riêng cho Answer graph builder. Fixture được phép có helper dựng vault lớn để các ca biên không cồng kềnh.

### Seam web: trang Knowledge

Mount trang Knowledge, mock `fetch` với luồng SSE dựng sẵn và mock Cytoscape, kiểm tra DOM và `elements` truyền cho Cytoscape. Mẫu có sẵn: `KnowledgeGraph.test.ts` và `KnowledgePage.test.ts`.

Các hành vi phải phủ:

- Gửi câu hỏi, câu trả lời stream, Graph chuyển scope và lớn dần.
- Trích dẫn focus đúng node; link ngoài bằng chứng là văn bản thường.
- Bấm node ở scope `answer` không rời tab Graph; ở Toàn bộ vẫn mở trang.
- Sang tab đọc rồi quay lại vẫn còn hội thoại và đồ thị.
- Dừng, lỗi giữa chừng, xóa hội thoại, xem đồ thị của tin cũ, chưa cấu hình provider.
- Test hiện có của Jobs và Notifications vẫn qua sau khi tách SSE parser.

Mọi lệnh test và check chạy qua `scripts/agent-test.sh`, theo chính sách của repo. Chạy targeted trước toàn bộ.

### Kiểm tra tay

Không tự động hóa trong spec này, phải ghi kết quả khi hoàn tất. Trên vault thật với provider thật:

- "Trading trong wiki có gì?" ra cụm trading, không có trang hệ thống.
- "FVG liên hệ thế nào với Sharp Turn Entry?" ra hai cụm và trích dẫn đúng trang.
- "Giao dịch theo FVG là gì?" vẫn tìm ra cụm trading.
- "Công thức nấu phở?" được trả lời là không có, Graph không đổi.
- Một Source chứa chỉ dẫn xóa wiki không làm agent làm theo, wiki không đổi.
- Giao diện ở chế độ sáng, tối và 700px.

## Out of Scope

- Lưu lịch sử chat qua lần tải lại trang.
- Lưu câu trả lời thành trang synthesis.
- Link do AI suy luận.
- Tích hợp giọng nói; Assistant voice giữ nguyên `server_wiki_search`.
- Cấp tool duyệt đồ thị cho agent.
- Agent ghi vào wiki bằng bất kỳ cách nào.
- Thay đổi `llm-wiki-rs`, `orchestrator-agent`, `orchestrator-flow`.
- Tối ưu `get_graph` trước khi có số đo cho thấy cần.

## Further Notes

- Đo trên vault hiện tại (37 tài liệu, 149 cạnh) với các file chứa "trading", mở rộng 1 bước: giữ trang hệ thống ra 37 node và 149 cạnh; bỏ trang hệ thống ra 10 node và 38 cạnh. Đo bằng `grep` và bảng `graph_links`, chưa qua `wiki_query`.
- Chưa kiểm chứng: đường lấy provider ở chế độ managed, ngưỡng `min_score`, embedder lúc truy vấn, và đúng vị trí đường dẫn trong `metadata` của `wiki_query`. Ba điểm đầu đã có việc tương ứng trong thứ tự thực hiện; điểm cuối được test kết quả dài xác nhận.
- Nơi lưu setting provider là ưu tiên, chưa chốt. Người triển khai được chọn nơi khác nếu vẫn không cần migration và phải ghi lý do.
- Các giới hạn là giá trị đầu, chỉnh được sau khi đo lượt thật.
