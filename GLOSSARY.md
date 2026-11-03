# Voice Agent Conversation Domain

Các khái niệm domain cho hội thoại qua Voice Session, thực thi công cụ, điều phối tác vụ nền (`orchestrator-flow` / `orchestrator-agent`) và kho tri thức LLM Wiki (`llm-wiki-rs`).

## Conversation

**Conversational Turn**:
Một lượt tương tác được server chấp nhận, có thể gồm nhận dạng lời nói, phản hồi ngôn ngữ và các lần gọi tool; kết thúc độc lập với từng Tool Call.
_Avoid_: tool turn, tool request

**Tool Round**:
Tập các Tool Call mà LLM tạo trong một lần sinh phản hồi; các call được thực thi tuần tự trong cùng Conversational Turn.
_Avoid_: tool batch song song

## Tool Execution

**Tool Call**:
Yêu cầu thực hiện một công cụ do LLM chọn trong Tool Round, chỉ được chạy khi executor cho phép.
_Avoid_: tool intent as execution

**Tool Result**:
Kết quả đã hoàn tất của một Tool Call, được dùng để tiếp tục xử lý hội thoại; lỗi ở cấp tool không đồng nghĩa lượt hội thoại thất bại.
_Avoid_: tool status, turn outcome

**Tool Execution Status**:
Tín hiệu quan sát tạm thời báo một Tool Call đã bắt đầu, hoàn tất thành công hoặc thất bại; không phải Tool Result và không quyết định kết quả Conversational Turn.
_Avoid_: MCP response, assistant message, turn status

**System MCP**:
Tập công cụ quản trị do deployment cung cấp cho Studio Assistant hoặc chủ thể remote đã xác thực, với quyền riêng cho đọc dữ liệu, khám phá công cụ và đề xuất thay đổi.
_Avoid_: Device MCP, External MCP server, Admin tool authority

**System Tool Grant**:
Quyền cho Studio Assistant sử dụng một nhóm công cụ System MCP; độc lập với quyền gọi công cụ của External MCP và không được cấp bởi nội dung hội thoại.
_Avoid_: Agent–External MCP binding, tool execution status

**System Change Request**:
Đề xuất cố định để thay đổi MCP Server, binding của Agent hoặc quyền công cụ đã review; chỉ thao tác xác nhận riêng của Admin đã xác thực mới được áp dụng.
_Avoid_: spoken approval, tool execution permission, automatic mutation

## Reminders and Direct Speech

**Reminder**:
Lời nhắc gồm nội dung và thời điểm đến hạn, thuộc đúng một Device; việc đổi Agent hoặc Template không thay đổi Device sở hữu.
_Avoid_: Agent-owned reminder, countdown của LLM

**Schedule Revision**:
Phiên bản tăng đơn điệu của tập lời nhắc đang chờ trên một Device; khác phiên bản của từng Reminder và của Voice Session.
_Avoid_: Reminder row revision, Session Profile Revision

**Device Cache ACK**:
Xác nhận Device đã lưu bền vững một bản lịch đánh thức; không phải xác nhận phát lời nhắc.
_Avoid_: saved on server, Playback ACK

**Delivery Attempt**:
Một lần thử phát Reminder đến đúng Device gắn với Voice Session thực hiện, tách biệt giai đoạn giữ chỗ trước khi phát và giai đoạn đã bắt đầu phát âm thanh chờ xác nhận.
_Avoid_: Reminder, Conversational Turn, waiting for device

**Bound Reminder Target**:
Thiết bị vật lý đang hoạt động thuộc cùng Agent được Admin gắn tạm thời cho một phiên Studio Assistant để nhận thao tác đặt lịch nhắc.
_Avoid_: Virtual Device reminder owner, spoken device name, permanent assistant binding

**Playback ACK**:
Xác nhận client đã phát hết dữ liệu qua lớp audio output; không chứng minh âm vật lý đã phát ra loa.
_Avoid_: end of network stream, WebSocket delivery, Device Cache ACK

**Direct Speech**:
Lần phát nội dung đã có bằng giọng tổng hợp, không nhận dạng lời nói hoặc sinh nội dung bằng mô hình ngôn ngữ.
_Avoid_: generated assistant response, Reminder (khi nói về khả năng phát dùng chung)

## Background Tasks, Reminders, and Wiki Auto-Flow

**Background Task**:
Đơn vị công việc chạy nền bền vững độc lập với Voice Session hoặc yêu cầu Admin đã khởi tạo nó, được chiếu sang giao diện Job để quan sát và quản trị.
_Avoid_: Conversational Turn, Plugin Operation Job

**Task Recipe**:
Quy trình Background Task có version, schema tham số và thứ tự bước do server định nghĩa trong code; cùng đầu vào và cùng lịch sử bước thì runtime tiếp tục theo cùng quy trình, còn phán đoán chỉ xảy ra trong Agent Step đã khai báo.
_Avoid_: Job Skill, workflow do LLM tự lập, recipe do Admin chỉnh sửa

**Collection Ledger**:
Sổ theo dõi bền trong `tasks.db` ghi trạng thái từng video của một playlist, để session mới hoặc lần chạy lại biết còn việc nào mà không dựa vào context của LLM.
_Avoid_: danh sách video trong context, tiến độ trong transcript

**Quarantined Source**:
Nguồn trong `sources/` mà Wiki Auto-Flow đã thử ingest đủ số lần cho phép nhưng chưa xong, được loại khỏi hàng đợi để không chặn các nguồn khác cho tới khi người vận hành giải phóng hoặc nguồn đã đủ trang.
_Avoid_: poison file, nguồn hỏng

**Task Follow-up**:
Chỉ dẫn bổ sung gửi tiếp vào một Background Task hiện có, được xếp hàng nếu lượt hiện tại đang chạy hoặc đánh thức lại Agent Session khi đã hoàn tất.
_Avoid_: Replan, Step retry

**Background Settings**:
Cấu hình quản trị có phiên bản quyết định việc bật/tắt thực thi Background Task, lựa chọn LLM Provider mặc định cho Orchestrator/Sub-Agent và ghi nhận các tùy chỉnh giới hạn của từng Agent.
_Avoid_: Agent Template, Voice Profile, Plugin Value Settings

**Agent Configuration**:
Định nghĩa chuyên môn do server sở hữu cho từng Root Orchestrator (`daily_root`, `wiki_root`) và các Sub-Agent chuyên biệt (`source_collector`, `wiki_researcher`, `report_stager`, `media_creator`, `media_processor`, `reminder_planner`, `wiki_ingestor`, `wiki_curator`, `cloud_worker`), bao gồm chỉ dẫn hệ thống, giới hạn bước và tập công cụ/kỹ năng hợp lệ.
_Avoid_: Agent Run, Voice Agent, requester-provided executable code

**Root Orchestrator**:
Agent cấp gốc nhận mục tiêu của Background Task, điều phối các Sub-Agent chuyên biệt qua công cụ `subagent` và tổng hợp kết quả cuối cùng; chỉ Root Orchestrator dành cho người dùng (`daily_root`) mới nhận trực tiếp yêu cầu từ Voice Session.
_Avoid_: Worker, plugin runner, Sub-Agent

**Sub-Agent**:
Agent chuyên biệt cấp con được Root Orchestrator ủy quyền qua công cụ `subagent` để tự chủ thực hiện một phần nhiệm vụ bằng tập công cụ và kỹ năng trong phạm vi được cấp.
_Avoid_: registered Voice Agent, Plugin Installation, Root Orchestrator

**Built-in Sub-Agent**:
Sub-Agent chuyên biệt chuẩn do hệ thống cung cấp sẵn kèm phạm vi công cụ, kỹ năng và chỉ dẫn mặc định; Admin có thể bật/tắt, tùy chỉnh cấu hình hoặc khôi phục về mặc định nhưng không thể xóa bỏ.
_Avoid_: Custom Sub-Agent, Root Orchestrator, hardcoded worker

**Cloud Worker**:
Built-in Sub-Agent chuyên nhận công việc dài trên dịch vụ tác nhân cloud, giữ ngữ cảnh riêng trong nhiệm vụ và bàn giao message cùng Job Artifact cho cha trước khi kết thúc. Nhiệm vụ hỏi thêm sau khi kết thúc thuộc một Sub-Agent mới với ngữ cảnh được cha cung cấp.
_Avoid_: Root Orchestrator, shared Muse chat, permanent assistant

**Custom Sub-Agent**:
Sub-Agent chuyên biệt do Admin tự định nghĩa thêm cho một Flow, được cấp tập công cụ và kỹ năng con trong phạm vi của Flow đó và có thể bật/tắt, chỉnh sửa hoặc xóa bỏ hoàn toàn khỏi hệ thống.
_Avoid_: Built-in Sub-Agent, Root Orchestrator, Custom Job Skill

**Agent Session**:
Phiên hội thoại bền vững có lịch sử thông điệp riêng của một Root Orchestrator hoặc một Sub-Agent trong một Background Task; các lượt Task Follow-up tiếp tục trên cùng Agent Session gốc nhưng tạo Agent Run mới.
_Avoid_: Agent Run, LLM Run, Voice Session

**Agent Run**:
Một lần thực thi có giới hạn bước của Root Orchestrator hoặc Sub-Agent trên một Agent Session để hoàn thành nhiệm vụ được giao hoặc một lượt Task Follow-up.
_Avoid_: registered Voice Agent, Job Step, Voice Session

**Waiting Retry**:
Trạng thái tạm dừng bền vững của một Agent Run khi gặp lỗi nhà cung cấp hoặc giới hạn tốc độ có thể phục hồi, lưu thời điểm thử lại kế tiếp và nhả slot thực thi mà không kết thúc lượt hiện tại.
_Avoid_: failed run, in-memory sleep retry, provider-layer retry

**Job Budget**:
Giới hạn tài nguyên của một lượt thực thi Background Task, gồm thời gian chạy tích lũy, thời hạn tổng thể và khoảng dự trữ để dừng có kiểm soát kèm kết quả từng phần. Các lần phục hồi dùng chung ngân sách; một Task Follow-up bắt đầu lượt thực thi với ngân sách mới.
_Avoid_: per-request timeout, single-attempt step limit

**Tool Operation**:
Tiến trình bền vững của một lần thực thi công cụ có khả năng nối lại qua các giai đoạn gửi yêu cầu, xử lý từ xa, tải dữ liệu và chốt Job Artifact.
_Avoid_: Tool Call, Tool Execution Status, in-memory future

**Request Limits**:
Bộ ba giới hạn thời gian của một lượt gọi mô hình ngôn ngữ gồm ngưỡng chờ tín hiệu đầu tiên, khoảng lặng tối đa giữa hai tín hiệu hoạt động và tổng thời lượng tối đa của lượt gọi.
_Avoid_: uniform wall-clock timeout, socket read timeout

**Delegated Task Prompt**:
Chỉ dẫn nhiệm vụ đầy đủ do Root Orchestrator biên soạn và truyền cho Sub-Agent dưới dạng thông điệp người dùng mở đầu thông qua công cụ `subagent`.
_Avoid_: Parent Session dump, permission grant

**Voice Task Tool Suite**:
Bộ 9 công cụ nội bộ (`task_create`, `task_followup`, `task_list`, `task_detail`, `task_cancel`, `reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`) được cung cấp cho Voice Session đã được chấp nhận trong phạm vi thiết bị và Agent của phiên đó.
_Avoid_: `server_create_job`, `task_delete`, MCP Tool

**Workspace ToolKit**:
Tập công cụ gắn sẵn theo vai trò Sub-Agent gồm truy vấn/biên soạn Wiki, lưu nguồn thô (`add_to_sources`), tạo báo cáo Markdown, dựng biểu đồ minh họa kiến thức trading (`chart_write`, `chart_get`), sinh hình ảnh/video, tổng hợp âm thanh TTS và phiên âm giọng nói ASR (`asr_transcribe`), trích xuất trang Web, đọc PDF, thu thập phụ đề YouTube và quản lý lịch nhắc.
_Avoid_: Plugin Package, External Plugin Installation, subprocess plugin runner

**Job Artifact**:
Tệp bất biến thuộc nhiều phân loại (báo cáo, hình ảnh, video, âm thanh, tài liệu hoặc dữ liệu có cấu trúc) được đính kèm đầu vào khi tạo Background Task hoặc do công cụ của Root Orchestrator / Sub-Agent xuất bản, gắn với Background Task kèm thông tin nguồn gốc và kiểm chứng toàn vẹn.
_Avoid_: Worker Output, Tool Result, temporary scratch file

**Artifact Catalog**:
Danh mục tập trung liệt kê các Job Artifact đã xuất bản trên toàn bộ các Background Task kèm ngữ cảnh Task nguồn, thống kê theo phân loại và khả năng lọc, xem trước, tải xuống có xác minh toàn vẹn.
_Avoid_: Knowledge Browser Space, Raw Working Note, thư mục tạm trên đĩa

**Job Read View**:
Biểu diễn hiện thời và nhất quán của một Background Task cùng các Agent Run, bước gọi công cụ, Task Follow-up và metadata Job Artifact để Admin quan sát.
_Avoid_: Job Event, execution authority

**Job Event**:
Dấu mốc tiến trình theo thứ tự nhân quả của các Agent Run, bước gọi công cụ và trạng thái kết thúc của một Background Task phục vụ quan sát; không phải lệnh thực thi hoặc quyết định cấp quyền.
_Avoid_: Tool Execution Status, Job Change Signal, authority

**Job Change Signal**:
Tín hiệu thời gian thực báo một Background Task đã thay đổi trạng thái hoặc phiên bản tiến trình và trình quan sát cần đọc lại dữ liệu chính thức; không mang kết quả hay quyết định trạng thái.
_Avoid_: Job Event, Job Result, delivery acknowledgement

**Job Skill**:
Hướng dẫn chuyên môn có tên và mô tả phạm vi áp dụng để Sub-Agent nạp vào ngữ cảnh khi thực hiện nhiệm vụ; bản thân hướng dẫn không cấp quyền thực thi công cụ.
_Avoid_: Plugin Package, Tool Grant, executable script

**Built-in Job Skill**:
Job Skill chuẩn do hệ thống cung cấp sẵn kèm nội dung mặc định để bảo đảm các quy trình cốt lõi của Sub-Agent; Admin có thể tùy chỉnh nội dung hoặc khôi phục về mặc định nhưng không thể xóa bỏ.
_Avoid_: hardcoded prompt, Custom Job Skill, read-only skill

**Custom Job Skill**:
Job Skill do Admin tự định nghĩa thêm để mở rộng hướng dẫn chuyên môn cho Sub-Agent, có thể chỉnh sửa hoặc xóa bỏ hoàn toàn khỏi hệ thống.
_Avoid_: Built-in Job Skill, custom plugin, custom tool

**Skill Catalog**:
Tập các Job Skill mà một Agent Session được phép khám phá và lựa chọn thông qua công cụ `skill` cho nhiệm vụ của mình.
_Avoid_: tool catalog, installed plugins, all deployment skills

**Loaded Skill**:
Một Job Skill có nội dung đã được đưa vào ngữ cảnh của một Agent Session; các Agent Session khác không mặc nhiên đã đọc hướng dẫn đó.
_Avoid_: enabled tool, globally loaded skill

**Wiki Auto-Flow**:
Luồng tự động chạy nền khi phát hiện nguồn mới trong `sources/`, điều phối `wiki_root` cùng `wiki_ingestor` và `wiki_curator` thực hiện tuần tự việc biên dịch sang bài viết `wiki/`, tổng hợp Synthesis và kiểm tra cấu trúc bằng `wiki_lint`.
_Avoid_: synchronous HTTP ingest, manual plugin step

## Knowledge and Memory V3

**Wiki Page**:
Tài liệu tri thức Markdown đã biên soạn trong vùng `wiki/` (gồm `sources/`, `entities/`, `concepts/`, `syntheses/`), có định danh tài liệu (`docid`) và đường dẫn tương đối trong kho tri thức.
_Avoid_: Raw Working Note, Job Artifact

**Source**:
Tài liệu bằng chứng gốc bằng Markdown được lưu giữ trong vùng `sources/` theo phân loại `<domain>/<topic>/` để các Wiki Page trích dẫn và đối chiếu.
_Avoid_: temporary attachment, Wiki Page

**Knowledge Revision**:
Phiên bản nội dung đã được chấp nhận của một Wiki Page hoặc Source trong kho tri thức, được bảo vệ trước xung đột ghi đè bằng số phiên bản hoặc mã băm SHA-256.
_Avoid_: Job Step, search-index row

**Wiki Provenance**:
Quan hệ truy nguyên giữa nội dung của một Wiki Page và các Source làm bằng chứng cho nội dung đó thông qua trường frontmatter `sources` và các liên kết trích dẫn.
_Avoid_: Wiki Link, Backlink, truth guarantee

**Memory Fact**:
Khẳng định bền vững được người dùng hoặc Admin chủ động lưu, thuộc về đúng một Device và nằm trong Memory Vault; có thể được sửa, hoàn tác hoặc quên.
_Avoid_: inference, Memory Summary, Wiki Page, temporary session note

**Memory Summary**:
Mục lục có giới hạn kích thước mà model nhận mỗi lượt, sinh lại từ các Memory Fact đang hiệu lực của một Memory Scope; không thay thế cho Memory Fact gốc.
_Avoid_: canonical memory page, index row

**Memory Vault**:
Kho Markdown riêng của Memory, tách khỏi kho tri thức Wiki, gồm một phần cho từng Device và một phần dùng chung của deployment.
_Avoid_: Wiki Vault, knowledge store

**Memory Scope**:
Phần của Memory Vault mà một thao tác được phép chạm tới, là một Device hoặc deployment; chỉ server xác định, Agent và Speaker không tham gia, và đây không phải ranh giới bảo mật.
_Avoid_: Agent memory, Speaker memory, model-selected scope

**Deployment Profile**:
Hồ sơ dùng chung do Admin viết trong phần deployment của Memory Vault, đứng đầu Memory Summary của mọi Device; không thuộc Device nào và không phải Memory Fact.
_Avoid_: Device Profile, Voice Profile, Session Profile

**Memory Proposal**:
Đề xuất do tự học tạo, chờ Admin xác nhận trước khi thành Memory Fact; thuộc giai đoạn sau, chưa có trong runtime.
_Avoid_: unconfirmed Memory Fact, auto-learned fact

**Memory Receipt**:
Bằng chứng một thay đổi Memory đã có hiệu lực, do chính thao tác ghi trả về; là căn cứ để Agent nói đã nhớ hoặc đã quên.
_Avoid_: Tool Execution Status, disk scan result

**Forget Tombstone**:
Dấu vết không đọc ngược được của nội dung đã quên, giữ cho nội dung đó không quay lại qua các đường đọc của Memory; Hoàn tác không tạo dấu vết này.
_Avoid_: soft delete, audit record

**Search Projection**:
Chỉ mục tìm kiếm được suy dẫn từ các tệp Markdown chuẩn, là BM25 kết hợp vector ngữ nghĩa với kho tri thức và BM25 với Memory Vault; việc mất hoặc tạo lại chỉ mục không làm mất tài liệu gốc.
_Avoid_: authoritative database, document source of truth

## Knowledge Physical Folder Tree

**Physical Knowledge Folder**:
Thư mục con vật lý do người dùng hoặc công cụ tạo trong vùng `sources/` (hiển thị là `raw`) hoặc `wiki/`; phản ánh cấu trúc thư mục thực tế trên đĩa thay vì nhãn phân loại ảo.
_Avoid_: tag, virtual collection, Source version

**Raw Working Note**:
Tệp Markdown nguồn nằm trong thư mục thuộc vùng `sources/` (duyệt qua không gian `raw`), có thể chỉnh sửa hoặc di chuyển kèm kiểm tra SHA-256 trước khi được phân tích và biên dịch vào `wiki/`.
_Avoid_: Wiki Page, Job Artifact

**Physical File Path**:
Đường dẫn thư mục và tên tệp Markdown thực tế trong kho tri thức mà người dùng nhìn thấy và thao tác.
_Avoid_: content hash, search snippet ID

**Knowledge Browser Space**:
Vùng thư mục vật lý (`raw` tương ứng `sources/`, hoặc `wiki`) được chọn để duyệt hoặc tìm kiếm trên giao diện quản trị; đây là bộ lọc hiển thị của Admin, không thay đổi quyền truy xuất của Agent.
_Avoid_: Agent retrieval scope, immutable snapshot

## Knowledge Wiki Graph

**Wiki Link**:
Một liên kết tường minh trong nội dung Markdown hướng tới bài viết khác theo đường dẫn hoặc tên ghi chú; khác với việc hai bài có nội dung tương tự về ngữ nghĩa.
_Avoid_: semantic similarity, external URL

**Backlink**:
Liên kết đi vào một Wiki Page từ một tài liệu khác trong kho tri thức; là góc nhìn ngược của Wiki Link, không phải quan hệ được lưu riêng.
_Avoid_: generated relationship, provenance citation

**Graph Projection**:
Biểu diễn có thể tái tạo về các tài liệu (`nodes`), liên kết hợp lệ (`edges`) và liên kết chưa phân giải (`unresolved`), phục vụ duyệt toàn bộ hoặc theo vùng lân cận mà không thay đổi nội dung Wiki.
_Avoid_: canonical Wiki body, second knowledge store, Memory graph

**Answer Graph**:
Đồ thị con gắn với một câu trả lời Wiki, gồm các trang đã được truy vấn, đọc hoặc trích dẫn cùng những láng giềng nối với chúng bằng Wiki Link có thật.
_Avoid_: AI-inferred graph, canonical graph, second knowledge store

**Wiki Conversation**:
Chuỗi câu hỏi và câu trả lời đã lưu của Admin với wiki, có thể mở lại và hỏi tiếp; là dữ liệu đã lưu, giữa các lượt không có gì đang chạy.
_Avoid_: session, phiên, Voice Session, Agent Session

**Unresolved Wiki Link**:
Liên kết có đích chưa tồn tại hoặc không phân giải được sang tài liệu hợp lệ trong kho tri thức; được ghi nhận riêng trên đồ thị và trong báo cáo `wiki_lint`, không tự động tạo trang mới.
_Avoid_: verified edge, automatically created page

## Multi-Channel Notifications

**Notification**:
Thông điệp bền vững phản ánh một sự kiện cần chú ý của hệ thống (Background Task kết thúc, lịch hẹn đến hạn, hoặc yêu cầu duyệt quản trị), độc lập với Reminder phát giọng nói trên Device.
_Avoid_: Device Reminder, Job Change Signal, Tool Execution Status

**Notification Inbox**:
Hộp thư thông báo mặc định dành cho Admin trên giao diện quản trị, duy trì trạng thái chưa đọc/đã đọc của từng Notification.
_Avoid_: ephemeral toast, Job Event log

**Notification Channel**:
Kênh đích nhận Notification, gồm kênh mặc định trên giao diện quản trị và các kênh nhắn tin bên ngoài đã cấu hình.
_Avoid_: Direct Speech, Voice Session

**Notification Channel Policy**:
Quyết định quản trị xác định kênh thông báo nào được bật, đích nhận và những chủ đề sự kiện nào được phép chuyển phát qua kênh đó.
_Avoid_: Agent Tool Allowlist, Reminder policy

**Channel Delivery Attempt**:
Một lần thử chuyển phát một Notification tới một Notification Channel bên ngoài, có kết quả độc lập với trạng thái đã đọc trong Notification Inbox.
_Avoid_: Delivery Attempt (của Device Reminder)

## System Observability & Structured Logs

**Structured Log Entry**:
Một bản ghi quan sát vận hành tại một thời điểm của tiến trình server, mang mức độ nghiêm trọng (`level`), phân hệ (`subsystem`), tên sự kiện chuẩn hóa (`event`), định danh ngữ cảnh (`session_id`, `turn_id`, `request_id`, `agent_key`) và tập trường dữ liệu đã qua kiểm duyệt bảo mật (redacted); chỉ tồn tại trong bộ nhớ đệm vòng (ring buffer) của tiến trình, không phải bản ghi kiểm toán hay lịch sử hội thoại.
_Avoid_: Admin Audit Event, Persistent Transcript, Job Event, Notification

**Correlation Context**:
Tập các định danh truy vết (`request_id`, `session_id`, `turn_id`, `generation`, `agent_key`, `device_id`, `task_id`) được gắn vào một đơn vị thực thi (Span) và tự động kế thừa xuống mọi Structured Log Entry phát sinh bên trong đơn vị đó.
_Avoid_: Session Profile, Turn System Prompt, Scenario State

**Log Ring Buffer**:
Bộ đệm vòng có sức chứa cố định trong RAM của tiến trình server, giữ lại các Structured Log Entry gần nhất kèm số thứ tự tăng đơn điệu (`seq`) để phục vụ truy vấn tức thời và bù dữ liệu khi kết nối mạng bị gián đoạn.
_Avoid_: History Archive, Notification Inbox, SQLite log table

**Live Log Stream**:
Luồng đẩy thời gian thực một chiều từ server tới trình duyệt của Admin đã xác thực, áp dụng bộ lọc ngay tại server trước khi phát từng Structured Log Entry.
_Avoid_: Job Change Signal, Voice Session WebSocket, unauthenticated tail

## Internal LLM Gateway & OAuth Accounts

**Gateway Account**:
Một tài khoản nhà cung cấp mô hình ngôn ngữ đã được xác thực qua OAuth 2.0 hoặc phiên máy ảo/cookie ủy quyền (như Claude, OpenAI Codex, Google Gemini, GitHub Copilot, hoặc Meta Muse AI) do Admin quản lý độc lập với từng Provider Instance, có vòng đời làm mới phiên xác thực và trạng thái sẵn sàng riêng.
_Avoid_: Provider Instance, static API key, Admin User, OAuthAccount

**Gateway Account Pool**:
Tập hợp các Gateway Account đang bật và không nằm trong thời gian tạm nghỉ do giới hạn tốc độ của cùng một nhà cung cấp, cho phép phân phối lượt gọi luân phiên và chuyển đổi dự phòng có giới hạn trước khi phát dữ liệu giữa nhiều tài khoản.
_Avoid_: Provider Runtime Pool, unbounded retry loop, AccountPool

**Gateway Binding**:
Tham chiếu phi bí mật trên cấu hình của một Provider Instance liên kết nó với một Gateway Account cụ thể hoặc một Gateway Account Pool thay vì sử dụng khóa API tĩnh.
_Avoid_: Secret Reference, inline credential, OAuth token

**OAuth Login Session**:
Phiên đăng nhập tạm thời được khởi tạo khi Admin bắt đầu luồng ủy quyền OAuth trên trình duyệt (qua mã PKCE hoặc mã thiết bị), tồn tại cho đến khi đổi thành công thành một Gateway Account hoặc hết hạn.
_Avoid_: Admin Session, Voice Session, Gateway Account

**Job Session**:
Phiên làm việc từ xa trên nhà cung cấp tác nhân, giữ ngữ cảnh qua nhiều lượt công cụ trên cùng Gateway Account. Phạm vi sở hữu có thể là Background Task đối với video hoặc Agent Session của Sub-Agent đối với công việc cloud; kết thúc phạm vi đó cũng kết thúc phiên.
_Avoid_: Agent Session, Voice Session, single-call operation session

**Continuation Key**:
Định danh ổn định của phạm vi sở hữu công việc từ xa, gắn các lần gọi công cụ kế tiếp với cùng Job Session và Gateway Account đã ghim; mỗi loại công việc và mỗi phiên Sub-Agent có phạm vi riêng.
_Avoid_: remote session ID, tool invocation ID, turn ID

## Database Backup & Transactional Restore

**Database Backup Bundle**:
Gói sao lưu khả chuyển chứa cấu hình mong muốn, danh mục thiết bị và giọng nói, bí mật đã niêm phong và dữ liệu lưu trữ theo phạm vi đã chọn, định danh tài nguyên bằng khóa nghiệp vụ bất biến thay vì khóa nguyên nội bộ của cơ sở dữ liệu.
_Avoid_: raw database dump, migration journal, plaintext credential export

**Database Snapshot**:
Bản chụp tệp vật lý nhất quán tại một thời điểm của cơ sở dữ liệu điều khiển chính, được xuất thành một tệp độc lập mà không làm gián đoạn các phiên đang hoạt động.
_Avoid_: live WAL file copy, filesystem volume clone, partial table dump

**Backup Inspection**:
Bước đánh giá chỉ đọc trên một bản sao lưu ứng viên để báo cáo số lượng tài nguyên, các thay đổi thêm mới, cập nhật hoặc xóa bỏ dự kiến, cùng khả năng tương thích của khóa giải mã bí mật trước khi áp dụng.
_Avoid_: dry-run migration, schema diff, live import




