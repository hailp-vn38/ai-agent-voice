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
Môi trường thực thi của một Plugin Package; độc lập với Plugin Operation và không phải một Agent hay quyền gọi công cụ.
_Avoid_: MCP Binding, LLM Provider, Job

Thuật ngữ của miền Job được định nghĩa tại [CONTEXT.md](CONTEXT.md#background-jobs-and-executable-plugins). Job độc lập với Reminder và Conversational Turn.

## Agent Job Planning

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
