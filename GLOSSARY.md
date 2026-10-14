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

**Core Operation**:
Operation được sở hữu và cung cấp sẵn bởi hệ thống, không phụ thuộc Plugin Package hoặc Plugin Installation.
_Avoid_: Plugin Installation, Built-in Voice Tool, External Plugin

**External Plugin Operation**:
Operation đến từ một Plugin Installation có thể được quản trị và cập nhật riêng với phiên bản server.
_Avoid_: Core Operation, MCP Tool

**Frozen Operation Choice**:
Quyền lựa chọn một Operation được cố định cho một Job; tham chiếu tới bản hợp đồng Core hoặc tới đúng Plugin Installation.
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
