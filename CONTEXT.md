# xiaozhi-lite-rs

Voice-agent server cá nhân tương thích firmware Xiaozhi. Context này điều phối một Voice Session giữa một thiết bị Xiaozhi và các provider AI.

## Language

**Compatibility Profile**:
Tổ hợp firmware baseline, wire protocol và audio/MCP contract mà server cam kết tương thích.
_Avoid_: protocol version (khi chỉ nói đến phiên bản wire protocol)

**Firmware Baseline**:
Phiên bản firmware và commit đã pin dùng làm nguồn chân lý tương thích.
_Avoid_: upstream main, firmware mới nhất

**HIL Reference Profile**:
Thiết bị và cấu hình mạng vật lý chuẩn dùng để xác nhận tương thích firmware.
_Avoid_: fake client, simulated client

**Voice Session**:
Một kết nối WebSocket đang hoạt động, thuộc duy nhất một Device ID và mang dialogue chỉ trong RAM.
_Avoid_: device session, persistent session

**Ready**:
Trạng thái Voice Session còn kết nối nhưng không nhận microphone audio.
_Avoid_: Idle, Listening

**Conversational Turn**:
Một lượt xử lý giọng nói có thể hủy độc lập trong một Voice Session.
_Avoid_: request, job

**Active Turn**:
Conversational Turn đã có utterance hoàn tất và đang giữ global capacity từ trước ASR đến terminal state.
_Avoid_: listening turn, queued turn

**Canonical Audio Profile**:
Wire-audio profile cố định của Compatibility Profile: uplink Opus 16 kHz mono 60 ms và downlink Opus 24 kHz mono 60 ms.
_Avoid_: negotiated audio params, supported audio formats

**Trace Session ID**:
UUID ngẫu nhiên chỉ dùng để tương quan telemetry của một Voice Session mà không ghi Device ID hay Client ID.
_Avoid_: device identifier, client identifier

**Discovered Tool**:
Tool mà thiết bị công bố qua MCP, chưa mặc nhiên được phép cho mô hình ngôn ngữ dùng.
_Avoid_: permitted tool

**LLM-visible Tool**:
Discovered Tool đã qua policy server và được phép đưa vào schema của mô hình ngôn ngữ.
_Avoid_: discovered tool, authorized tool

**Generated Assistant Response**:
Nội dung assistant đã được LLM tạo cho Conversational Turn nhưng chưa chắc đã được người dùng nghe hết.
_Avoid_: delivered response, dialogue assistant message

**Delivered Assistant Response**:
Generated Assistant Response chỉ trở thành một phần dialogue khi audio của nó đã drain hoàn toàn.
_Avoid_: partial response, cancelled response

**Exchange Atom**:
Đơn vị history không thể tách khi dựng prompt: một user-only turn hoặc toàn bộ chuỗi user, assistant tool call, tool result và delivered assistant response.
_Avoid_: message, partial exchange
