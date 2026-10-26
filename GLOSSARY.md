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
Một lần thử phát Reminder đến đúng Device, có danh tính riêng gắn với Voice Session thực hiện lần thử.
_Avoid_: Reminder, Conversational Turn, waiting for device

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

**Task Follow-up**:
Chỉ dẫn bổ sung gửi tiếp vào một Background Task hiện có, được xếp hàng nếu lượt hiện tại đang chạy hoặc đánh thức lại Agent Session khi đã hoàn tất.
_Avoid_: Replan, Step retry

**Background Settings**:
Cấu hình quản trị có phiên bản quyết định việc bật/tắt thực thi Background Task, lựa chọn LLM Provider mặc định cho Orchestrator/Sub-Agent và ghi nhận các tùy chỉnh giới hạn của từng Agent.
_Avoid_: Agent Template, Voice Profile, Plugin Value Settings

**Agent Configuration**:
Định nghĩa chuyên môn do server sở hữu cho từng Root Orchestrator (`daily_root`, `wiki_root`) và các Sub-Agent chuyên biệt (`source_collector`, `wiki_researcher`, `report_stager`, `media_creator`, `reminder_planner`, `wiki_ingestor`, `wiki_curator`), bao gồm chỉ dẫn hệ thống, giới hạn bước và tập công cụ/kỹ năng hợp lệ.
_Avoid_: Agent Run, Voice Agent, requester-provided executable code

**Root Orchestrator**:
Agent cấp gốc nhận mục tiêu của Background Task, điều phối các Sub-Agent chuyên biệt qua công cụ `subagent` và tổng hợp kết quả cuối cùng; chỉ Root Orchestrator dành cho người dùng (`daily_root`) mới nhận trực tiếp yêu cầu từ Voice Session.
_Avoid_: Worker, plugin runner, Sub-Agent

**Sub-Agent**:
Agent chuyên biệt cấp con được Root Orchestrator ủy quyền qua công cụ `subagent` để tự chủ thực hiện một phần nhiệm vụ bằng tập công cụ và kỹ năng trong phạm vi được cấp.
_Avoid_: registered Voice Agent, Plugin Installation, Root Orchestrator

**Agent Session**:
Phiên hội thoại bền vững có lịch sử thông điệp riêng của một Root Orchestrator hoặc một Sub-Agent trong một Background Task; các lượt Task Follow-up tiếp tục trên cùng Agent Session gốc nhưng tạo Agent Run mới.
_Avoid_: Agent Run, LLM Run, Voice Session

**Agent Run**:
Một lần thực thi có giới hạn bước của Root Orchestrator hoặc Sub-Agent trên một Agent Session để hoàn thành nhiệm vụ được giao hoặc một lượt Task Follow-up.
_Avoid_: registered Voice Agent, Job Step, Voice Session

**Delegated Task Prompt**:
Chỉ dẫn nhiệm vụ đầy đủ do Root Orchestrator biên soạn và truyền cho Sub-Agent dưới dạng thông điệp người dùng mở đầu thông qua công cụ `subagent`.
_Avoid_: Parent Session dump, permission grant

**Voice Task Tool Suite**:
Bộ 9 công cụ nội bộ (`task_create`, `task_followup`, `task_list`, `task_detail`, `task_cancel`, `reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`) được cung cấp cho Voice Session đã được chấp nhận trong phạm vi thiết bị và Agent của phiên đó.
_Avoid_: `server_create_job`, `task_delete`, MCP Tool

**Workspace ToolKit**:
Tập công cụ gắn sẵn theo vai trò Sub-Agent gồm truy vấn/biên soạn Wiki, lưu nguồn thô (`add_to_sources`), tạo báo cáo Markdown, sinh hình ảnh và âm thanh TTS, trích xuất trang Web, đọc PDF, thu thập phụ đề YouTube và quản lý lịch nhắc.
_Avoid_: Plugin Package, External Plugin Installation, subprocess plugin runner

**Job Artifact**:
Tệp kết quả bất biến thuộc nhiều loại (báo cáo Markdown, hình ảnh PNG/SVG, âm thanh WAV hoặc dữ liệu cấu trúc) do công cụ của Sub-Agent xuất bản và gắn với một Background Task kèm mã băm SHA-256 xác minh toàn vẹn.
_Avoid_: Worker Output, Tool Result, temporary scratch file

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
Khẳng định tri thức bền vững về một thực thể hoặc sự kiện được lưu trữ dưới dạng trang thực thể (`wiki/entities/`), có thể được tạo mới, cập nhật hoặc xóa bỏ (`forget`).
_Avoid_: inference, Memory Summary, temporary session note

**Memory Summary**:
Bản tóm tắt có giới hạn kích thước được trích xuất từ chỉ mục tổng hợp (`INDEX.md`) của kho tri thức, không thay thế cho từng Memory Fact hay Wiki Page gốc.
_Avoid_: canonical memory page, index row

**Search Projection**:
Chỉ mục tìm kiếm lai (BM25 kết hợp vector ngữ nghĩa) được suy dẫn từ các tệp Markdown chuẩn trong kho tri thức; việc mất hoặc tạo lại chỉ mục không làm mất tài liệu gốc.
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
Vùng thư mục vật lý (`raw` tương ứng `sources/`, hoặc `wiki`) được chọn để duyệt trên giao diện quản trị; đây là bộ lọc hiển thị, không thay đổi quyền truy xuất của Agent.
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

**Unresolved Wiki Link**:
Liên kết có đích chưa tồn tại hoặc không phân giải được sang tài liệu hợp lệ trong kho tri thức; được ghi nhận riêng trên đồ thị và trong báo cáo `wiki_lint`, không tự động tạo trang mới.
_Avoid_: verified edge, automatically created page
