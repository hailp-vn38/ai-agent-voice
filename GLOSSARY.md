# Voice Agent Conversation Domain

Các khái niệm domain cho hội thoại qua Voice Session và tool execution; tách thông tin thực thi khỏi nội dung phản hồi cho người dùng.

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

## Background Jobs and Executable Plugins

**Plugin Runtime**:
Môi trường mà Plugin Operation được thực hiện, độc lập với Voice Session, Agent và quyền sử dụng công cụ.
_Avoid_: MCP Binding, LLM Provider, Inference Worker Runtime

**Plugin Package**:
Gói ZIP chứa mã thực thi, manifest và các operation contract; được nhận diện theo nội dung bất biến.
_Avoid_: Plugin Installation, Docker image

**Plugin Installation**:
Môi trường plugin đã chuẩn bị đầy đủ dependencies và entrypoint, được định danh bất biến bằng Installation Digest, nhưng chưa mặc nhiên được kích hoạt.
_Avoid_: Plugin Package, runtime image

**Installation Digest**:
Định danh cho đúng môi trường Plugin Installation mà một Job đã lựa chọn và đóng băng.
_Avoid_: Docker Image Digest, mutable latest

**Worker Attempt**:
Một lần thực thi Operation có danh tính riêng và checkpoint bền vững; kết quả chỉ được Job Engine chấp nhận sau khi kiểm tra.
_Avoid_: Plugin Installation, Agent Job

**Core Plugin**:
Nhóm các Core Operation cùng chức năng được Rust Server cung cấp sẵn, không có vòng đời cài đặt hoặc gỡ bỏ; khả năng bắt đầu operation có thể được bật/tắt bằng Core Plugin Policy.
_Avoid_: Plugin Installation, Plugin Package, External Plugin

**Core Plugin Policy**:
Quyết định quản trị đối với việc có cho phép bắt đầu Core Operation thuộc một Core Plugin hay không; độc lập với sự tồn tại của implementation trong Rust Server.
_Avoid_: Plugin Installation, Plugin Activation, Core Operation contract

**Core Operation**:
Operation được sở hữu và cung cấp sẵn bởi hệ thống, không phụ thuộc Plugin Package hoặc Plugin Installation.
_Avoid_: Plugin Installation, Built-in Voice Tool, External Plugin

**External Plugin Operation**:
Operation đến từ một Plugin Installation có thể được quản trị và cập nhật riêng với phiên bản server.
_Avoid_: Core Operation, MCP Tool

**Frozen Operation Choice**:
Giới hạn tối đa của các Operation mà một Job đã được quyền lựa chọn, tham chiếu tới bản hợp đồng Core hoặc đúng Plugin Installation; không tự động vượt qua quyết định tạm ngừng khả năng bắt đầu operation sau đó.
_Avoid_: live catalog entry, mutable latest, Tool Call

**Plugin Activation**:
Quyết định quản trị đưa một Plugin Installation vào trạng thái có thể được Job lựa chọn; không phải phê duyệt quyền truy cập mạng.
_Avoid_: Plugin Permission Grant, Network Hosts Approval, Agent grant

**Plugin Network Reachability**:
Tập đích mạng mà môi trường thực thi plugin có thể tiếp cận; không phải quyền do manifest hoặc server cấp và không đảm bảo kết nối thành công.
_Avoid_: Network Hosts, Egress Allowlist, Plugin Network Grant

Thuật ngữ Job, Plugin Package, Plugin Installation và Worker Attempt được định nghĩa tại [CONTEXT.md](CONTEXT.md#background-jobs-and-executable-plugins); ranh giới mạng được quyết định tại [ADR 0101](docs/adr/0101-direct-plugin-worker-network.md). Job độc lập với Reminder và Conversational Turn.

## Agent Job Planning


**Unified Job Activity**:
Biểu diễn theo thứ tự thời gian các mốc LLM, Plugin, Fork/Join và kết thúc của một Job, được ghép từ Job Event và trạng thái Job chính thức.
_Avoid_: raw model transcript, tool output archive, Job Change Signal

**LLM Run**:
A single bounded invocation of a model during an Agent Job, including provider admission, model response and Rust validation; distinct from the Job, the Step, and a Plan Proposal.
_Avoid_: Agent account, Voice Session, vendor background task, Job Step

**LLM Activity Event**:
A durable metadata-only observation correlated to one LLM Run, ordered within its Job; it does not own Job/Step state or authorize model execution.
_Avoid_: model content log, prompt archive, token stream, Job checkpoint

**Resolved LLM Provider Identity**:
The adapter, model and transport verified for an actual LLM Run against its pinned Provider identity; if resolution fails, only the expected pin is known.
_Avoid_: current Provider Catalog selection, inferred model, remote request acknowledgement



**Plan Proposal**:
Ý định gồm các bước công việc mà Orchestrator đề xuất; chưa có quyền thực thi và có thể bị từ chối hoặc yêu cầu sửa.
_Avoid_: executable plan, worker command

**Executable Plan**:
Kế hoạch được Rust xác thực và chấp nhận để điều phối; dựa trên các lựa chọn năng lực đã đóng băng của Job.
_Avoid_: LLM response, proposal

**Plan Repair**:
Lượt sửa một Plan Proposal chưa được chấp nhận, trước khi bất kỳ bước mới nào được thực thi.
_Avoid_: execution retry, replan

**Replan**:
Đề xuất bổ sung công việc sau khi đã có kết quả thực thi, chỉ được thêm bước mới mà không sửa các bước đã công bố.
_Avoid_: plan repair, replay

**Job Event**:
Dấu mốc trạng thái được ghi bền vững, dành cho việc quan sát Job; không phải lệnh thực thi hoặc quyết định cấp quyền.
_Avoid_: Tool Execution Status, authority


**Job Change Signal**:
Tín hiệu tạm thời báo một Job đã thay đổi và người quan sát cần đọc lại dữ liệu chính thức; không mang kết quả hay quyết định trạng thái.
_Avoid_: Job Event, Job Result, delivery acknowledgement

**Job Read View**:
Biểu diễn hiện thời và nhất quán của Job, các Step và metadata Artifact để Admin quan sát, khác cấu hình thực thi được đóng băng khi nhận Job.
_Avoid_: JobSnapshot, Job Event, execution authority

## Hierarchical Agent Jobs

**Agent Run**:
Một phiên thực thi của Root Orchestrator hoặc Sub-Agent trong Agent Job, khác với Job, LLM Run và Worker Attempt.
_Avoid_: registered Voice Agent, Job Step, LLM Run

**Root Decision**:
Đề xuất `delegate` hoặc `finish` của Root Orchestrator, chỉ có hiệu lực sau khi được Rust kiểm tra và chấp nhận.
_Avoid_: Executable Plan, plugin call

**Plugin Tool Binding**:
Ánh xạ trong phạm vi một Sub-Agent từ tên function mà LLM thấy tới Frozen Operation Choice đã được phép, bất kể Core hay External.
_Avoid_: permission grant, MCP Binding, Plugin Activation

**Child Completion Tool**:
Function nội bộ `submit_result` để Child đề xuất hoàn tất Agent Run, không phải Plugin Operation hoặc quyết định hoàn tất Job.
_Avoid_: Plugin Step, Job Result

**Child Evidence Reference**:
UUID của Job Artifact được một Plugin Step thành công thuộc chính Sub-Agent đó tạo ra, được Rust đưa vào Tool Result và kiểm tra khi Child gọi `submit_result`.
_Avoid_: URL nguồn bài viết, Step Key, Provider Call ID, Artifact của Sub-Agent khác

**Root Orchestrator**:
Agent Run sở hữu quyết định giao nhiệm vụ và kết thúc Job; chỉ nhận kết quả tổng hợp từ Sub-Agent.
_Avoid_: Worker, plugin runner

**Sub-Agent**:
Agent Run cấp con tự ra quyết định gọi công cụ hoặc kết thúc, trong phạm vi quyền được Job đóng băng.
_Avoid_: registered Agent, Plugin Installation

**Agent Decision Turn**:
Một LLM Run duy nhất đề xuất một hành động Root hoặc Child; chỉ Rust mới có thể chấp nhận hành động.
_Avoid_: Conversational Turn, Tool Round

**Fork Group**:
Tập Sub-Agent có cùng quyết định giao nhiệm vụ, được Join sau khi mọi thành viên kết thúc.
_Avoid_: Job Queue, durable independent Job


## Agent Job V3 — OpenCode-style delegation

**Agent Configuration**:
Server-owned specialist definition (initially research/document/general), including system instructions. It does not give permission to expand the frozen Job tool set.
_Avoid_: Agent Run, ProviderPin, requester-provided executable code

**Agent Session**:
Durable independent transcript plus pinned route identity for one Root or Child. Child follow-up retains the Session but creates another Agent Run.
_Avoid_: Agent Run, LLM Run, Voice Session

**Canonical Message**:
Provider-neutral system/user/assistant/tool-call/tool-result record persisted under an Agent Session and translated by a concrete provider Adapter.
_Avoid_: raw provider response, Job Event, SSE metadata

**Delegated Task Prompt**:
Bounded full task instructions authored by Root and supplied as a new Child user message through the server-owned `subagent` tool.
_Avoid_: `goal_excerpt`, Parent Session dump, permission grant

**ChildOutcome**:
Rust-owned task outcome containing session/run identity, status, bounded summary, verified artifact references and stable error code. It is delivered as a Tool Result matched to the Root's call ID.
_Avoid_: raw Child transcript, model-defined Job status

**Provider Capability**:
A route-specific ability (native tools, structured output, replay constraints), verified for the pinned provider/model/transport, not inferred from adapter naming.
_Avoid_: operator boolean alone, model marketing description

**Root Tool Result**:
Canonical tool observation paired with the original native Root `subagent` call ID, carrying only validated ChildOutcome rather than provider-native Child tool blocks.
_Avoid_: joined_children synthetic user prompt


## Document Conversion and Tool Output

**Document Conversion**:
Phép chuyển một Job-owned source document thành nội dung Markdown có cấu trúc; bản gốc vẫn là bằng chứng riêng và kết quả Markdown không tự trở thành Knowledge Source.
_Avoid_: Knowledge Import, Wiki Publication, OCR

**Durable Step Result**:
Kết quả JSON đã được Rust xác thực và lưu cùng một Job Step; có thể chứa metadata tham chiếu tới Job Artifacts nhưng không phải transcript mà LLM nhất thiết đã xem.
_Avoid_: Model Observation, Worker Output, Job Artifact

**Model-visible Tool Observation**:
Phần kết quả Tool có ngân sách rõ ràng được đưa vào Agent Session dưới dạng một Tool Result tương ứng với Tool Call; có thể kèm chỉ dẫn đọc tiếp và không được coi là toàn bộ Durable Step Result.
_Avoid_: complete Worker Output, verified full document read, Job Event

## Agent Job Artifacts

**Job Artifact**:
Tệp bất biến do một Job Tool Step đã được chấp nhận xuất bản, thuộc đúng một Job.
_Avoid_: Worker Output, Tool Result

**Worker Output**:
Tệp tạm thời do Plugin Worker Attempt tạo ra trước khi Step xuất bản thành Job Artifact.
_Avoid_: Job Artifact

**Display Filename**:
Tên tệp hiển thị cho người dùng của Job Artifact, không phải định danh hay khóa sở hữu.
_Avoid_: Artifact ID, đường dẫn lưu trữ

## Plugin Value Settings and Manual Operation Jobs

**Plugin Key**:
Định danh ổn định để Admin quản lý một Core Plugin group hoặc External Plugin installation; khác frozen plugin ID dùng để thực thi Core Operation.
_Avoid_: Frozen Operation Choice, installation digest, Device Tool ID

**Plugin Value Settings**:
Các giá trị cấu hình được Admin cấp cho một Plugin và được kiểm tra theo Settings Schema, độc lập trạng thái bật/tắt hoặc installation lifecycle.
_Avoid_: Plugin Activation, Agent Job Settings, Tool Arguments

**Plugin Config Revision**:
Một phiên bản bất biến của Plugin Value Settings mà Job có thể tham chiếu để giữ đúng cấu hình được chấp nhận tại admission.
_Avoid_: Core activation revision, package version, live config pointer

**Plugin Config Pin**:
Quyền tham chiếu của một Job tới một Plugin Config Revision cụ thể; không cho phép lấy ngầm cấu hình mới hoặc secrets mới sau admission.
_Avoid_: Provider Pin, live Settings, credential reference

**Job TTS Provider Pin**:
Danh tính và phiên bản cấu hình chính xác của một TTS Provider được đóng băng khi Job được chấp nhận để thực hiện Core TTS Operation; khác với Plugin Config Pin và provider hiện thời.
_Avoid_: Plugin Config Pin, current provider, runtime pointer

**Plugin Operation Job**:
Job được Admin yêu cầu để chạy chính xác một Plugin Operation đã công bố mà không cần Root/Sub-Agent, Agent Session hoặc LLM.
_Avoid_: Agent Job, subagent call, Plugin Runtime Check

**Operation Contract**:
Mô tả được Plugin công bố cho một operation gồm tên, đầu vào và đầu ra hợp lệ; là căn cứ Rust xác thực trước khi gọi thực thi.
_Avoid_: arbitrary function, LLM-generated tool implementation

## Provisioned Core Dependencies

**Provisioned Core Plugin**:
Core Plugin được tích hợp cố định trong server nhưng chỉ có thể thực hiện operation cần binary ngoài khi các dependency của nó đã sẵn sàng.
_Avoid_: External Plugin Installation, Plugin Store package, Agent permission

**Dependency Bundle**:
Tập binary có phiên bản và nguồn gốc đã xác minh, được lưu bất biến và dùng chung cho nhiều lượt thực thi Core Operation.
_Avoid_: Job Artifact, temporary download, plugin ZIP

**Availability**:
Khả năng thực thi hiện tại của dependency, độc lập với quyết định bật/tắt Core Plugin và danh sách tool đã đóng băng của Job.
_Avoid_: Plugin Activation, Frozen Operation Choice, Job status

## Knowledge and Memory V3

**Wiki Page**:
Reusable sourced knowledge document with stable identity independent of its title, path and revision.
_Avoid_: Memory Fact, Job Artifact

**Source**:
Preserved version of original evidence which a Wiki Page may cite.
_Avoid_: temporary attachment, Wiki Page

**Knowledge Revision**:
One accepted immutable version of a Wiki Page.
_Avoid_: Job Step, search-index row

**Memory Fact**:
An owner-scoped durable assertion, independently confirmed, corrected, superseded or forgotten.
_Avoid_: Wiki Page, inference, Memory Summary

**Memory Summary**:
Bounded generated view of active confirmed Memory Facts, never the canonical representation of the facts.
_Avoid_: canonical memory page, index row

**Publication Proposal**:
Candidate Wiki content awaiting trusted Rust validation and revision checking.
_Avoid_: accepted Wiki Page, direct filesystem instruction

**Search Projection**:
Derived index of canonical Markdown whose loss does not delete the underlying documents.
_Avoid_: authoritative database, document source of truth


## Knowledge Physical Folder Tree

**Physical Knowledge Folder**:
Thư mục con vật lý do người dùng tạo trong Raw hoặc Wiki; không được suy ra từ nhãn hay phân loại tìm kiếm.
_Avoid_: tag, virtual collection, Source version

**Raw Working Note**:
Markdown có thể chỉnh sửa trong một thư mục Raw do người dùng chọn; chưa phải Source được xác minh và không tự xuất hiện trong kết quả Knowledge của Agent.
_Avoid_: immutable Source, Wiki Page, Job Artifact

**Immutable Source Snapshot**:
Bản sao bằng chứng có nội dung và nguồn gốc được ghim cố định, độc lập với Raw Working Note có thể thay đổi.
_Avoid_: Raw Working Note, current document path

**Staged Wiki Draft**:
Phiên bản sửa chưa được publish của một Wiki đã publish; có thể được người dùng hoặc một Job chuẩn bị, trong khi Agent vẫn dùng Knowledge Revision cuối cùng được chấp nhận.
_Avoid_: canonical Wiki Page, Raw Working Note

**Physical File Path**:
Thư mục và tên file thực tế mà người dùng nhìn thấy; có thể đổi mà không đổi stable ID của Wiki Page.
_Avoid_: Wiki stable ID, Source reference, Knowledge Revision

**Knowledge Browser Space**:
Một trong ba vùng thư mục vật lý Raw, Wiki hoặc Drafts được chọn để duyệt trên giao diện. Đây là bộ lọc hiển thị, không thay đổi quyền truy xuất của Agent.
_Avoid_: immutable Source, Knowledge Revision, Agent retrieval scope


## Knowledge Wiki Graph

**Wiki Link**:
Một liên kết tường minh trong nội dung Wiki, có đích là bài viết được nhận diện theo đường dẫn ghi chú; khác với việc hai bài có nội dung tương tự.
_Avoid_: semantic similarity, Source reference, Wiki stable ID

**Backlink**:
Liên kết đi vào một Wiki Page từ một Wiki Page khác; là góc nhìn ngược của Wiki Link, không phải quan hệ được tạo riêng.
_Avoid_: generated relationship, provenance citation

**Graph Projection**:
Biểu diễn có thể tái tạo về các Wiki Page và Wiki Link hiện hành, phục vụ duyệt toàn bộ hoặc theo vùng lân cận mà không thay đổi nội dung Wiki.
_Avoid_: canonical Wiki body, second knowledge store, Memory graph

**Unresolved Wiki Link**:
Liên kết có đích không tồn tại, mơ hồ hoặc không hợp lệ; không được tự suy đoán thành một Wiki Page đã được xác nhận.
_Avoid_: verified edge, automatically created page

## Knowledge Jobs and Job Skills

**Knowledge Job**:
Một lần xử lý Source để tạo hoặc cập nhật Wiki Page, theo nhiệm vụ chuyên Knowledge và quyền xuất bản được cấp cho lần chạy đó.
_Avoid_: permanent Job, Knowledge scheduler, Publication Proposal

**Job Skill**:
Hướng dẫn có tên và mô tả phạm vi áp dụng để Agent lựa chọn cách thực hiện một nhiệm vụ; bản thân hướng dẫn không cấp quyền thực thi công cụ.
_Avoid_: Plugin Package, Tool Grant, executable script

**Skill Catalog**:
Tập các Job Skill mà một Agent Session được phép khám phá và lựa chọn cho nhiệm vụ của mình.
_Avoid_: tool catalog, installed plugins, all deployment skills

**Loaded Skill**:
Một Job Skill có nội dung đã được đưa vào ngữ cảnh của một Agent Session; Session khác không mặc nhiên đã đọc hướng dẫn đó.
_Avoid_: enabled tool, globally loaded skill

**Knowledge Prompt**:
Chỉ dẫn xác định vai trò, mục tiêu và điều kiện hoàn tất của một Knowledge Job; khác với hướng dẫn cách biên soạn nội dung của Job Skill.
_Avoid_: Source content, Skill Catalog, user-authored Wiki instructions

**Wiki Provenance**:
Quan hệ giữa nội dung của một Knowledge Revision và các Source làm bằng chứng cho nội dung đó; một revision có thể dựa trên nhiều Source.
_Avoid_: Wiki Link, Backlink, truth guarantee

**Job-owned Wiki Draft**:
Bản Wiki chưa publish được chuẩn bị cho đúng một Job; quyền sở hữu này ngăn Job khác thay thế công việc đang thực hiện.
_Avoid_: human draft, published Knowledge Revision

**Source Coverage**:
Phần nội dung của Source được pin đã thực sự được giao cho Knowledge Session; coverage đầy đủ bao gồm toàn bộ nội dung và xác nhận kết thúc Source.
_Avoid_: AI comprehension, artifact reference, search visibility

**Draft Resolution**:
Quyết định của Admin sau khi Job kết thúc để tiếp quản hoặc loại bỏ đúng bản draft còn lại; không thay đổi kết quả lịch sử hoặc tự xuất bản Wiki.
_Avoid_: mandatory approval, Job retry, publication rollback

**Instruction Snapshot**:
Bản chỉ dẫn và Skill Catalog được chốt cho một Job; thay đổi hướng dẫn cho công việc mới không thay chỉ dẫn của Job đã được nhận.
_Avoid_: live deployment prompt, Tool Grant

## Background Tasks, Reminders, and Wiki Auto-Flow

**Background Task**:
Đơn vị công việc chạy nền bền vững độc lập với Voice Session hoặc yêu cầu Admin đã khởi tạo nó.
_Avoid_: Conversational Turn, Plugin Operation Job

**Task Follow-up**:
Chỉ dẫn bổ sung gửi tiếp vào một Background Task hiện có, được xếp hàng nếu lượt hiện tại đang chạy hoặc đánh thức lại phiên khi đã hoàn tất.
_Avoid_: Replan, Step retry

**Voice Task Tool Suite**:
Bộ 9 công cụ nội bộ (`task_create`, `task_followup`, `task_list`, `task_detail`, `task_cancel`, `reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`) được cung cấp cho Voice Session đã được chấp nhận.
_Avoid_: `server_create_job`, `task_delete`, MCP Tool

**Wiki Auto-Flow**:
Luồng tự động chạy nền khi phát hiện nguồn mới trong `sources/`, thực hiện tuần tự việc biên dịch sang bài viết `wiki/`, tổng hợp Synthesis và kiểm tra cấu trúc bằng `wiki_lint`.
_Avoid_: synchronous HTTP ingest, manual plugin step

**Workspace ToolKit**:
Tập công cụ gắn sẵn theo vai trò Agent gồm truy vấn/ghi Wiki, tạo báo cáo Markdown, trích xuất trang Web, chuyển đổi PDF, thu thập phụ đề YouTube và quản lý lịch nhắc.
_Avoid_: Plugin Package, External Plugin Installation

