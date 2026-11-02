# Muse Task: giao việc cloud qua Sub-Agent với phiên bền vững

Status: ready-for-agent

Ngày soạn và xuất bản: 2026-11-01. Người dùng đã xác nhận seam kiểm thử theo skill `to-spec`. Spec sẵn sàng giao cho agent; các gate kiểm chứng Muse thật vẫn phải được thực hiện theo nội dung dưới đây.

## Problem Statement

Người dùng muốn giao một việc dài như tìm hiểu, tổng hợp và viết báo cáo cho Muse, rồi nhận message và file trong Background Task. Đường Muse chat và media hiện có chưa cung cấp một Host Tool tổng quát duy trì ngữ cảnh qua nhiều lượt trong cùng Sub-Agent, thu hồi tài liệu không phải media, phục hồi sau ngắt kết nối hoặc restart mà không gửi trùng việc, và tự đóng phiên khi Sub-Agent hoàn tất.

Timeout, history còn thuộc lượt trước, mất ACK và các đường dọn session hiện tại có thể khiến hệ thống mất việc, trả nhầm kết quả hoặc tạo lại thao tác từ xa. Người vận hành cũng chưa có bằng chứng Muse thật hỗ trợ đầy đủ hành vi nghiên cứu, độ bền session và thu hồi file được đề xuất.

## Solution

Thêm Host Tool `muse_task` dành riêng cho Sub-Agent `cloud_worker` của `daily_root`. Mỗi Agent Session của Sub-Agent gắn đúng một Job Session trên Muse và một Gateway Account xuyên suốt các lần `send`, `collect`, retry và restart. Hai Sub-Agent dùng hai session riêng.

Sub-Agent gửi nhiều lượt và lấy đủ kết quả trước khi kết thúc. Tool chờ theo lát, nối lại công việc đã gửi và xuất message cùng Job Artifact. Khi Sub-Agent hoàn tất hoặc không còn khả năng tiếp tục, hệ thống archive Agent Session rồi xóa Muse session có xác nhận. Cha vẫn đọc được Job Artifact đã lưu; muốn hỏi tiếp sau khi đóng phải tạo Sub-Agent mới và truyền lại ngữ cảnh cần thiết.

Tính năng mặc định tắt, Admin chủ động bật và chọn Muse provider. Kiểm thử xác định trên VM giả và kiểm chứng Muse thật là hai gate riêng; test giả thành công chưa đủ để tuyên bố dùng được với Muse thật.

## User Stories

1. As an người dùng Background Task, I want giao việc nghiên cứu cho `cloud_worker`, so that tôi nhận được nội dung tổng hợp và báo cáo từ Muse.
2. As an Root Orchestrator, I want khám phá `cloud_worker` khi Admin đã bật, so that tôi chỉ giao việc cho năng lực đang được cấp.
3. As an Admin, I want tính năng tắt mặc định, so that tôi chủ động quyết định việc sử dụng dịch vụ bên thứ ba.
4. As an Admin, I want chọn Muse provider bằng cấu hình tool provider hiện có, so that tôi không phải quản lý cơ chế cấu hình riêng.
5. As an Sub-Agent, I want nhiều lần gọi tool dùng cùng Muse session, so that Muse giữ được ngữ cảnh trong nhiệm vụ.
6. As an người dùng, I want hai Sub-Agent có Muse session riêng, so that nhiệm vụ không lẫn hội thoại.
7. As an người vận hành, I want root bị từ chối khi gọi `muse_task`, so that vòng đời phiên luôn bám đúng Sub-Agent.
8. As an Sub-Agent, I want `send` tạo lượt mới khi lượt cũ đã xong, so that tôi có thể yêu cầu Muse bổ sung hoặc sửa kết quả.
9. As an Sub-Agent, I want `collect` chỉ lấy kết quả, so that tôi không vô tình gửi lại yêu cầu.
10. As an Sub-Agent, I want nhận `MUSE_TASK_BUSY` và `next_action=collect` khi lượt trước còn chạy, so that tôi tiếp tục đúng việc đang có.
11. As an Sub-Agent, I want biết rõ chưa có session khi gọi `collect` trước `send`, so that tôi sửa được trình tự thao tác.
12. As an người dùng, I want lượt dài tiếp tục qua nhiều lát chờ, so that timeout của runtime không làm mất việc đã gửi.
13. As an người vận hành, I want Waiting Retry nhả slot và giữ invocation, so that việc chờ không chiếm worker và không tạo yêu cầu mới.
14. As an người dùng, I want server restart vẫn nối lại đúng session, so that tôi không phải bắt đầu lại nhiệm vụ.
15. As an người vận hành, I want mất ACK được đối soát trước khi gửi tiếp, so that Muse không nhận hai lần cùng một việc.
16. As an người dùng, I want WebSocket đứt được nối lại trong lúc chờ, so that tool không im lặng tới hết hạn.
17. As an người vận hành, I want lỗi tạm thời của một session không đóng socket dùng chung, so that các nhiệm vụ khác trên tài khoản tiếp tục chạy.
18. As an Sub-Agent, I want được báo `MUSE_SESSION_LOST` khi session mất, so that tôi báo đúng việc mất ngữ cảnh cho cha.
19. As an người vận hành, I want phiên đã ghim không failover tài khoản, so that phục hồi luôn truy cập đúng VM chứa công việc.
20. As an Sub-Agent, I want đặt thời hạn trong khoảng hỗ trợ, so that tôi kiểm soát thời gian chờ mà vẫn có giới hạn rõ ràng.
21. As an Sub-Agent, I want nhận phần text đã có khi timeout hoặc stalled, so that tôi có thể quyết định thu hồi tiếp hoặc báo kết quả dở dang.
22. As an người dùng, I want kết quả luôn thuộc lượt vừa gửi, so that tôi không nhận lại báo cáo cũ khi history cập nhật chậm.
23. As an Sub-Agent, I want commentary và tool đang chạy chưa được coi là hoàn tất, so that tôi không kết thúc nhiệm vụ quá sớm.
24. As an người dùng, I want nhận câu trả lời dạng message, so that tôi có thể đọc trực tiếp kết quả ngắn.
25. As an người dùng, I want `output=markdown` luôn tạo Job Artifact `.md` khi thành công, so that tôi có báo cáo để lưu và tải xuống.
26. As an người dùng, I want biết Markdown do Muse tạo hay được lưu từ message, so that tôi hiểu nguồn gốc báo cáo.
27. As an người dùng, I want nhận tài liệu và bảng dữ liệu ngoài ảnh, video, so that Muse có thể bàn giao đầy đủ sản phẩm công việc.
28. As an Sub-Agent, I want chế độ `files` nhắc đính kèm tối đa một lần và báo thiếu file, so that tôi không nhầm text thành file đã bàn giao.
29. As an Root Orchestrator, I want ID artifact xuất hiện trong nội dung tool, so that tôi có thể dùng kết quả để tổng hợp cho người dùng.
30. As an người vận hành, I want message lớn được lưu thành artifact, so that kết quả không vượt giới hạn đầu vào của model.
31. As an người dùng, I want artifact có tên, MIME, kích thước và checksum, so that tôi có thể kiểm tra file nhận được.
32. As an người vận hành, I want giới hạn kích thước và chuẩn hóa tên file, so that dữ liệu từ Muse được nhập qua Artifact Store có kiểm soát.
33. As an Root Orchestrator, I want đọc được artifact sau khi con đã archive, so that tôi hoàn thành phần tổng hợp cuối cùng.
34. As an người vận hành, I want Sub-Agent xong được archive rồi đóng Muse session trong chu kỳ khoảng 10 giây, so that phiên và công việc từ xa không bị bỏ quên.
35. As an người vận hành, I want chỉ ghi nhận xóa sau ACK, so that cleanup thất bại được thử lại.
36. As an Sub-Agent còn có thể phục hồi, I want phiên được giữ trong waiting, interrupted hoặc failed còn retry được, so that tôi có thể tiếp tục công việc.
37. As an người vận hành, I want phiên bị dọn khi root không còn việc đang hoạt động, so that trạng thái có thể phục hồi không giữ tài nguyên vô hạn.
38. As an Root Orchestrator, I want gọi lại con đã archive nhận `SESSION_ARCHIVED`, so that tôi biết phải tạo con mới cho phần hỏi thêm.
39. As an người dùng Task Follow-up, I want phần hỏi thêm dùng Sub-Agent mới với ngữ cảnh được cha cung cấp, so that vòng đời dùng một lần được thể hiện rõ.
40. As an người dùng video, I want `video_create` và `muse_task` không dùng chung session hoặc cleanup, so that tính năng mới không ảnh hưởng luồng video.
41. As an Admin, I want prompt cảnh báo việc gửi dữ liệu ra ngoài và không tự đính kèm Memory hay Wiki, so that tôi kiểm soát nội dung chuyển sang Muse.
42. As an người vận hành, I want kết quả Muse được coi là dữ liệu ngoài, so that nội dung web không trở thành quyền thực thi chỉ dẫn cho Sub-Agent.
43. As an người vận hành, I want approval không được phép bị từ chối rõ ràng, so that nhiệm vụ không treo tới hết hạn.
44. As an Admin, I want giới hạn mặc định hai lượt tool đồng thời mỗi tài khoản, so that tải và quota có giới hạn.
45. As an Root Orchestrator, I want biết không giao việc dài này cho Task nguồn Voice, so that ngân sách ngắn không làm việc bị dừng ngoài dự kiến.
46. As an người vận hành, I want phân biệt PASS, FAIL và NOT_RUN của kiểm chứng thật, so that tôi đánh giá đúng điều kiện triển khai.

## Implementation Decisions

- Triển khai trong server; dùng nguyên các API công khai của `orchestrator-agent` 0.6.0 và `orchestrator-flow` 0.3.0. Không sửa thư viện, thêm bảng hay migration cho tính năng này.
- Bốn phần trách nhiệm: `MuseTaskTool` quản lý tham số, journal và artifact; `AgentTaskProvider` che adapter gateway; workflow `agent_task` quản lý lượt Muse, reconnect và file; `SubagentMuseCloser` sở hữu vòng đời đóng phiên. Tracker, trích file và dựng prompt là logic thuần.
- Thêm port `AgentTaskProvider` qua accessor tùy chọn của `LlmProvider`, theo mô hình `DurableMediaProvider`. Gateway Muse là adapter thật; provider giả phục vụ kiểm thử tool. Port nhận action, Continuation Key, Tool Operation, lát chờ, journal và callback tiến độ; trả lượt, message, file và cờ thiếu file.
- `muse_task` khai báo `ToolEffect::Resumable`, chỉ chấp nhận scope Sub-Agent; root nhận `MUSE_TASK_SUBAGENT_ONLY` trước khi tạo tác dụng từ xa.
- Hợp đồng đầu vào gồm `action=send|collect` mặc định `send`, `prompt` cho lượt gửi, `output=message|markdown|files` mặc định `message`, `filename` tùy chọn và `max_wait_secs` mặc định 3600. Sai tham số hoặc thời hạn ngoài 60–7200 giây trả `INVALID_ARGUMENT`, không tự kẹp giá trị.
- Không có action `close`, `reset`, `start`, `status` hoặc tham số chọn conversation. `send` khi lượt trước chưa xong trả `MUSE_TASK_BUSY` cùng `next_action=collect`; `collect` trước khi có phiên trả `MUSE_TASK_NO_SESSION`.
- Continuation Key là `muse_task:<Agent Session ID của Sub-Agent>`, ghi đè key root do runtime khởi tạo. Không dùng chung key với `video_create`. Gate của registry tuần tự hóa lượt cùng session; phiên rảnh không chiếm `active_ops`.
- Mỗi invocation có Tool Operation riêng, chứa owner, workspace, Agent Session, pending turn, baseline lượt user, account và remote session. Ghi account cùng remote session trước POST đầu tiên, cập nhật ID thật nếu Muse cấp lại. Mọi POST, kể cả nhắc đính kèm, phải có checkpoint trước khi gửi.
- Ghim Gateway Account xuyên vòng đời session; không failover sau khi có việc trên remote. Tạm không dùng được hoặc quota sau khi gửi trả `TOOL_RETRY_LATER`; lỗi trước khi gửi trả `MUSE_TASK_REJECTED`; cần xác thực lại trả `PROVIDER_AUTH_FAILED`.
- `Submitting` có baseline được đối soát qua số lượt user: lớn hơn baseline thì tiếp tục chờ, bằng baseline mới được gửi pending turn; không xác minh được trả `TOOL_OUTCOME_UNKNOWN`. Mất ACK hoặc lỗi journal không chứng minh POST bị từ chối; phải giữ operation/session để đối soát.
- `Running` chỉ chờ, `Downloading` chỉ tải lại; không gửi lại prompt. `Committed` trả kết quả đã lưu mà không gọi provider. Tool xử lý trạng thái kết thúc trước khi gọi port. Lỗi mơ hồ khi operation còn mở tuân thủ ADR 0134.
- Timeout runtime là 1800 giây, lát chờ nội bộ 1700 giây; lưu trần tuyệt đối vào `deadline_at` và giữ qua retry/restart. Không có tín hiệu 600 giây trả `MUSE_TASK_STALLED`. Quá trần trả `MUSE_TASK_TIMEOUT` cùng partial text nếu có và `next_action=collect`; hết lát khi còn chạy trả `TOOL_RETRY_LATER`.
- `collect` sau timeout là invocation thu hồi riêng, vẫn bám lượt gần nhất và session cũ; không reset deadline của invocation cũ hay POST lại prompt. Nó chỉ hữu ích khi Sub-Agent chưa kết thúc và Job Budget còn cho phép.
- Chờ slot tối đa 60 giây. Ngắt WebSocket nối lại cùng account/session với backoff 1, 2, 5, 10, 30 giây rồi đọc history; sau 300 giây chưa nối được trả `TOOL_RETRY_LATER`. Dùng sự kiện và history theo nhịp 2, 5, 15 giây thay cho polling 400 ms.
- Tín hiệu chống stalled gồm delta, `agent.status`, `task.*` và history thực sự thay đổi. Không gọi `fail_account` cho lỗi Transient vì hàm này đóng socket dùng chung. AuthInvalid và QuotaExceeded vẫn báo pool, nhưng lease đã retain phải giữ session.
- Không kiểm tra history của session vừa tạo như session đã tồn tại. Thêm `completed_turns` vào registry trong bộ nhớ, lưu lượt đã xong qua `batch_index`, restore bằng giá trị lớn nhất cùng key. Với phiên từng có lượt hoàn tất, history 404 hoặc ít lượt user hơn số đã hoàn tất trả `MUSE_SESSION_LOST`; không tự tạo session thay thế.
- Hoàn tất phải đúng lượt user `baseline + 1` và không còn tool đang chạy. Marker `[MUSE_TASK_DONE]` là tín hiệu nhanh; thiếu marker cần session và assistant đều completed, ổn định 5 giây. Commentary không tính là kết quả cuối. Chọn điều kiện chặt này để thống nhất phần mô tả và các test bắt buộc trong tài liệu nguồn.
- Chỉ phân loại lỗi assistant trên message ngắn không kèm file; báo cáo nhắc tới rate limit không tự động là lỗi provider. Kết quả bỏ commentary và marker giao thức.
- Prompt yêu cầu làm việc trong thư mục riêng `tasks/<Agent Session ID>/`, ghi đầu ra tại đó và chỉ ra đường dẫn sandbox. Trích đường dẫn từ presentation, turn events, bubble text và fallback text, bỏ trùng. Với Markdown hoặc filename cụ thể, thử đọc đường dẫn hợp đồng trước. Giữ nguyên bộ trích media của `video_create`.
- Chấp nhận đuôi `.md`, `.txt`, `.pdf`, `.csv`, `.json`, `.html`, `.docx`, `.xlsx`, `.png`, `.jpg`, `.webp`, `.mp4`. Mỗi file tuân thủ `artifact_max_bytes` hiện tại 128 MiB; chuẩn hóa tên, không dùng đường dẫn do Muse đưa để ghi trực tiếp xuống filesystem local.
- `message` trả text và lưu file nếu có. `markdown` dùng file Markdown phù hợp của Muse hoặc lưu message thành `.md` khi không có Markdown, ghi rõ `origin=muse_file|message`. `files` thiếu đính kèm chỉ nhắc một lần có journal chống lặp; vẫn thiếu thì trả `status=partial` và mô tả rõ.
- Lưu file qua Artifact Store của ToolContext, bảo toàn scope. Nội dung tool chứa `status=completed|partial`, turn, message, artifact IDs, thông tin file và `next_action=none|collect`; metadata chứa `artifact_ids` để runtime kiểm tra quyền. File có filename, MIME, size, SHA-256 và origin.
- Message vượt `tool_output_cap_bytes`, tính từ read budget và tối thiểu 24 KiB, chuyển thành artifact; content giữ đoạn đầu và ID. Sau khi lưu artifact, commit cả artifact IDs, result và số lượt. Crash giữa put và commit có thể tạo artifact thừa; chỉ công bố kết quả cuối, như hành vi video hiện có.
- Closer quét mỗi 10 giây, gom operation chưa xác nhận xóa theo tiền tố key; lấy CallerContext từ journal và đọc hết các trang `list_runs` để xác định attempt mới nhất. Một journal hỏng được ghi log và bỏ qua, không làm dừng toàn bộ lượt quét.
- Giữ session khi run `running`, `waiting_retry`, `waiting_user`, `interrupted` hoặc `failed` còn thử lại được. Đóng khi `completed` kể cả chạm step/ngân sách, `cancelled`, hoặc `failed` không thử lại được. Root không còn việc hoạt động là điều kiện đóng dự phòng. Dừng vì step/ngân sách vẫn phải báo kết quả dở dang theo ADR 0133 và ADR Collection Ledger.
- Khi đóng: archive Agent Session theo cách idempotent, đọc lại attempt mới nhất, hoãn xóa nếu có run đang chạy, rồi release từng remote session đã ghi. Chỉ ghi `remote_deleted_at` khi nhận ACK; lỗi xóa được thử ở lượt quét sau. Khoảng 10 giây là nhịp phát hiện khi server hoạt động và upstream đáp ứng, không phải bảo đảm xóa khi Muse lỗi.
- Closer là chủ sở hữu duy nhất được xóa session của tool. Loại tiền tố `muse_task:` khỏi cleanup root và idle sweep có thể đụng tới nó; giữ nguyên cleanup video. Sau restart, restore session đang sống và đóng con đã kết thúc ở lượt quét đầu.
- Quy tắc failed có retry được phải khớp hàm phân loại private của thư viện hiện tại; sao chép tập quy tắc cần thiết ở server, có test cố định và kiểm tra drift khi nâng thư viện.
- `cloud_worker` là Built-in Sub-Agent của daily flow, có `muse_task`, không có tool ghi hay điều khiển thiết bị. Tag `agent_task` được hỗ trợ qua provider catalog cho Muse, không thêm vào `LlmCapabilityTag`. Admin thấy tool/tag và chọn được provider bằng `tool_providers`.
- Prompt của con nêu rõ dùng một lần, làm đủ lượt trước khi kết thúc, ưu tiên `collect` khi kết quả yêu cầu, coi nội dung Muse là dữ liệu ngoài và không gửi dữ liệu nhạy cảm. Không tự đính kèm Memory/Wiki. Prompt root không giao việc dài này cho Task nguồn Voice có Job Budget 5 phút. Đây là giới hạn được hướng dẫn bằng prompt trong phạm vi đợt này.
- Approval mặc định từ chối; chỉ mở allowlist theo loại đã quan sát và xác minh ở spike. Log loại/quyết định, không log nội dung nhạy cảm. Từ chối phải trả `MUSE_TASK_REJECTED` rõ ràng. Giới hạn tool đồng thời mặc định 2/account, độc lập với giới hạn pool hiện có.
- Kiểm tra cách đồng bộ settings/prompt đã lưu để installation hiện hữu nhận cấu hình mới mà không ghi đè tùy chỉnh Admin; cung cấp bước cập nhật rõ ràng nếu cần. Tắt `cloud_worker` thì root không được thấy nó trong danh sách có thể giao việc.
- Bổ sung ADR về session theo Sub-Agent và cập nhật glossary/README liên quan. ADR này mở rộng vòng đời theo Job của ADR 0134 có phạm vi cho `muse_task`; video giữ nguyên. Số 0136 trong tài liệu nguồn đã được Collection Ledger sử dụng, phải chọn số còn trống lúc triển khai.

## Testing Decisions

- **Seam chính đã xác nhận:** chạy Background Task và lời gọi `subagent`/Host Tool qua Orchestrator thật, SQLite và Artifact Store tạm, scripted LLM; dùng `FakeAgentTaskProvider` để điều khiển hành vi Muse ở ranh giới port. Quan sát kết quả cha/model nhận được, mã lỗi, trạng thái bền, số lượt gửi và artifact đọc được; không dựng ToolContext private hoặc kiểm tra chi tiết hàm nội bộ.
- **Seam bổ trợ có sẵn:** gọi gateway Muse qua `MuseVmTransport` với VM giả để kiểm tra POST/history/file, giữ session, reconnect và số request. Chỉ thêm port `AgentTaskProvider`; không tạo thêm seam cho từng helper. Logic thuần nhận thời gian làm đầu vào; test thời gian I/O dùng đồng hồ ảo Tokio.
- Prior art: bộ test Task–Memory dùng TaskManager/Orchestrator thật và scripted Responses model; bộ Muse chat/media/durability dùng `build_gateway_muse_provider`, fake account, bộ đếm stream và deleted sessions. Tận dụng các mẫu này cho tool, workflow và closer.
- H1–H2: hai send trong cùng Sub-Agent dùng một session; hai Sub-Agent dùng hai session; root bị từ chối trước I/O; collect chưa có session báo đúng; tham số không hợp lệ không tạo operation từ xa.
- H4: message, Markdown từ file và từ text; chế độ files thiếu file có đúng một follow-up qua restart; tên/MIME/size/hash/origin đúng; message lớn thành artifact; content có ID; vượt giới hạn file trả lỗi rõ; chỉ nhận đuôi cho phép và không lấy trùng file.
- H5: phục hồi ở submitting trước POST, sau POST mất ACK, running, downloading và committed. Kiểm số POST và session để chứng minh không gửi trùng; journal không đọc/ghi được không bị biến thành từ chối chắc chắn. Committed replay không gọi provider; running/download không gửi prompt mới.
- Waiting Retry: hết lát hoặc lỗi tạm thời sau khi gửi không settle invocation, run chuyển `waiting_retry`, nhả worker; attempt sau dùng cùng invocation/account/session và deadline. Hủy trả `CANCELLED`; shutdown giữ operation mở và session để phục hồi. Job Budget không reset qua retry/restart.
- H6: ngắt socket giữa chờ rồi nối lại đọc đúng kết quả; hết 300 giây reconnect trả retry sớm hơn deadline; Transient không xóa session hoặc ngắt các session khác; chờ slot có hạn; quota/auth không failover session đã ghim.
- H7: phiên mới chưa có history không bị nhận nhầm là mất; phiên đã có lượt hoàn tất bị 404 hoặc history mất lượt trả `MUSE_SESSION_LOST`; restore lấy completed_turns lớn nhất và không tạo phiên thay thế.
- Tracker: history cũ, thiếu lượt user, commentary, marker khi tool còn chạy đều chưa hoàn tất; nhánh không marker phải ổn định đủ 5 giây; từng loại tín hiệu cập nhật chống stalled; phân biệt hết lát, hết trần, stalled và kết quả hoàn tất. Timeout/stalled giữ partial text và hướng dẫn phù hợp.
- H3: test từng trạng thái trong bảng đóng phiên, cả root hết việc; archive trước xóa; đọc lại run chặn race; phân trang lấy đúng attempt mới nhất; xóa thất bại chưa ghi ACK và được thử lại; journal hỏng không cản key khác; tất cả remote session đã ghi đều được dọn.
- Test tích hợp closer: con xong được archive/xóa trong một lượt quét; gọi lại con trả `SESSION_ARCHIVED`; cha vẫn đọc artifact. Khôi phục kho sau restart khi con đang chạy dùng session cũ, khi con đã xong thì đóng ở lượt quét đầu. Cùng root có video và Muse Task vẫn tách session và giữ hành vi cleanup video.
- Admin/catalog: mặc định tắt; bật và chọn Muse provider được; provider khác không nhận tag; tắt thì root không thấy con; settings cũ có đường cập nhật rõ; prompt phản ánh use-once, collect, dữ liệu ngoài và giới hạn Voice. Approval bị từ chối không treo.
- Mọi test/check chạy qua wrapper kiểm thử của repo; test mục tiêu trước workspace, log đầy đủ lưu riêng, chỉ đọc phần liên quan khi lỗi và không bỏ qua exit code. Các thư viện không thay đổi nên không phát sinh nghĩa vụ sửa/test ở repo thư viện.
- **Spike giai đoạn 0, gate trước khi cam kết hành vi phụ thuộc Muse:** chạy runbook độ bền hiện có và đo tìm kiếm web/viết báo cáo, tiếp tục sau đứt socket, session ID thực tế, thời lượng nghiên cứu, loại approval, vị trí file tài liệu trong history, khả năng đọc history theo mốc, và tác dụng delete đối với việc đang chạy/file/quota. Các phép đo chưa chạy phải giữ `NOT_RUN`.
- **Kiểm chứng thật cuối:** giao việc tìm hiểu–tóm tắt–viết báo cáo, hai send trong cùng con, kill/restart giữa chừng, kiểm ID session không đổi, không có POST trùng, nhận artifact đúng checksum và không còn session của nhiệm vụ sau khi đóng. Cần tài khoản thật và tiêu quota; ghi PASS/FAIL/NOT_RUN cùng bằng chứng đã loại bí mật, tách khỏi test giả.

## Out of Scope

- Viết code triển khai trong lượt tạo spec; phân rã thành ticket thực thi hoặc commit/push không thuộc yêu cầu này.
- Hội thoại Muse xuyên nhiều Sub-Agent hoặc tiếp tục Agent Session con sau archive; action đóng/reset do model điều khiển.
- API không chặn kiểu start/status/result và hủy riêng một lượt mà giữ session; endpoint hủy chưa được kiểm chứng.
- Provider ngoài Muse; thay đổi thư viện, schema hoặc thêm mã đỗ polling riêng trong thư viện trong đợt hiện tại.
- Tự động gửi Memory/Wiki, cho Muse cấp quyền công cụ cục bộ hoặc cấp tool ghi/thiết bị cho `cloud_worker`.
- Bảo đảm việc dài trên Task nguồn Voice bằng cơ chế ngân sách mới; tăng hạn mức Job Budget hoặc né ngân sách đang có.
- Thay đổi thuật toán trích media, timeout hay vòng đời video ngoài phần cô lập key/cleanup cần thiết.
- Tuyên bố đã kiểm chứng nghiên cứu web, sống qua restart hoặc dọn VM chỉ từ kết quả VM giả.

## Further Notes

- Nguồn chính: [thiết kế](../../docs/muse-task-tool-design.md) và [triển khai](../../docs/muse-task-tool-implementation.md). Đối chiếu ADR 0129, 0130, 0132, 0133, 0134 cùng ADR Collection Ledger hiện tại, domain context và glossary của repo.
- H1–H7 trong tài liệu triển khai là các hành vi nghiệm thu bắt buộc; phần Testing Decisions nối từng hành vi với seam và tình huống kiểm thử.
- Thứ tự công việc: spike; tracker/file paths/prompt/closure thuần; workflow message và reconnect; file; Host Tool; closer/restore; catalog/Admin/prompt; ADR/glossary; kiểm chứng thật. Viết test trước cho từng phần và commit theo bước sau khi gate tương ứng đạt.
- Có thể xây và kiểm thử xác định trên VM giả trước spike, nhưng nên chạy spike trước workflow I/O. Nếu Muse không tự tìm kiếm web, dừng hoặc trình lại phạm vi chỉ tóm tắt đầu vào. Nếu session không sống qua ngắt kết nối, không giữ lời hứa phục hồi chưa chứng minh; báo `MUSE_SESSION_LOST` và ghi rõ giới hạn.
- Nếu delete không dọn thư mục/không dừng công việc thì bổ sung dọn thư mục theo kết quả spike và ghi rõ quota còn tiêu. Nếu việc thường dài hơn 30 phút, cần xem lại cơ chế đỗ riêng ngoài phạm vi thư viện hiện tại. Backoff Waiting Retry hiện có được chấp nhận ở bản đầu; đối chiếu chính sách retry thật khi triển khai vì ADR Collection Ledger đã thay đổi cách đếm attempt không tiến độ.
- Các điểm đã làm rõ khi tổng hợp: marker không thắng điều kiện tool đang chạy; Markdown phải có artifact Markdown dù Muse chỉ cung cấp file định dạng khác; nhịp closer 10 giây không cam kết ACK trong lúc upstream lỗi; ADR mới không dùng lại số 0136.
- Chưa chạy test hay spike trong lượt soạn tài liệu. Các trạng thái kiểm chứng lịch sử không được nâng thành bằng chứng cho tính năng mới.

## Comments

- 2026-11-01: Soạn bản nháp từ hai tài liệu và code hiện tại. Chờ người dùng xác nhận seam chính Orchestrator thật + SQLite/Artifact Store tạm + provider giả, cùng seam transport VM giả cho kiểm chứng giao thức. Sau xác nhận mới chuyển tên chuẩn và gắn `ready-for-agent` theo skill `to-spec`.
- 2026-11-01: Người dùng xác nhận cả seam chính và seam bổ trợ, yêu cầu xuất bản với `ready-for-agent`. Chuyển bản nháp thành spec chính thức trong issue tracker local.
