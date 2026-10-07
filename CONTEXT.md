# voice-agent-server

Voice-agent server cá nhân. Context này điều phối một Voice Session giữa một Voice Protocol Client và các provider AI.

## Language

## Quy ước cấu trúc mã nguồn

- Khi một file mã nguồn trở nên dài hoặc ôm nhiều trách nhiệm, phải tách thành các module theo trách nhiệm rõ ràng; giữ đường dẫn public ổn định bằng module cha và re-export khi cần.
- Test của một module phải nằm trong file test riêng thuộc module đó, thay vì tiếp tục làm phình file implementation. Test tích hợp/public-boundary vẫn đặt ở khu vực integration test phù hợp.

**Compatibility Profile**:
Tổ hợp voice wire protocol, audio/MCP contract và provenance external reference mà server cam kết tương thích.
_Avoid_: protocol version (khi chỉ nói đến phiên bản wire protocol), vendor-specific compatibility

**Voice Protocol Client**:
Một client phần mềm hoặc phần cứng tuân thủ voice wire protocol và Compatibility Profile. Có thể là firmware, ứng dụng Linux hoặc Rust/Python Reference Client.
_Avoid_: vendor simulator, vendor client

**Reference Client**:
Một Voice Protocol Client độc lập dùng để kiểm thử protocol conformance mà không cần phần cứng reference.
_Avoid_: firmware simulator, fake hardware

**Reference Integration Client**:
Một client qualification độc lập kiểm tra public control-plane và Voice Protocol contracts theo scenario deterministic; không phải Admin CLI vận hành tổng quát.
_Avoid_: operations console, production management CLI, server-side test harness

**Qualification Provider**:
Provider Adapter deterministic chỉ được compile vào qualification build, nhưng đi qua cùng desired-configuration, startup load plan, Runtime Catalog, diagnostic và Voice Session boundaries như production provider.
_Avoid_: injected ProviderSet, runtime fake switch, external smoke provider

**Qualification Build**:
Build của production `voice-agent-server` entrypoint với compile-time Qualification Providers; không phải default/release artifact và không thể được bật bằng runtime configuration.
_Avoid_: deployed release binary, test-only server binary, runtime qualification mode

**Integration Harness**:
Owner của Mandatory Qualification automated: tạo môi trường tạm, spawn production binary, điều phối controlled restart và deterministic doubles, rồi chạy Reference Integration Client qua public boundary. Không rebuild `AppState` trong process để giả lập restart.
_Avoid_: in-process restart helper, server-side test seam, operations CLI

**Process Startup Handshake**:
Machine-readable nonce-bound artifact mà production process exclusive-create và publish atomically sau listener bind để Integration Harness khám phá đúng child/address trước khi probe Readiness.
_Avoid_: human-log parsing, reserved-port handoff, PID-only identity, file existence as readiness

**Scenario State**:
Artifact handoff immutable, create-new và được validate trước side effect giữa lifetime hai process của Integration Harness. Chỉ giữ public resource identity, revision, run/spec identity và runtime observation cần thiết; không giữ credential, secret reference, authorization header, prompt, result hay audio.
_Avoid_: resume journal, mutable provisioning cache, secret store

**Scenario Plan**:
Kế hoạch immutable được materialize và validate từ ScenarioSpec trước bất kỳ provisioning side effect nào, chứa identity run và resource graph dự kiến nhưng chưa khẳng định resource đã tồn tại.
_Avoid_: Scenario State, partial provisioning record, retry journal

**Reference Client Wire Type**:
Biểu diễn request/response public contract do Reference Integration Client sở hữu độc lập với server implementation types, để API drift trở thành failure quan sát được.
_Avoid_: shared repository row, imported server handler DTO, server domain type

**Mandatory Qualification**:
Gate compatibility deterministic bắt buộc, chạy qua public boundary bằng provider doubles hoặc local fixtures và không phụ thuộc credential, remote service, model thật hay hardware.
_Avoid_: real-environment smoke test, optional runtime evidence

**Qualification Deadline**:
Hard upper bound của toàn Mandatory Qualification; remaining time của nó giới hạn mọi stage deadline và khi hết luôn tạo failure trước teardown.
_Avoid_: sum of stage timeouts, advisory timeout, per-request timeout

**Qualification Report**:
Artifact JSON versioned, privacy-safe và machine-readable ghi mandatory result, stage outcomes, optional evidence và cleanup mà không chứa nội dung request/response nhạy cảm.
_Avoid_: raw log archive, transcript report, debug dump

**Optional Runtime Evidence**:
Bằng chứng report riêng từ provider, remote service hoặc hardware thật; trạng thái `PASS`, `FAIL` hay `NOT_RUN` không thay đổi Mandatory Qualification.
_Avoid_: completion gate, required CI evidence

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

**Turn ID**:
Identity tăng đơn điệu, không tái sử dụng trong một Voice Session, được cấp khi utterance qua terminal boundary và được Active Turn admission chấp nhận; identity này theo lượt qua ASR finalization, LLM và speech delivery.
_Avoid_: Generation ID, ASR stream identity, VAD Capture Cycle ID

**Active Turn**:
Conversational Turn đã qua utterance terminal boundary và đang giữ global capacity từ ASR finalization đến terminal state.
_Avoid_: listening turn, queued turn

**History-Barrier Turn**:
Active Turn đã qua utterance terminal boundary và giữ capacity, nhưng accepted user text chưa được commit hoặc đưa vào LLM vì Conversational Turn trước chưa có terminal writer outcome.
_Avoid_: queued turn, capacity-free pending turn

**Active Turn Limiter**:
Capacity domain toàn application giới hạn số Active Turn đồng thời, độc lập với capacity provider worker.
_Avoid_: ASR semaphore, provider limit

**ASR Stream Lease**:
Quyền capacity dành riêng cho một recognition stream đang mở, từ lúc bắt đầu thu cho tới khi ASR final, cancel hoặc lỗi.
_Avoid_: Active Turn, ASR queue slot

**ASR Stream Identity**:
Identity của một recognition stream, có thể tồn tại trước Turn ID; ASR final sau utterance terminal boundary được liên kết với Turn ID của lượt đã nhận stream đó.
_Avoid_: Turn ID, ASR Stream Lease

**Semantic ASR Ownership**:
Quyền duy nhất để ASR event tạo accepted user text; quyền này bị revoke ngay khi Detect được accept, độc lập với physical worker cleanup.
_Avoid_: ASR Stream Lease, cleanup acknowledgement

**ASR Cleanup Obligation**:
Identity cleanup-only của ASR stream đã detach semantic ownership nhưng worker chưa xác nhận terminal. Mọi cleanup timeout của obligation vẫn fail closed Voice Session, kể cả qua generation mới.
_Avoid_: active ASR stream, stale event ignored

**Inbound Session-Scoped Command**:
Listen hoặc Abort mang `session_id` optional trong V1. Field missing/rỗng được compatibility accept; string non-empty phải khớp Voice Session, còn field present sai JSON type là Unknown.
_Avoid_: authentication credential, mandatory V1 session ID

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

**Acoustic Barge-in**:
Interruption của một assistant turn đang `Speaking`, chỉ do `SpeechStart` từ VAD của microphone uplink đã được client AEC/echo-suppressed khai báo và server tin cậy cho phép. Nó snapshot PCM giữ lại trước, rồi invalidate turn cũ, gửi đúng một urgent `tts:stop` nếu playback đã bắt đầu, và mở ASR cho turn mới.
_Avoid_: server-side AEC, any microphone packet, explicit abort

**VAD Capture Cycle**:
Chu kỳ semantic có identity riêng của VAD gồm timeline PCM, segmenter và retention; khác lifetime của VAD worker lease và khác Conversational Turn. Event semantic chỉ hợp lệ cho cycle hiện hành; cleanup acknowledgement vẫn xử lý khi cycle đã stale.
_Avoid_: VAD worker identity, turn generation, reset requested

**Generation ID**:
Epoch dùng để vô hiệu hóa output khi một lượt bị hủy hoặc ngắt; nhiều Conversational Turn hoàn tất bình thường có thể cùng Generation ID.
_Avoid_: Turn ID, operation identity

**Echo-safe Client Assertion**:
`features.aec=true` trong ClientHello là assertion client uplink đã echo-suppressed, không phải bằng chứng server-side AEC. Nó chỉ cho Acoustic Barge-in khi cả `barge_in.enabled` lẫn `barge_in.trust_client_aec_feature` được bật.
_Avoid_: verified server AEC, capability unconditionally trusted

**Dialogue History**:
Lịch sử Exchange Atom trong RAM thuộc một Voice Session; user message được commit sau ASR final non-empty. Số message là eviction target, còn request có hard byte bound riêng.
_Avoid_: persistent memory, transcript log

**Agent Persona**:
Cấu hình deployment định hình tên, vai trò và phong cách của trợ lý trong Voice Session; không chứa credential hay trạng thái provider.
_Avoid_: OpenAI prompt, provider prompt, model personality

**Prompt Template**:
Khuôn mẫu do deployment quản lý để kết hợp Agent Persona với quy tắc hội thoại giọng nói thành system message cho mỗi LLM Operation.
_Avoid_: provider request template, Dialogue History

**Prompt/LLM Base Snapshot**:
Tập message bất biến của một Conversational Turn gồm system snapshot, các Exchange Atom đã commit trước turn và current User đã commit; tool continuation chỉ nối completed tool prefix vào tập này.
_Avoid_: per-round history rebuild, mutable provider prompt

**Provider Asset Declaration**:
Module `assets.rs` cạnh một local provider khai báo authoritative URL upstream đã pin, revision, relative install path của từng file model, danh sách voice, và phép chuyển đổi riêng của provider nếu upstream format cần nó. Không có tài liệu manifest trung tâm; provider tự là nguồn.
_Avoid_: model manifest, artifact registry, database asset row

**Provider Asset Manager**:
Phần của Provider Adapter Registration chịu trách nhiệm làm cho file model của provider tồn tại. Provider Runtime Manager gọi `ensure_assets()` khi materialize provider; Provider Factory sau đó resolve path và build, không tải thêm.
_Avoid_: provider download, startup model scan, asset framework

**Provider Asset**:
Một file model mà provider cần, khai báo trong Provider Asset Declaration. Sẵn sàng khi nó là regular file và có kích thước lớn hơn 0 — không checksum, không fingerprint, không parse ONNX. Model có nạp được hay không do runtime initialization xác nhận.
_Avoid_: verified artifact, immutable install, content-addressed copy

**Provider Asset Download**:
Một lần tải asset ghi vào `<target>.part`, kiểm kích thước khác 0, rồi atomic rename vào final path; mỗi asset giữ một striped lock nên hai request materializing cùng provider không tải trùng file. File final không bao giờ tồn tại ở trạng thái dở dang, và một lần thất bại để lại gì để retry sạch.
_Avoid_: partial final file, unlocked concurrent download

**Typed Provider Configuration**:
Cấu hình selection adapter bằng `[providers.<kind>].adapter` và cấu hình concrete dưới bảng cùng tên adapter; startup chỉ chấp nhận bảng khớp adapter được compile vào binary.
_Avoid_: adapter_config table, compatibility parser, runtime provider discovery

**Worker Runtime Configuration**:
Cấu hình `[workers.vad]`, `[workers.asr]` hoặc `[workers.tts]` điều khiển capacity, mailbox, timeout, cleanup và quarantine của Inference Worker Runtime; không chứa model hoặc inference option.
_Avoid_: provider config, model option, adapter setting

**Unexpected Tool Call**:
Tool call mà LLM phát trong một round không được cấp tool definitions; đây là terminal failure của generation, không phải Device MCP request.
_Avoid_: implicit tool request, unsupported tool fallback

**Speech Segment**:
Đơn vị text speakable, được Sentence Segmenter tách từ Generated Assistant Response theo delivery policy và submit nguyên tử vào SpeechOutput.
_Avoid_: token, full response, TTS chunk

**LLM Operation**:
Một streaming request theo đúng Voice Session và generation, giữ một global LLM permit từ lúc runtime accept tới terminal event; không phải persistent provider session.
_Avoid_: LLM worker session, global chat, provider connection

**Speech Output Backpressure**:
Trạng thái tạm dừng đọc LLM delta khi hard-bounded pending Speech Segment queue hết capacity; actor giữ tối đa một delta đang xử lý dở và tiếp tục sau khi TTS lấy bớt segment. Buffer câu chưa kết thúc vượt ngưỡng khẩn cấp vẫn là terminal failure; không được bỏ hoặc overwrite segment.
_Avoid_: skipped sentence, best-effort speech queue

**Provider Adapter**:
Implementation compile-time của một provider trait, được chọn bằng typed provider configuration khi materialize runtime. Legacy deployment mode chọn adapter tại startup; managed runtime mode có thể materialize adapter tại acquisition. Adapter không biết Voice Session, WebSocket hoặc worker runtime.
_Avoid_: dynamic plugin, provider platform, service locator

**Provider Instance**:
Một cấu hình có ID ổn định của đúng một Provider Adapter; ID là giá trị agent bind, còn adapter chỉ định implementation.
_Avoid_: adapter name, active provider

**Provider Catalog**:
Tập read-only các Provider Instance đã materialize, được index theo Provider Instance ID và không thuộc Voice Session. Legacy deployment mode build catalog tại startup; managed runtime mode giữ catalog deployment rỗng và resolve exact Provider Version qua Provider Runtime Manager.
_Avoid_: adapter registry, session provider map

**Effective Provider Bindings**:
Tập Provider Instance ID hoàn chỉnh sau khi materialize provider defaults và agent override tại Config boundary.
_Avoid_: runtime fallback, adapter binding

**Runtime Catalog**:
Tập read-only Inference Worker Runtime theo Provider Instance ID; nó resolve Effective Provider Bindings thành runtime snapshot trước khi tạo Voice Session.
_Avoid_: SessionActor provider lookup, runtime plugin registry

**Runtime Snapshot**:
Các runtime concrete được resolve một lần cho Voice Session, giữ ổn định trong connection đó.
_Avoid_: hot-switched segment runtime, catalog-aware SessionActor

**Provider Benchmark**:
Developer CLI chạy cùng fixed, versioned TTS workload, selected Typed Provider Configuration và run policy ở hai mode để đo initialization và steady-state processing trên hardware hiện tại. Nó tách Provider Asset Download và provider build/startup readiness (cold) khỏi workload warmup và measured run (steady); warmup không thuộc samples. Không sở hữu Voice Session, WebSocket hoặc pacing. Mode `provider` kết thúc ở PCM provider-facing; mode `delivery` dùng cùng PCM stream và cùng deterministic canonical downlink conversion với production, gồm fade, resample, framing, Opus encode và tail finalization, kết thúc ở canonical Opus packet cuối cùng sẵn sàng gửi. Mặc định chỉ in stdout; artifact JSON là opt-in, không chứa benchmark text, audio, filesystem path, secret hoặc deployment endpoint. Không còn phân biệt model đã verify với model đã tải, vì đó giờ là một khái niệm.
_Avoid_: correctness test, end-to-end latency benchmark, playback benchmark

**Provider Factory**:
Factory compile-time build một Provider Adapter từ typed provider configuration và, khi cần, Resolved Model; không tự acquire model hoặc biết Voice Session.
_Avoid_: provider downloader, runtime plugin factory

**Provider Registry**:
Tập Provider Factory được compile vào binary, lookup khi startup theo typed adapter selection và chỉ thay đổi khi build/restart; không discovery hay load code lúc runtime.
_Avoid_: dynamic plugin registry, service locator

**Logical Model Identity**:
Khoá model do typed provider configuration chọn, dùng để chọn đúng Provider Asset Declaration của adapter; nó không phải filesystem path hay tên thư mục, và không định nghĩa nội dung file.
_Avoid_: model directory, latest model, adapter name

**Model License**:
License của một model, khai báo cùng Provider Asset Declaration. ZeroTTS bundle codec Apache-2.0 nên dùng composite `MIT; bundled-codec=Apache-2.0`. License không phải config: một deployment không acknowledge hay override nó.
_Avoid_: license acknowledgement config, per-deployment license gate

**Phase Completion Gate**:
Gate bắt buộc để một phase được đánh dấu hoàn tất. Gate phải dùng boundary thực của phase; với Phase 3 là real-model Voice Protocol E2E Manual và Auto qua canonical Opus tới exactly one STT, còn Phase 6 là Reference Client MCP E2E qua WebSocket và SessionActor. Cả hai tách biệt implementation gate dùng fake provider.
_Avoid_: ignored smoke test, compile success, hardware dependency không thuộc Compatibility Profile

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

**Device MCP Server**:
Capability MCP do một Voice Protocol Client sở hữu, công bố tool qua `initialize` và `tools/list`, rồi thực thi `tools/call` trong phạm vi state của chính client.
_Avoid_: server-side MCP plugin, ESP32-only capability, global tool registry

**Reference Client MCP Gate**:
Phase Completion Gate của Device MCP: Reference Client vừa dùng WebSocket voice protocol thật vừa làm Device MCP Server deterministic, thực thi tối thiểu một tool stateful và chứng minh tool result đi qua LLM continuation tới final TTS lifecycle.
_Avoid_: mocked actor response, ESP32 HIL requirement, fixed firmware tool catalog

**Generated Assistant Response**:
Nội dung assistant đã được LLM tạo cho Conversational Turn nhưng chưa chắc đã được người dùng nghe hết.
_Avoid_: delivered response, dialogue assistant message

**Delivered Assistant Response**:
Generated Assistant Response chỉ trở thành một phần dialogue khi WebSocket writer đã gửi thành công toàn bộ audio của lượt và normal `tts:stop` theo đúng thứ tự; không hàm ý client đã phát xong.
_Avoid_: partial response, cancelled response, client playback complete

**Persistent Transcript**:
Bản ghi tùy chọn, có retention, của final user text và Delivered Assistant Response theo một Voice Session; không phải Dialogue History trong RAM và mặc định tắt.
_Avoid_: dialogue history, full conversation log, audio archive

**Database-backed Device Admission**:
Chính sách bắt buộc tại WebSocket boundary: mọi Voice Protocol Client phải resolve Device đã provision trong SQLite. Database là startup dependency luôn có; không có cờ tắt Device admission hoặc đường WS bỏ qua database.
_Avoid_: implicit device registration, migration-based admission

**Database Desired Configuration**:
Cấu hình persistent mà admin đã yêu cầu cho provider/template, có thể chưa có hiệu lực trong process đang chạy.
_Avoid_: loaded runtime, active runtime configuration

**Loaded Runtime**:
Backing resource đã hoàn tất validation, native readiness và warmup. Provider Runtime Manager giữ resource theo generation và RAM budget; lease giữ resource usable cho admitted snapshot. Legacy deployment mode vẫn dùng process-lifetime Runtime Catalog.
_Avoid_: database desired configuration, hot-reloaded provider

**Effective Session Profile**:
Snapshot immutable của Device, Agent, Template tùy chọn, Provider bindings, prompt/language và MCP bindings được resolve trước khi Voice Session bắt đầu.
_Avoid_: per-frame database lookup, mutable agent configuration

**Template Switch Catalog**:
Tập immutable các configured Template Profile enabled, hợp lệ và exact desired provider snapshots khi admit. Chỉ selected profile được acquire trước upgrade; switch prepare acquire cold candidate ngoài actor rồi commit sau writer/history/native cleanup barrier. Legacy injected catalog có thể chứa resolved profiles.
_Avoid_: live template query, pending restart candidate, mutable assignment list

**Session Profile Revision**:
Counter lifecycle chỉ trong một Voice Session, tăng khi một Template Switch thành công được apply tại normal turn boundary.
_Avoid_: template revision, provider revision, database row version

**External MCP Binding**:
Liên kết của Agent với một MCP Streamable HTTP server, cung cấp snapshot tool LLM-visible tách khỏi Device MCP Server.
_Avoid_: Device MCP Server, global tool registry, verified stale tool cache

**External MCP Protocol Engine**:
Thành phần thực thi protocol lifecycle và message transport cho một External MCP client, tách khỏi Database Desired Configuration, outbound security policy và Voice Session lifecycle.
_Avoid_: database repository, arbitrary HTTP client, SessionActor configuration source

**Tool Origin**:
Định danh có kiểu của capability tool trước khi route execution, phân biệt Device MCP original name với External MCP server key và original name.
_Avoid_: LLM-visible name as authority, bind-order routing

**External Tool Segment**:
Một server key hoặc original MCP tool name được normalize độc lập, bounded và deterministic để tạo LLM-visible External MCP tool name.
_Avoid_: vendor hierarchy inference, hash collision suffix, truncated name

**Expected Revision**:
Revision immutable mà Admin API client trình bày để conditional mutate một Database Desired Configuration.
_Avoid_: last write wins, Session Profile Revision, database migration version

**Secret Reference**:
Identifier opaque printable ASCII `1..=256` bytes trỏ đến secret deployment-owned dùng bởi Provider hoặc External MCP, không phải secret value và không được đọc lại qua Admin API.
_Avoid_: API key field, resolver-specific syntax, trim/normalization, secret value

**Secret Resolver**:
Abstraction deployment-owned được bootstrap inject để biến Secret Reference thành Secret Value tại runtime; V1 backend là environment variables.
_Avoid_: repository reads environment directly, SQLite secret storage, Admin API resolution endpoint

**Secret Value**:
Wrapper runtime chỉ expose credential cho request/provider construction và redacts `Debug` output.
_Avoid_: ordinary debug string, clone/display implementation, telemetry label, persisted configuration

**Secret Rotation Snapshot**:
Credential lifecycle snapshot: backing Provider Runtime giữ resolved secret trong resource lifetime; admitted session giữ resource lease, External MCP Client giữ secret đến disconnect. Resolver hiện chưa cung cấp credential generation nên remote/authenticated resources không share giữa desired versions; không silently rotate secret của resource đang dùng.
_Avoid_: per-request secret resolution, silent credential replacement, runtime failure refresh

**Credential-free Provider Config**:
Canonical serialization của typed adapter configuration không chứa credential; credential Provider chỉ qua Secret Reference và Secret Resolver.
_Avoid_: arbitrary JSON bag, adapter-owned api key field, plaintext header/options escape hatch

**Provider Config Shape**:
Resource-abuse boundary chung của Provider config: raw UTF-8 bytes, JSON depth và aggregate object-key/array-item nodes trước typed validation.
_Avoid_: deep JSON allocation, separate Admin/startup validity rules, semantic adapter limits

**Forward-only Schema Migration**:
SQLx migration history monotonic mà binary chỉ được migrate tiến; binary cũ gặp schema mới hơn phải fail trước listener.
_Avoid_: automatic downgrade, unknown-schema best effort, application-owned backup restore

**SQLite Lock Contention**:
Lock SQLite còn tồn tại sau busy timeout, tách khỏi SQLx pool exhaustion và storage unavailable; không được retry ở application layer.
_Avoid_: pool timeout named busy, transaction retry, SessionActor database wait

**Admin JSON Transport Boundary**:
Shared pre-deserialization protection của Admin mutation body: content encoding/type và raw size limit, trước domain validation.
_Avoid_: handler-local body checks, decompression bypass, parser error/body logging

**Single-owner SQLite Deployment**:
Một Voice Agent process duy nhất sở hữu local SQLite database path trong V1.
_Avoid_: active-active writer, NFS/SMB database, implicit migration leader election

**Patch Field Intent**:
Ý định update typed phân biệt field vắng mặt, set value và clear explicit, trước domain validation.
_Avoid_: JSON Merge Patch, nested Option ambiguity, null clears immutable field

**Readiness**:
Khả năng process nhận connection mới bằng các dependency application-owned, tách process liveness và không probe External MCP optional.
_Avoid_: full admission probe, per-device resolution, optional MCP availability gate

**Admission Gate**:
Một gate application-owned duy nhất quyết định công việc mới có được bắt đầu hay không: listener mới, DB admission mới và Tool-round work mới. Shutdown đóng nó một lần trước khi drain, và không SessionActor nào phải quan sát shutdown trước khi nó đóng.
_Avoid_: per-session shutdown flag, cancel token thay gate, admission check lặp lại ở từng component

**Session Drain Registry**:
Registry application-owned của các Voice Session đã nhận và chưa xong, mỗi entry giữ một completion handle đăng ký trước khi connection bắt đầu làm việc, để shutdown quan sát drain completion và phát controlled close cho đúng những session còn mở tại deadline.
_Avoid_: task abort, broadcast không đếm, đếm session theo ước lượng

**Controlled Close**:
Close protocol do chính Voice Session thực hiện khi drain deadline tới hoặc process dừng, khác với abort cưỡng bức task; client vẫn nhận close code bình thường.
_Avoid_: task abort, drop socket im lặng, ungraceful server shutdown

**Liveness**:
Câu hỏi duy nhất process còn chạy hay không, không phụ thuộc database, External MCP hay shutdown; `/health` chỉ trả lời điều này.
_Avoid_: readiness synonym, dependency-aware health check, restart khi database hỏng

**History Purge**:
Thao tác destructive tường minh xóa Persistent Transcript trong scope Device, Voice Session hoặc toàn bộ archive, độc lập Dialogue History của session đang mở.
_Avoid_: side effect of disabling Device, implicit transcript delete, session memory reset

**Resource Key**:
Public resource identity ổn định, lowercase ASCII và immutable của Agent, Template, Provider hoặc MCP Server; database primary key chỉ là implementation detail. Agent, Template và MCP Server do client chọn khi tạo; Provider Key do server sinh ở thời điểm tạo theo format `{provider_type}_{uuid32}` vì Provider name có thể trùng và đổi.
_Avoid_: mutable display name, client-supplied Provider key, identity derived from display name, case-insensitive alias, database primary key

**Protocol Device Identity**:
Identity opaque và immutable do Voice Protocol Client cung cấp để provision Device, được so sánh byte-preserving tại database boundary.
_Avoid_: normalized MAC address, display name, Client ID

**Device Enrollment**:
Bản ghi control-plane SQLite ngắn hạn liên kết một Protocol Device Identity chưa đăng ký với một Activation Code và metadata đã scrub; không phải credential, Voice Session hoặc audio state.
_Avoid_: device authentication, session record, audio-pipeline cache

**Activation Code**:
Chuỗi 6 chữ số ASCII sinh CSPRNG, TTL-bound và chỉ được dùng tối đa một lần để Admin claim Device.
_Avoid_: device token, password, Device ID

**Enrollment Session**:
Kết nối WebSocket control-plane của Device chưa đăng ký, chỉ hiển thị/phát Activation
Code và quan sát Enrollment Claim; không có Effective Session Profile, provider,
transcript hoặc quyền hội thoại. Kết nối Voice Session mới thực hiện admission sau claim.
_Avoid_: anonymous Voice Session, temporary Agent, provider fallback

**Enrollment Claim**:
Transaction Admin tạo Device enabled, consume đúng một Device Enrollment và ghi audit tối thiểu; không tải provider/runtime hoặc xác nhận thiết bị đang online.
_Avoid_: WebSocket admission, runtime warmup, online presence

**External MCP Network Policy**:
Allowlist hostname/CIDR và scheme policy kiểm soát destination outbound của External MCP sau DNS resolution.
_Avoid_: arbitrary admin URL, hostname-only validation, redirect destination trust

**External MCP Authentication**:
Auth configuration có kiểu `none`, `bearer` hoặc một header an toàn, kết hợp Secret Reference deployment-owned để inject credential lúc request.
_Avoid_: query-string auth, template header value, Authorization header override

**Admin Audit Event**:
Metadata bounded ghi nhận mutation hoặc authenticated optimistic-concurrency conflict của Admin API, không sao chép nội dung resource hay secret.
_Avoid_: request archive, configuration diff, authentication failure record

**Provider Load Plan**:
Phân hoạch startup provider thành required phải materialize trước listener và optional được thử load để làm switch candidate khả dụng mà không chặn boot.
_Avoid_: all-enabled provider preload, non-default provider skip forever, duplicate load

**Runtime Status**:
Trạng thái usable của Loaded Runtime trong process, tách khỏi việc runtime đó có khớp Database Desired Configuration revision hiện tại không.
_Avoid_: desired-state freshness, provider enabled flag, restart completion

**Admin Request ID**:
UUID do server tạo cho một request Admin API để correlation response, telemetry và audit, không do client điều khiển.
_Avoid_: client correlation identifier, database primary key, authentication credential

**External Tool Call**:
Một logical LLM ToolCall routed tới External MCP, có tối đa một outbound attempt và luôn kết thúc bằng normal hoặc typed synthetic ToolResult.
_Avoid_: retried HTTP request, dangling tool call, remote error passthrough

**Tool-round Executor**:
Owner chung thực thi ToolCall trong đúng model order và ghép từng terminal ToolResult cùng index trước LLM continuation.
_Avoid_: origin-specific scheduler, parallel tool batch, reordered tool result

**Tool Execution Budget**:
Budget thời gian session-local của một Conversational Turn cho Tool-round Executor, bắt đầu ở ToolCall đầu tiên và giới hạn việc bắt đầu call tiếp theo.
_Avoid_: per-call timeout only, unbounded tool loop, audio pipeline budget

**Session Tool Catalog**:
Snapshot immutable của toàn bộ LLM-visible tools đã được resolve và validate khi admission, giữ nguyên đến khi Voice Session disconnect.
_Avoid_: remove capability sau một runtime failure, DB availability mutation từ tools/call telemetry, implicit circuit breaker

**External MCP Call Limiter**:
Semaphore process-global theo MCP server identity giới hạn outbound External Tool Call đồng thời giữa mọi Voice Session.
_Avoid_: per-session-only cap, unbounded cross-session fan-out, permit held during LLM continuation

**Admin API**:
Surface quản trị tùy chọn cho Database Desired Configuration và Persistent Transcript, chỉ mount khi được enable và luôn dùng credential riêng với Voice/OTA.
_Avoid_: Voice API, trusted-LAN anonymous endpoint, shared OTA token

**Admin Web**:
Ứng dụng Vue tùy chọn tại `apps/admin-web/` quản lý server qua Admin API công khai. Nó sở hữu trình bày và browser-side read model, nhưng không import Rust internal module, không đọc hoặc sửa `config.toml` trực tiếp, và không sở hữu Voice Session state. Khi Admin API không được enable hoặc bearer token không hợp lệ, UI không có quyền thay thế bằng một control path khác.
_Avoid_: server module, Admin API handler, direct SQLite/config editor, Voice Session owner

**Exchange Atom**:
Đơn vị Dialogue History không thể tách khi dựng prompt hoặc eviction: một user turn với các Completed Tool Round theo thứ tự, mỗi round gồm các cặp assistant tool call/tool result đã terminal, và Delivered Assistant Response nếu writer đóng turn Normal. Tool call chưa có terminal result không thuộc atom; turn lỗi trước tool đầu tiên là user-only atom.
_Avoid_: message, partial exchange

**ProviderVersion**:
Identity của một Database Desired Configuration của Provider Instance tại đúng desired revision, phân biệt source và không tái sử dụng sau delete/recreate. Voice Session sử dụng version đã snapshot tại admission.
_Avoid_: provider key alone, latest mutable provider, Session Profile Revision

**Runtime Resource Key**:
Opaque identity của backing runtime resource có resource specification tương đương theo hợp đồng Provider Adapter, bao gồm immutable artifact identity, execution settings, physical replica count và credential scope khi cần. Khác Resource Key public của Admin resource. Không chứa voice, language, Template hay Agent, vì các đó là logical selection.
_Avoid_: public provider key, raw JSON digest, path as model identity

**Physical Replica**:
Một bản sao resident của native engine mà một Runtime Resource sở hữu, khai báo bởi Provider Adapter chứ không phải operator. ZeroTTS giữ đúng một Physical Replica vì mỗi replica commit bốn ONNX session; generic worker concurrency không nhân bản nó. Số này là phần của Runtime Resource Key, nên đổi topology vẫn cô lập resource.
_Avoid_: engine instance per voice, worker count as replica count, template runtime

**Prepared Runtime**:
Kết quả mà preparation đã xác lập trước khi `build` chạy: các file model của provider đã tồn tại, kèm thời gian đã dùng. `build` chỉ resolve path và dựng runtime, nên một materialization không bao giờ tải lại cùng một model.
_Avoid_: prepared model cache, immutable model tree

**Physical Resource Key — model identity**:
Phần identity của một physical runtime đến từ adapter, MODEL_REVISION đã pin của provider, ONNX execution identity và thread count — không từ việc đọc model file. Bump revision tạo key khác mà không phải hash gì.
_Avoid_: model content hash, manifest fingerprint

**Resource Lease**:
Quyền sử dụng backing runtime resource được Provider Runtime Manager cấp cho Voice Session hoặc operation. Resource không thể unload khi quyền này hay cleanup obligation tương ứng còn tồn tại.
_Avoid_: inference permit, best-effort Arc count, runtime lookup without ownership

**Bounded Startup Warmup**:
Readiness pass chạy trên từng retained native worker, chỉ chạm mỗi graph trên hot path đúng một lần và xác minh PCM terminal finite non-empty, rồi reset trước traffic. Không synthesize utterance đầy đủ; gate deterministic full-utterance thuộc Optional Runtime Evidence, không thuộc startup path.
_Avoid_: full-sentence warmup, warmup as qualification, unbounded readiness loop

**Provider Runtime Manager**:
Application owner cấp runtime đúng ProviderVersion và giữ lifecycle/backing resources dùng chung trong các budget rõ ràng. Không sở hữu session history, stream state hay mutate Database Desired Configuration.
_Avoid_: runtime plugin registry, per-frame provider factory, automatic provider retry

## Speaker recognition

**Speaker Match**:
Kết quả đối chiếu giọng của đoạn audio được kiểm tra với Voiceprint theo calibration tương ứng; chưa phải quyền thực hiện yêu cầu và không chứng minh toàn bộ utterance do cùng một người nói.
_Avoid_: speaker authorization, authenticated request, liveness proof

**Speaker Authorization**:
Quyết định cho phép một Speaker sử dụng Agent và Template cho một voice turn, dựa trên Speaker Match mới cùng policy, grants và hiệu lực quyền. Quyền này chưa đủ để thực hiện thao tác nhạy cảm.
_Avoid_: speaker match, independent confirmation, blanket tool permission

**Independent Confirmation**:
Xác nhận riêng cho một thao tác nhạy cảm bằng căn cứ độc lập với Speaker Match của yêu cầu đó.
_Avoid_: repeated speaker match, spoken yes, Device authentication alone

**Preliminary Calibration**:
Calibration sơ bộ dùng thử nghiệm chất lượng audio, tính nhất quán của mẫu, holdout enrollment và Observe; chưa đủ điều kiện cho Speaker Authorization trong Required.
_Avoid_: Required-qualified calibration, production authorization threshold

**Required-qualified Calibration**:
Calibration được người vận hành xác nhận dựa trên báo cáo đánh giá độc lập cho đúng candidate set của Agent/Template, voiceprint revisions, embedding space, preprocessing, scoring parameters và điều kiện audio/tải. Chỉ đủ điều kiện Required khi qualification còn hiệu lực; subset của candidate set không tự được bao phủ.
_Avoid_: preliminary calibration, browser holdout alone, demo threshold

**Agent Tool Allowlist**:
Tập tool được admin đánh giá và cho phép cho một Agent, định danh theo Protocol Device Identity (`device_id`) hoặc External MCP server key cùng tên tool gốc; tool ngoài tập bị từ chối. Resource được tạo lại không kế thừa quyền của resource đã xóa. Đây là giới hạn thao tác của Agent, độc lập với Speaker Match và Speaker Authorization.
_Avoid_: speaker grants, LLM tool name allowlist, read-only safety inference

**Misidentification**:
Kết quả nhận diện 1:N chấp nhận một identity khác với người thực sự nói trong trial genuine; thuộc genuine failure dù hệ thống có trả match.
_Avoid_: genuine success, pure rejection, successful match

**Voice Pipeline Processing Permit**:
Quyền độc quyền xử lý pipeline của một Voice Session trong envelope pilot, gồm cả capture đang armed và công việc hội thoại chưa terminal. Khác Resource Lease giữ model và permit riêng của từng inference operation.
_Avoid_: Resource Lease, Active Turn Permit, speaker inference permit

**Reviewed Tool Contract**:
Contract quan sát được của một tool mà admin đã đánh giá, gồm identity nguồn, tên gốc, input schema, description có ảnh hưởng cách sử dụng và cấu hình nguồn liên quan. Review mất hiệu lực khi server quan sát contract thay đổi; không chứng minh hành vi implementation bên ngoài giữ nguyên.
_Avoid_: tool name alone, remote implementation attestation, secret value fingerprint
