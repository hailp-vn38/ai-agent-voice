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
Tập công cụ quản trị do deployment cung cấp cho Studio Assistant đã xác thực, với quyền riêng cho việc đọc trạng thái hệ thống và đọc metadata MCP.
_Avoid_: Device MCP, External MCP server, Admin tool authority

**System Tool Grant**:
Quyền cho Studio Assistant sử dụng một nhóm công cụ System MCP; độc lập với quyền gọi công cụ của External MCP và không được cấp bởi nội dung hội thoại.
_Avoid_: Agent–External MCP binding, tool execution status
