# voice-agent-server

Voice-agent server cá nhân. Context này điều phối một Voice Session giữa một Voice Protocol Client và các provider AI.

## Language

**Compatibility Profile**:
Tổ hợp voice wire protocol, audio/MCP contract và provenance external reference mà server cam kết tương thích.
_Avoid_: protocol version (khi chỉ nói đến phiên bản wire protocol), vendor-specific compatibility

**Voice Protocol Client**:
Một client phần mềm hoặc phần cứng tuân thủ voice wire protocol và Compatibility Profile. Có thể là firmware, ứng dụng Linux hoặc Rust/Python Reference Client.
_Avoid_: vendor simulator, vendor client

**Reference Client**:
Một Voice Protocol Client độc lập dùng để kiểm thử protocol conformance mà không cần phần cứng reference.
_Avoid_: firmware simulator, fake hardware

**Firmware Baseline**:
Phiên bản firmware và commit đã pin dùng làm nguồn chân lý tương thích.
_Avoid_: upstream main, firmware mới nhất

**HIL Reference Profile**:
Thiết bị và cấu hình mạng vật lý chuẩn dùng để xác nhận tương thích firmware.
_Avoid_: Reference Client, simulated client

**Voice Session**:
Một kết nối WebSocket đang hoạt động, thuộc duy nhất một Device ID và mang dialogue chỉ trong RAM.
_Avoid_: device session, persistent session

**Ready**:
Trạng thái Voice Session còn kết nối nhưng không nhận microphone audio.
_Avoid_: Idle, Listening

**Processing**:
Trạng thái Voice Session đã endpoint một utterance và đang hoàn tất Conversational Turn; microphone audio bị drop cho tới terminal outcome.
_Avoid_: Listening, queued turn

**Listening Mode**:
Ý nghĩa capture do Voice Protocol Client khai báo trong `listen:start`; V1 có Manual, Auto và Realtime là các giá trị wire riêng, không được server suy đoán hoặc đổi thay thế.
_Avoid_: capture option, implicit manual mode

**Conversational Turn**:
Một lượt xử lý giọng nói có thể hủy độc lập trong một Voice Session.
_Avoid_: request, job

**Active Turn**:
Conversational Turn đã qua utterance terminal boundary và đang giữ global capacity từ ASR finalization đến terminal state.
_Avoid_: listening turn, queued turn

**Active Turn Limiter**:
Capacity domain toàn application giới hạn số Active Turn đồng thời, độc lập với capacity provider worker.
_Avoid_: ASR semaphore, provider limit

**ASR Stream Lease**:
Quyền capacity dành riêng cho một recognition stream đang mở, từ lúc bắt đầu thu cho tới khi ASR final, cancel hoặc lỗi.
_Avoid_: Active Turn, ASR queue slot

**Inference Worker Runtime**:
Nhóm worker bounded thuộc application, mỗi worker sở hữu mutable provider stream/session và chỉ trao đổi command/event mang identity; không sở hữu Voice Session state hay WebSocket.
_Avoid_: provider pool, background task, SessionActor worker

**VAD Probability**:
Kết quả inference Silero cho đúng một khoảng PCM liên tục của Auto cycle, gồm xác suất speech và sample cursor `[start_sample, end_sample)`; đây chưa phải ranh giới utterance.
_Avoid_: SpeechStart, SpeechEnd, speech decision

**VAD Stream Integrity Failure**:
Lỗi khi một Auto cycle nhận VAD Probability có gap, duplicate, thứ tự sai hoặc sample range không hợp lệ; Voice Session bị ảnh hưởng fail closed vì endpoint không còn đáng tin.
_Avoid_: dropped VAD frame, recoverable VAD delay

**Worker Cleanup Acknowledgement**:
Event xác nhận worker đã kết thúc hoặc reset mutable runtime của một lease, là điều kiện duy nhất để slot trở lại reusable; vẫn được xử lý khi generation logic đã stale.
_Avoid_: cancel requested, Drop, logical cancellation

**Dialogue History**:
Lịch sử message bounded, RAM-only thuộc một Voice Session; user message được commit sau ASR final non-empty.
_Avoid_: persistent memory, transcript log

**Model Artifact Manifest**:
Tài liệu versioned pin source, revision, license, tên upstream, tên cài đặt, checksum và derivation của từng model artifact; path directory không tự xác nhận model identity.
_Avoid_: model folder name, latest model

**Typed Provider Configuration**:
Cấu hình selection adapter bằng `[providers.<kind>].adapter` và cấu hình concrete dưới bảng cùng tên adapter; startup chỉ chấp nhận bảng khớp adapter được compile vào binary.
_Avoid_: adapter_config table, compatibility parser, runtime provider discovery

**Worker Runtime Configuration**:
Cấu hình `[workers.vad]` hoặc `[workers.asr]` điều khiển capacity, mailbox, timeout, cleanup và quarantine của Inference Worker Runtime; không chứa model hoặc inference option.
_Avoid_: provider config, model option, adapter setting

**Provider Adapter**:
Implementation compile-time của một provider trait, được chọn một lần tại startup bằng typed provider configuration; adapter không biết Voice Session, WebSocket hoặc worker runtime.
_Avoid_: dynamic plugin, provider platform, service locator

**Logical Model Identity**:
Khoá model do typed provider configuration chọn, dùng để lookup đúng entry authoritative trong Model Artifact Manifest; không phải filesystem path hoặc tên thư mục.
_Avoid_: model directory, latest model, adapter name

**Model License Acknowledgement**:
Khai báo deployment khớp chính xác logical model, revision và license trong Model Artifact Manifest; thiếu hoặc lệch thì server fail trước bind.
_Avoid_: license bypass, generic agreement flag

**Phase Completion Gate**:
Gate bắt buộc để một phase được đánh dấu hoàn tất; với Phase 3 là real-model Voice Protocol E2E Manual và Auto qua canonical Opus tới exactly one STT, tách biệt implementation gate dùng fake provider.
_Avoid_: ignored smoke test, compile success

**Canonical Audio Profile**:
Wire-audio profile cố định của Compatibility Profile: uplink Opus 16 kHz mono 60 ms và downlink Opus 24 kHz mono 60 ms.
_Avoid_: negotiated audio params, supported audio formats

**Uplink PCM Frame**:
Một khung PCM16 mono 16 kHz, đúng 960 samples, đã được giải mã từ đúng một Opus uplink packet trước khi được đưa vào một bộ thu audio.
_Avoid_: PCM Frame, audio bytes, sample chunk

**Downlink PCM Frame**:
Một khung PCM16 mono 24 kHz, đúng 1.440 samples, sẵn sàng để encode thành đúng một Opus downlink packet.
_Avoid_: PCM Frame, output chunk

**Uplink Audio Utterance**:
PCM16 mono 16 kHz canonical hoàn chỉnh của một lượt thu âm, chỉ được tạo khi một bộ thu kết thúc thành công.
_Avoid_: Audio Utterance, PCM buffer, partial capture

**Manual Capture**:
Bộ thu audio theo listen mode manual, sở hữu PCM và capacity của một lượt thu, rồi trả Uplink Audio Utterance hoặc outcome không có audio.
_Avoid_: actor buffer, manual listen buffer

**Uplink Audio Stream**:
Chuỗi Opus microphone liên tục trong suốt một Voice Session; ranh giới Conversational Turn hoặc Manual Capture không tạo stream mới.
_Avoid_: capture stream, turn stream

**Capture Outcome**:
Kết quả có kiểu của việc dừng một Manual Capture: Uplink Audio Utterance, empty hoặc overflowed.
_Avoid_: optional audio, capture status flag

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
