# Hội thoại Wiki Chat lưu bền

Status: ready-for-human

Quyết định kiến trúc: [ADR 0139](../../docs/adr/0139-durable-wiki-conversations.md). Tiếp nối [spec Wiki Chat](../wiki-chat-graph/spec.md); chỉ triển khai sau khi `feat/wiki-chat-graph` đã merge.

## Problem Statement

Wiki Chat chỉ giữ hội thoại trong bộ nhớ trình duyệt. Tải lại trang là mất câu trả lời, trích dẫn và Answer Graph, trong khi đây là phần tốn công và tốn tiền LLM nhất của mỗi lượt. Admin không thể quay lại một cuộc tìm hiểu cũ hay tiếp tục nó trên trình duyệt khác.

Ngoài ra, trình duyệt đang tự gửi cả lịch sử, kể cả các tin vai `assistant`, nên server tin vào nội dung mà nó không tự tạo ra.

## Solution

Mỗi cuộc trao đổi là một Wiki Conversation lưu trong SQLite. Admin thấy danh sách hội thoại trong khung chat, mở lại được một hội thoại cũ cùng trích dẫn và Answer Graph, rồi hỏi tiếp từ đó.

Cách chạy một lượt không đổi: vẫn là SSE, vẫn chỉ đọc, các giới hạn giữ nguyên, và đóng kết nối vẫn hủy lượt. Chỉ khác ở chỗ server tự ghi lượt và tự đọc lịch sử từ DB.

## Thuật ngữ

Dùng **Wiki Conversation** (giao diện: "Hội thoại"), không dùng "session" hay "phiên". Trong repo, các từ này đã chỉ Voice Session, Agent Session và Muse session, đều có vòng đời đang chạy. Wiki Conversation chỉ là dữ liệu đã lưu, giữa các lượt không có gì chạy trên server.

## User Stories

### Giữ và mở lại

1. As an Admin, I want a conversation to survive a page reload, so that I don't lose answers I already paid for.
2. As an Admin, I want to reopen a conversation from another browser, so that my research isn't tied to one machine.
3. As an Admin, I want a list of conversations with the most recent first, so that I can find what I was working on.
4. As an Admin, I want a reopened answer to show the same citations, tool steps, and Answer Graph, so that I can review it as it was.
5. As an Admin, I want the current conversation in the URL, so that a reload brings me back to it.
6. As an Admin, I want to keep asking in a reopened conversation, so that follow-ups use the earlier context.
7. As an Admin, I want a conversation's pinned files restored from its latest question, so that I don't have to attach them again.

### Quản lý

8. As an Admin, I want a conversation titled from its first question, so that the list is readable without any effort.
9. As an Admin, I want to rename a conversation, so that I can label it in my own words.
10. As an Admin, I want to delete a conversation after confirming, so that I can remove content I no longer want stored.
11. As an Admin, I want a New conversation action that keeps the old one, so that starting a topic doesn't destroy earlier work.

### Khi có sự cố

12. As an Admin, I want a stopped answer to stay in the conversation as stopped, so that partial content isn't lost.
13. As an Admin, I want an answer cut off by a server restart marked as interrupted, so that it never looks like it is still running.
14. As an Admin, I want to retry the latest failed or interrupted answer, so that a failure doesn't force me to retype.
15. As an Admin, I want to be told when an answer could not be saved, so that I don't assume it is stored.
16. As an Admin, I want a citation to a page deleted later to say the page is gone, so that the old answer stays readable without a broken action.
17. As an Admin, I want a clear message when a conversation is already answering in another tab, so that two answers don't interleave.

### Vận hành

18. As an operator, I want stored conversations capped, so that chat history can't grow the database without bound.
19. As an operator, I want a config-only backup to exclude conversations, so that a config export never carries private questions.
20. As an operator, I want logs to keep excluding questions and page text, so that storing conversations doesn't widen where that content appears.
21. As an unauthenticated visitor, I must be refused by every conversation route, so that stored questions stay private.

## Implementation Decisions

### Modules

- **Wiki Conversation store (mới, ở server, `database/`).** Tạo, đọc, đổi tên, xóa; bắt đầu lượt; chốt lượt; đánh dấu lượt mồ côi; dựng lịch sử có giới hạn. Không biết về LLM hay HTTP.
- **Ask loop (sửa).** `run_ask` trả về một kết quả lượt thay vì tự phát sự kiện cuối ở nhiều nhánh.
- **Knowledge Ask endpoint (sửa).** Đổi hợp đồng request và ghi lượt trước khi mở SSE.
- **Conversation routes (mới).** Liệt kê, xem, đổi tên, xóa.
- **Chat state và Chat panel (sửa, ở web).** Tải và chuyển hội thoại, đồng bộ URL, bỏ phần tự dựng lịch sử.

### Dữ liệu

Migration `0142`, thêm hai bảng. Hội thoại thuộc về deployment, không có cột chủ sở hữu, vì Admin chỉ có một danh tính ([ADR 0008](../../docs/adr/0008-trusted-lan-authentication.md)).

`wiki_conversations`: `id`, `title`, `created_at_ms`, `updated_at_ms`.

`wiki_conversation_messages`:

| Cột | Ghi chú |
| --- | --- |
| `id`, `conversation_id` | Khóa ngoại `ON DELETE CASCADE` |
| `seq` | Tăng dần trong hội thoại, `UNIQUE(conversation_id, seq)` |
| `role` | `user` hoặc `assistant` |
| `content` | Câu hỏi hoặc câu trả lời |
| `content_truncated` | Câu trả lời đã bị cắt khi lưu |
| `status` | `running`, `done`, `stopped`, `failed`, `interrupted`. Tin `user` luôn là `done` |
| `error_code` | Mã công khai, không có thông điệp của engine |
| `pinned_paths_json` | Chỉ tin `user` |
| `tools_json`, `citations_json`, `graph_json` | Chỉ tin `assistant`; `graph_json` có thể `NULL` |
| `created_at_ms`, `updated_at_ms` | |

Chỉ mục duy nhất từng phần `(conversation_id) WHERE status = 'running'` bảo đảm mỗi hội thoại có tối đa một lượt đang chạy, kể cả khi mở hai tab.

Không có cột file ghim ở cấp hội thoại. File ghim hiện tại là `pinned_paths_json` của câu hỏi mới nhất, nên ghim rồi tải lại mà chưa gửi thì mất, và đây là hành vi chấp nhận được. Tiêu đề file được tra lúc đọc; trang đã mất thì dùng đường dẫn.

### Hợp đồng `POST /knowledge/ask`

Bỏ `messages`. Không giữ chế độ không trạng thái song song, để chỉ có một đường đi phải test.

```json
{ "conversation_id": "optional", "question": "…", "pinned_paths": ["wiki/a.md"] }
{ "conversation_id": "required", "retry_message_id": "…" }
```

- Phải có đúng một trong `question` và `retry_message_id`, nếu không trả `400 invalid_input`.
- Không có `conversation_id` thì server tạo hội thoại mới, tiêu đề là 80 ký tự đầu của câu hỏi.
- Thêm sự kiện SSE đầu tiên `turn`: `{ conversation_id, user_message_id, assistant_message_id }`. Id do server sinh.
- Sự kiện `done` thêm trường `saved: bool`.
- Mã lỗi mới: `wiki_conversation_not_found` (404), `wiki_conversation_busy` (409), `wiki_conversation_full` (409), `database_unavailable` (503). `wiki_chat_busy` (429) vẫn là giới hạn hai lượt toàn server.

Retry chỉ áp dụng cho tin `assistant` mới nhất của hội thoại, ở trạng thái `failed` hoặc `interrupted`; các trường hợp khác trả `400 invalid_input`. Retry ghi đè đúng dòng đó và dùng lại `pinned_paths_json` của câu hỏi liền trước. Nút thử lại chỉ hiện ở tin mới nhất. Hiện nay có thể thử lại cả tin cũ, nên đây là thay đổi hành vi có chủ ý: thử lại giữa chừng sẽ làm lịch sử mất nhất quán với các lượt sau.

### Vòng đời một lượt

Thứ tự trong endpoint, để mọi từ chối không để lại dòng nào:

1. Kiểm tra input, tìm hội thoại.
2. Kiểm tra provider đã cấu hình, lấy permit, lấy provider.
3. Trong một transaction: đổi lượt `running` quá 120 giây của hội thoại này thành `interrupted`; tạo hội thoại nếu cần; ghi tin `user` và tin `assistant` trạng thái `running`; xóa hội thoại cũ nếu vượt giới hạn.
4. Dựng lịch sử từ DB, mở SSE, phát `turn`, chạy `run_ask` trong task đã spawn.
5. Khi `run_ask` trả về, cùng task đó ghi kết quả cuối rồi mới phát `done`.

Kết quả cuối được ghi ngay cả khi lượt bị hủy. Task đã spawn không bị drop khi SSE ngắt, token hủy chỉ làm `run_ask` trả về sớm với phần đã có. Vì vậy phải bỏ các lệnh `return` rải rác trong `run_ask` và thay bằng kết quả lượt có trạng thái rõ ràng.

| Kết thúc | Trạng thái | Ghi chú |
| --- | --- | --- |
| Xong | `done` | |
| Bấm Dừng hoặc đóng kết nối | `stopped` | Không phân biệt hai trường hợp |
| Lỗi provider, `timeout`, `step_limit` | `failed` | Giữ `error_code` và phần đã stream |
| Server dừng khi đang chạy | `interrupted` | Mất phần đã stream |

Không ghi DB theo từng `delta`. Khi server sập, phần đang stream sẽ mất; đổi lại không có ghi liên tục trong lúc stream.

Khi khởi động, trước khi nhận HTTP, server đổi mọi tin `running` thành `interrupted`. Nếu ghi kết quả cuối thất bại thì thử lại tối đa 3 lần, rồi phát `done` với `saved: false`; dòng `running` còn lại được dọn ở bước 3 hoặc lần khởi động sau.

Dừng rồi gửi tiếp ngay có thể gặp `wiki_conversation_busy` trong khoảnh khắc lượt cũ chưa ghi xong. Web hiện lỗi kèm nút thử lại, không tự gửi lại.

### Lịch sử gửi cho LLM

Dựng từ DB theo đúng quy tắc của web hiện tại, rồi xóa phần trên web:

- Lấy mọi tin `user` có nội dung và các tin `assistant` trạng thái `done`, theo `seq`.
- Giữ tin mới nhất trong giới hạn 12 tin và 16 KiB; không bắt đầu bằng tin `assistant`.
- Câu hỏi hiện tại luôn được giữ; tự nó vượt 16 KiB thì trả `400 invalid_input` trước khi ghi.
- Khi retry, lịch sử chỉ gồm các tin trước tin đang thử lại.

Evidence vẫn tính theo từng lượt. Lịch sử vẫn chỉ có văn bản, không có kết quả tool.

### Giới hạn

Các giá trị ban đầu, chỉnh được sau khi đo:

| Giới hạn | Giá trị | Khi vượt |
| --- | --- | --- |
| Số hội thoại | 200 | Xóa hội thoại `updated_at_ms` cũ nhất khi tạo mới, bỏ qua hội thoại đang chạy |
| Số tin mỗi hội thoại | 100 | `409 wiki_conversation_full`, web gợi ý mở hội thoại mới |
| Câu trả lời lưu | 64 KiB | Cắt theo ranh giới ký tự, đặt `content_truncated` |
| `tools_json` | 64 KiB | Bỏ các bước cuối |
| `graph_json` | 256 KiB | Lưu `NULL`, web ẩn nút xem đồ thị |
| Tiêu đề | 1–120 ký tự | `400 invalid_input` |

Không có setting và không có xóa theo thời gian ở bản đầu.

### Conversation routes

Đều nằm dưới router Admin đã xác thực.

| Route | Kết quả |
| --- | --- |
| `GET /knowledge/conversations` | Tất cả, tối đa 200, mới nhất trước: `id`, `title`, `updated_at_ms`, `message_count`, `running`. Không phân trang vì đã có giới hạn số hội thoại |
| `GET /knowledge/conversations/{id}` | Metadata và toàn bộ tin nhắn |
| `PATCH /knowledge/conversations/{id}` | `{ title }` |
| `DELETE /knowledge/conversations/{id}` | `204`; đang chạy thì `409 wiki_conversation_busy` |

Nội dung trả về dùng `Cache-Control: no-store`.

### Answer Graph đã lưu

Lưu ảnh chụp cuối cùng, không dựng lại khi mở. Ảnh chụp phản ánh đúng bằng chứng lúc trả lời, còn wiki có thể đã đổi. Đây không phải Graph Projection thứ hai: không có gì đọc nó ngoài việc hiển thị lại, và [ADR 0124](../../docs/adr/0124-wiki-wikilinks-graph-projection.md) không đổi.

Bấm node hoặc trích dẫn trỏ tới trang đã mất thì hiện thông báo trang không còn tồn tại. Câu trả lời vẫn đọc được.

### Backup

- **Bắt buộc:** thêm hai bảng vào danh sách xóa của snapshot `ConfigOnly` trong `database/backup/codec.rs`. Danh sách này liệt kê tường minh, nên bảng mới không tự được loại ra. Nếu quên, backup "chỉ cấu hình" sẽ chứa toàn bộ câu hỏi.
- Snapshot SQLite `Full` tự có hai bảng vì dùng `VACUUM INTO`. Sau khi khôi phục, bước khởi động sẽ dọn các dòng `running`.
- Database Backup Bundle không chứa hội thoại ở bản đầu, nên inspector và importer không đổi, và chế độ replace không chạm tới hai bảng này. Ghi rõ giới hạn này trong tài liệu API.

### UI

- Nút xóa hội thoại hiện tại được thay bằng "Hội thoại mới". Dữ liệu cũ vẫn được giữ.
- Danh sách hội thoại mở từ đầu khung chat, có đổi tên và xóa kèm xác nhận. Ngăn kéo dưới 950px dùng cùng danh sách.
- URL giữ `?conversation=<id>` qua `router.replace`. Id không tồn tại thì bỏ tham số và mở trạng thái trống.
- Đang có lượt chạy thì khóa chuyển hội thoại và "Hội thoại mới"; Admin bấm Dừng trước. Chuyển đi trong im lặng sẽ hủy một câu trả lời đang tốn tiền.
- Mở hội thoại có tin `running` từ tab khác thì hiện ở chế độ chỉ đọc, kèm thông báo và nút tải lại. Không polling, không stream lại.
- Mở hội thoại thì hiển thị Answer Graph của câu trả lời mới nhất có đồ thị.
- Text mới có đủ cả hai ngôn ngữ giao diện.

### Tài liệu cập nhật khi triển khai

`docs/api/knowledge-wiki-chat.md`, `docs/flows/14-knowledge-graph-v3.md`, phần "Ngoài phạm vi" và "Server không giữ phiên" của `docs/knowledge/wiki-chat-graph-design.md`, trạng thái ADR 0139 và ghi chú sửa đổi trên ADR 0138.

## Testing Decisions

Test chỉ kiểm tra hành vi nhìn thấy từ bên ngoài. Dùng lại hai seam của spec Wiki Chat; fixture server đổi từ `database = None` sang DB tạm đã chạy migration.

### Seam server: HTTP thật

Router thật, vault tạm, DB tạm, LLM giả theo kịch bản.

- Hỏi không có `conversation_id` tạo hội thoại; `turn` là sự kiện đầu tiên; sau `done`, `GET` trả đúng câu trả lời, trích dẫn, bước tool và đồ thị.
- Lịch sử LLM nhận được đến từ DB: đúng giới hạn 12 tin / 16 KiB, không có tin `assistant` chưa `done`.
- Body có `messages` bị từ chối.
- Ngắt kết nối giữa chừng: tin thành `stopped`, giữ phần đã stream, provider giả bị dừng.
- Lỗi provider, `timeout`, `step_limit`: tin thành `failed` với đúng mã.
- Thiếu cấu hình, hết permit, input sai: không tạo dòng nào.
- Hai lượt cùng hội thoại: lượt thứ hai nhận `409 wiki_conversation_busy`. Hai hội thoại khác nhau vẫn chạy song song.
- Retry ghi đè đúng dòng; retry tin cũ hoặc tin `done` trả `400`.
- Bước khởi động đổi `running` thành `interrupted`; `running` quá 120 giây không chặn lượt mới.
- Vượt 200 hội thoại xóa cái cũ nhất nhưng không xóa cái đang chạy; vượt 100 tin trả `409`.
- Cắt câu trả lời dài không vỡ UTF-8; đồ thị quá lớn lưu `NULL` mà lượt vẫn `done`.
- Đổi tên, xóa có cascade, xóa khi đang chạy, id không tồn tại, thiếu xác thực.
- Lỗi và log không chứa câu hỏi, nội dung trang hay đường dẫn tuyệt đối.

### Seam backup

Snapshot `ConfigOnly` không còn dòng nào trong hai bảng; snapshot `Full` còn đủ. Thêm vào test có sẵn tại `database/backup/tests.rs`.

### Seam web: trang Knowledge

Mount trang Knowledge, mock `fetch` và SSE.

- Tải trang với `?conversation=` khôi phục tin nhắn, trích dẫn, đồ thị và file ghim.
- Gửi câu đầu cập nhật URL theo sự kiện `turn`; gửi tiếp dùng đúng `conversation_id` và không gửi lịch sử.
- Đổi hội thoại, hội thoại mới, đổi tên, xóa có xác nhận; bị khóa khi đang chạy.
- Id không tồn tại, tin `running` từ tab khác, `saved: false`, `wiki_conversation_busy`, `wiki_conversation_full`.
- Nút thử lại chỉ có ở tin mới nhất.

Mọi lệnh test và check chạy qua `scripts/agent-test.sh`, targeted trước rồi mới toàn bộ.

### Kiểm tra tay

Trên vault thật với provider thật, ghi kết quả khi hoàn tất; chưa chạy thì để `NOT_RUN`.

- Hỏi, tải lại trang, hỏi tiếp câu nối tiếp và kiểm tra ngữ cảnh còn đúng.
- Đóng tab giữa câu trả lời rồi mở lại.
- Tắt server giữa câu trả lời rồi khởi động lại.
- Xóa một trang đã được trích dẫn rồi mở lại câu trả lời.
- Đo kích thước `graph_json` và `GET` chi tiết của một hội thoại dài.

## Out of Scope

- Chạy tiếp câu trả lời khi đóng tab, hoặc nối lại stream.
- Mang Evidence sang lượt sau.
- Tóm tắt hội thoại, bộ nhớ dài hạn, nới giới hạn lịch sử.
- Tiêu đề do LLM sinh, tìm kiếm hội thoại, ghim hay gom nhóm hội thoại.
- Chia sẻ hoặc xuất hội thoại, lưu câu trả lời thành trang synthesis.
- Hội thoại theo từng người dùng.
- Đưa hội thoại vào Database Backup Bundle.
- Chuyển lịch sử đang có trong trình duyệt sang DB. Nó vốn không tồn tại quá một lần tải trang.

## Further Notes

- Chưa kiểm chứng: kích thước thật của `graph_json`. Trường hợp xấu nhất theo giới hạn là khoảng 3 GB (200 hội thoại × 50 câu trả lời × 320 KiB), còn thực tế nên nhỏ hơn nhiều. Nếu số đo cho thấy `GET` chi tiết quá nặng thì mới tách đồ thị thành route riêng.
- Đã kiểm tra trong mã nguồn: route xóa ghi `admin_audit_events` với `resource_type=wiki_conversation`, `resource_id` và title để trống; nội dung câu hỏi/câu trả lời không được thêm vào audit.
- Mốc 120 giây là thời gian tối đa một lượt (90 giây) cộng phần dư.

## Implementation acceptance record (2026-11-03)

Tickets 01–06 đã triển khai và các kiểm tra tự động liên quan đã chạy qua `scripts/agent-test.sh`. Tài liệu triển khai đã được cập nhật. Ticket 07 vẫn `ready-for-human`: các kịch bản vault/provider thật và phép đo kích thước thực tế bên trên đều `NOT_RUN`, nên spec chưa được đánh dấu hoàn tất.
