# Speaker Identification — Hướng dẫn refactor CAM++ built-in (sau Ponytail Review)

> **Dành cho:** coding agent triển khai tại [`hailp-vn38/ai-agent-voice`](https://github.com/hailp-vn38/ai-agent-voice). Đây là tài liệu thiết kế, **chưa phải code đã sửa**.
> **Code baseline đã kiểm tra:** `main` — `1b3e0ef2e9d8081243bd7f013eb79c19266c27c7` (2026-10-08). Kiểm tra HEAD/diff trước khi sửa.
> **Tham khảo:** [`xinnan-tech/voiceprint-api`](https://github.com/xinnan-tech/voiceprint-api) cho flow `WAV → embedding → database → cosine identify`; không port Python/MySQL/HTTP sidecar.
> **Quyết định:** Speaker chỉ nhận dạng tên người nói để cá nhân hóa một lượt hội thoại; **không xác thực bằng giọng nói, không cấp quyền**.
> **Revision:** 2 — áp dụng 9 findings Ponytail Review, ưu tiên xóa và tái sử dụng thay vì thêm module/abstraction.

## 0. Contract không thay đổi

1. Một CAM++ native extractor **mặc định của server**, một `SpeakerRuntime` chia sẻ giữa enrollment và WebSocket. Model/threads do server sở hữu. Không có Speaker Provider Instance trong SQLite, Speaker Provider CRUD/Prepare/Test, lựa chọn Provider trên Web hoặc binding Speaker Provider trong Template.
2. **Giữ nguyên trang Speakers**, Speaker Detail, tìm kiếm, sửa/xóa, microphone recorder và đăng ký Speaker. Chỉ thay behavior: thu **một** WAV → server xác nhận extract thành công → nhập tên/mô tả → Save → một Speaker + một embedding được lưu. Không yêu cầu mẫu thứ hai, holdout, calibration, provisional hay kiểm định giọng.
3. Agent có toggle **Speaker Recognition ON/OFF** và danh sách Speaker thuộc Agent. Giữ `mode='off'|'observe'` trong API/database để giảm thay đổi; `observe` chính là identification. Không có grant theo Template.
4. Khi có utterance từ ESP32, ASR và Speaker chạy song song. Kết quả (nếu match) được đưa vào **LLM prompt của đúng turn**, không gắn với session lâu dài. `unknown`, model lỗi, worker busy hoặc timeout đều tiếp tục hội thoại.
5. Không còn `required`, `SpeakerGate`, `calibration.json` và mọi kiểm tra speaker authority. Match không ảnh hưởng WebSocket admission, Device authentication, quyền MCP/Tool, Template switch hoặc lịch sử người dùng.
6. Giữ nguyên Device/WS/Admin authentication, VAD/ASR/LLM/TTS, lịch sử/History Barrier, cancellation và cơ chế bảo vệ tool độc lập với Speaker.
7. Forward migration **bảo toàn** Speaker metadata, voiceprint tương thích, và Agent candidates. Không sửa migration cũ đã áp dụng.

**Ngoài phạm vi:** liveness/voice authentication, FAR/FRR qualification, diarization, multi-sample voting, provider API cho Speaker, custom model URL/threads/threshold trên Admin Web, engine thứ hai, orchestration framework mới.

## 1. Ponytail decisions — cắt gì, giữ gì

| # | Thay đổi đã chốt | Thay thế/tái sử dụng |
|---|---|---|
| 1 | **Không tạo `BuiltinSpeakerEngine` wrapper.** | `Arc<workers::speaker::SpeakerRuntime>` trong `AppState`, sở hữu runtime/lease theo lifecycle sẵn có. |
| 2 | **Không tạo `IdentificationOutcome` DTO.** | Dùng `session/speaker_observe.rs::ObserveDiagnostic`, `SpeakerStatus`, `ObserveIdentity`; thêm mapping `speaker_id → name` tối thiểu. |
| 3 | **Không thêm global `enabled` flag** trùng toggle Agent; **không chọn giữa lazy và eager**. | Khởi tạo **một lần lúc bootstrap**; Agent `off` không retain PCM hoặc chạy inference. Khi engine lỗi, Speaker unavailable nhưng voice pipeline vẫn hoạt động. |
| 4 | **Không thêm `capture_ttl_ms`.** | Dùng `speaker_quick.rs::CAPTURE_TTL_SECONDS = 10 * 60` đang có. |
| 5 | **Không tạo bảng `speaker_captures`.** | Forward-migrate chính `speaker_quick_captures` hiện có (migration `0016`). |
| 6 | **Không để hai lựa chọn cho legacy `required`.** | Migration dứt khoát `required → off`; ghi release note. Admin có thể tự bật `observe`. |
| 7 | **Không thêm máy trạng thái `ready/preparing/unavailable`.** | Summary dùng `available: bool` hiện có + reason/error công khai ngắn nếu cần. Không thêm polling state machine. |
| 8 | **Không lưu `enrollment_status` mới trong SQLite hay bắt buộc thêm field API.** | Derived tại một mapper từ voiceprint có/không và `embedding_space_id` mặc định. |
| 9 | **Không tạo recognition revocation registry/catalog-generation mới.** | Giữ `TurnId/generation/operation_id` + cancellation hiện có. Mutation Speaker/Agent áp dụng cho **kết nối mới hoặc sau reconnect**; không biến Speaker thành security authority. |

**Nguyên tắc triển khai:** ưu tiên sửa module đã có và xóa đường đi cũ; không thêm `SpeakerProviderManager`, `SpeakerRecognitionCoordinator`, cache tầng hai, event bus, DTO pass-through hay migration để mô hình hóa trạng thái vốn suy ra được.

## 2. Điểm thay đổi trong codebase hiện tại

| Đường dẫn (dưới `crates/voice-agent-server/src/` nếu không ghi rõ) | Thực hiện |
|---|---|
| `providers/speaker/mod.rs`, `assets.rs`, `workers/speaker.rs` | **Giữ** extractor CAM++ và bounded native worker; không viết lại inference, cosine hoặc thread queue. |
| `app/state.rs`, `app/mod.rs` | Lưu một shared runtime trong AppState; bootstrap và shutdown theo lifecycle hiện có. |
| `providers/speaker/descriptor.rs`, `providers/factory_registry.rs`, `providers/database_loader.rs`, `providers/runtime_catalog.rs` | Bỏ Speaker khỏi generic Provider/Template catalog, load plan, admin descriptors và runtime resolution theo DB. Giữ native factory/build nội bộ nếu vẫn được dùng. |
| `config/mod.rs`, `config/providers.rs`, `config/validation.rs`, `config.example.toml` | Bỏ `[providers.speaker.instances.*]`, Template Speaker slot và calibration; chỉ giữ config giới hạn cần thiết ở `[speaker_recognition]`. |
| `app/admin/speaker_quick.rs` | Tái sử dụng `create_capture`, `create_speaker_from_capture`, TTL, validation, idempotency; loại Provider key/revision và `pending`. |
| `app/admin/speakers.rs` | Giữ Speaker CRUD/purge/list; rút phần drafts, holdout, finalize/validation không còn caller; derived status. |
| `app/admin/speaker_policy.rs` | Giữ Agent policy revision/ETag và candidate CRUD, bỏ `required`, qualification, `template_keys`, per-Template grants. |
| `session/speaker_observe.rs` | Giữ `ObservePlan`, scoring, diagnostics; lookup theo Agent/cùng embedding space, không theo Template grant. |
| `app/websocket.rs` | Resolve ON/OFF + AppState runtime + Agent candidate snapshot; bỏ Speaker `resolved.speaker`, `SpeakerSwitchGuard` và Required gate. |
| `session/actor/observe.rs`, `actor/listening.rs`, `actor/delivery.rs` | Join bounded ASR Final và Speaker diagnostic **trước LLM**, không tạo module coordinator mới. |
| `session/speaker_gate.rs`, `session/actor/speaker.rs`, `app/admin/speaker_calibration.rs` | Bỏ Required/Calibration paths sau khi xóa caller; chỉ giữ status sender nếu actor còn cần. |
| `apps/admin-web/src/views/SpeakersView.vue`, `pages/speakers/SpeakerDetailPage.vue` | Giữ trang/route/actions, đổi nhãn enrollment theo derived status. |
| `apps/admin-web/src/components/speakers/SpeakerEnrollmentWizard.vue`, `QuickSpeakerEnrollment.vue` | Chọn **một** implementation, giữ recorder; bỏ selector Provider, Quick/Validated, draft/holdout. |
| `apps/admin-web/src/components/agents/AgentSpeakerPolicy.vue` | Toggle + Agent Speaker picker, không có Template picker/Required blockers. |
| `apps/admin-web/src/api/*`, `docs/api/00-all-apis.postman_collection.json` | Đồng bộ DTO/API, bỏ Speaker Provider endpoints và ví dụ cũ. |

**Audit thêm usages:** `session/profile.rs`, `actor/construct.rs`, `actor/tools/*`, `database/load_plan.rs`, `database/tool_security.rs`, `providers/descriptor.rs`, registry, i18n/tests, compile feature `qualification-providers`.

## 3. Một SpeakerRuntime, không thêm wrapper

### 3.1 Internal interface

Không khai báo trait/DTO mới. Code hiện có đã sở hữu:

```rust
// Existing interfaces — reuse, do not duplicate.
crate::providers::speaker::SpeakerProvider::extract(...);
crate::workers::SpeakerRuntime::extract(pcm, lease).await;
crate::workers::SpeakerRuntime::embedding_space_id();
crate::session::speaker_observe::ObservePlan::score(&embedding);
```

- `SpeakerProvider`/CAM++ chỉ chịu trách nhiệm extract. `SpeakerRuntime` sở hữu worker native và bounded queue; `SpeakerObserve` xử lý score/quality/candidates. Không truyền Agent/Template vào extractor.
- **Một runtime trong `AppState`**, khởi tạo lúc bootstrap trên blocking thread theo startup pattern hiện có; giữ owner/lease nội bộ từ runtime manager khi dùng `SpeakerRuntime::extract(..., ResourceLease)`. `ResourceLease` vẫn cần để bảo đảm native worker sống đến khi request hoàn tất: **không xóa parameter hoặc request guard** chỉ để bỏ database provider.
- Nếu owner được dựng bằng `ProviderRuntimeManager`, tạo duy nhất một **internal static identity** của built-in Speaker để acquire/retain lease, không đăng ký nó thành DesiredProvider DB, Template binding hay generic Admin Provider. Không tạo thêm lớp wrapper pass-through quanh manager.
- Vẫn giữ admission, slot, queue 1, cancellation và cleanup của `workers/speaker.rs`. Không chạy native ONNX trên Tokio reactor.
- Lấy model từ `providers/speaker/assets.rs`, threads từ cấu hình ONNX runtime của server; không có user-defined provider key/revision/model URL/folder.
- `embedding_space_id` phải cùng một contract giữa enrollment và identification. Vector phải hữu hạn, đúng dims, norm > 0; reuse normalize/validate/cosine trong `audio/enrollment.rs`.

### 3.2 Bootstrap và availability

Chọn **eager, one-time bootstrap**, không đồng thời hỗ trợ lazy mode:

1. Validate server config và initialize normal providers/SQLite theo startup hiện có.
2. Nếu build CAM++ thành công, ghi một runtime/lease trong AppState và trả `available=true`.
3. Nếu model/native engine không thể khởi tạo, log reason và giữ Speaker unavailable; **không chặn WS/ASR/LLM/TTS**. Enrollment trả `speaker_unavailable`; Admin summary trả `available=false`.
4. Việc Speaker unavailable không làm sai trạng thái health/readiness của phần voice pipeline; không thiết kế thêm retry loop hay API Prepare.
5. Shutdown thả owner/lease theo lifecycle; chắc chắn worker native thoát đúng pattern hiện có.

Không tự tạo global `speaker_recognition.enabled`: engine là capability mặc định, còn ON/OFF là quyết định của **Agent**. Nếu về sau cần tắt tải model cho deployment, đó là yêu cầu riêng, không nằm trong refactor này.

### 3.3 Cấu hình tối thiểu

Thay cấu hình Speaker Provider cũ bằng các giới hạn đang thực sự được dùng:

```toml
[speaker_recognition]
similarity_threshold = 0.50  # heuristic cho identification, KHONG PHAI auth confidence
max_speakers = 256
max_candidates_per_agent = 32

[speaker_recognition.enrollment]
min_clip_ms = 2000
max_clip_ms = 10000
min_speech_ms = 1800
max_window_ms = 6000
max_audio_body_bytes = 524288
```

- Đây là giá trị **đề xuất sau refactor**; cập nhật parser/default/validation và recorder thống nhất. Threshold `0.50` chưa hiệu chuẩn; đánh giá trên audio thực khi triển khai.
- **Không thêm** `capture_ttl_ms`, provider key, provider revision, model, threads, URL, `calibration_source`, `min_samples`, `max_samples`, `max_voiceprint_spaces_per_speaker` (nếu không còn consumer); giữ giới hạn tải hữu ích, không mở thêm knobs ngoài nhu cầu.
- Giữ hằng `CAPTURE_TTL_SECONDS` và giới hạn số capture đang mở bên trong handler nếu cần; tận dụng quota cleanup hiện có.

## 4. SQLite và migration: bảo toàn dữ liệu, không thêm bảng capture

### 4.1 Schema logic sau refactor

```text
speakers                    # giữ ID/key/name/description/enabled/revision/timestamps
speaker_voiceprints         # speaker_id, embedding_space, dims, vector, revision, timestamps
agent_speaker_candidates    # agent_id, speaker_id, created_at
agent_speaker_policies      # mode ('off'|'observe'), revision
speaker_quick_captures      # staging + TTL + idempotent tombstone; không tạo bảng mới
```

Một Speaker có tối đa **một voiceprint cho mỗi embedding space** (ràng buộc UNIQUE hiện có). V1 chỉ dùng voiceprint thuộc **built-in active embedding space**; không cần voting hay nhiều sample trong một lần đăng ký.

**Không thêm cột `enrollment_status`:** chỉ định nghĩa một mapper dùng chung cho Speaker HTTP responses/Web:

- Có voiceprint hợp lệ đúng `embedding_space_id` + dims của runtime: **Đã đăng ký**.
- Không có voiceprint nào: **Chưa có mẫu giọng**.
- Chỉ có voiceprint khác embedding space/dims: **Cần thu lại mẫu**.
- Engine unavailable: UI có thể hiển thị **Không xác định khả năng tương thích** thay vì suy diễn speaker chưa đăng ký.

### 4.2 Forward migration (không sửa lịch sử)

1. Sao lưu SQLite, thống kê `speakers`, `speaker_voiceprints`, `agent_speaker_candidates`, `agent_speaker_policies`, `speaker_quick_captures`, Speaker Provider rows và Template grants. Chạy thử trên backup.
2. Tạo migration tiếp theo; **không sửa** `0007`, `0009`, `0012`, `0013`, `0014`, `0016` đã apply.
3. `speaker_voiceprints` cũ có `provider_id NOT NULL` FK tới `providers` cùng `provider_key/revision`, `browser_validation_status`, `calibration_revision` và `sample_count`; rebuild schema theo SQLite forward migration để **bỏ FK sang providers** mà giữ `speaker_id`, `embedding_space`, `dims`, vector, revision, timestamps, UNIQUE.
4. **Bảo toàn bytes vector cũ** nếu valid; chỉ match nếu `embedding_space_id` + dims + embedding contract thực sự trùng built-in runtime. Không dùng tên Provider hay `pending/passed` để đoán compatibility; không convert embedding khác model. Voiceprint không tương thích được giữ lại trong bảng voiceprints để quản trị/thu lại, nhưng không tham gia candidate matching.
5. Giữ `agent_speaker_candidates`; bỏ phụ thuộc `agent_speaker_template_grants` trong query và API. Không mất candidate hợp lệ chỉ vì grant Template cũ. Điều này **mở scope từ per-Template sang per-Agent** theo quyết định mới; ghi release notes.
6. **Migrate mọi `agent_speaker_policies.mode='required'` về `off`**, tăng revision nếu policy schema yêu cầu, có script thống kê/báo cáo Agent bị chuyển. Không tự bật `observe`; giá trị lạ fail validation thay vì tự nâng quyền hay ngầm bật.
7. Chính `speaker_quick_captures` (migration `0016`) được rebuild/bỏ FK `provider_id` và provider revision, **không tạo `speaker_captures`**. Dọn accepted capture cũ khi nâng version (client nhận `capture_expired`/`capture_not_found` rồi thu lại); bảo toàn committed tombstone còn hợp lệ nếu có thể và không phá idempotency.
8. Chỉ drop tables/cột cũ của draft, holdout, calibration, grants khi code không còn caller/foreign-key reference. Không xóa `providers(type='speaker')` trước khi bỏ toàn bộ FK phụ thuộc; sau đó mới dọn instance và Template speaker bindings cũ. Không thay đổi các Provider loại khác.
9. Kết thúc bằng `PRAGMA foreign_key_check`, kiểm kê trước/sau, migration test trên DB cũ, reboot và verify API. Không để delete/purge Speaker bị chặn bởi orphan FK.

**Không xây thêm archive table/cache/revision registry cho Speaker.** Một forward migration giữ metadata cũ và xóa dependency Provider là đủ; cleanup schema chia hai migration nếu dependency thực tế yêu cầu.

### 4.3 Capture → Save

1. `POST /speakers/captures`: Admin gửi WAV, server dùng `audio/enrollment.rs` để parse/quality check, normalize embedding từ shared `SpeakerRuntime`, lưu vector tạm vào **`speaker_quick_captures`** với UUID opaque + TTL 10 phút. Không tạo Speaker trước khi Save; không lưu raw WAV lâu dài.
2. Sau khi extraction thành công, Web hiển thị thành công và form Name/Description.
3. `POST /speakers/from-capture`: transaction tạo `speakers` + `speaker_voiceprints`, đánh dấu capture `committed`, xóa `vector` khỏi tombstone, audit, bump revisions theo helper hiện hữu. Nếu lỗi rollback; retry capture committed trả cùng Speaker (idempotent behavior hiện có).
4. Re-enroll Speaker hiện hữu: dùng **cùng** capture endpoint và `PUT /speakers/{key}/voiceprint` + `If-Match` speaker revision để thay vector đúng active space. Không tạo profile thứ hai; không giữ sample bổ sung.
5. Capture quá hạn báo lỗi rõ và cho thu lại, không để Save một vector đã hết hạn. Không xóa/sửa Speaker metadata khi extract thất bại.

## 5. Admin API — chỉ quản lý Speakers, không quản lý Speaker Provider

Tất cả endpoint dưới `/api/admin`, vẫn dùng Admin Bearer, body cap, ETag/If-Match và audit hiện có.

| Method | Path | Contract sau refactor |
|---|---|---|
| `GET` | `/speaker-recognition` | `{available, embedding_space_id?, enrollment limits, speaker limits}`; **không có** mảng `providers`, không thêm state machine. |
| `POST` | `/speakers/captures` | `audio/wav` raw bytes → stage embedding, trả `{status:'accepted', capture_id, quality, expires_at}`. Không nhận provider key/revision. |
| `POST` | `/speakers/from-capture` | `{capture_id,name,description?}` → tạo Speaker + voiceprint atomically; giữ route hiện hữu. |
| `GET/POST` | `/speakers` | List/create Speaker metadata; profile mới chưa có voiceprint vẫn được hỗ trợ. |
| `GET/PATCH/DELETE` | `/speakers/{key}` | Read/edit/delete profile; giữ CAS. |
| `PUT` | `/speakers/{key}/voiceprint` | `{capture_id}` + `If-Match` → re-enroll bằng một mẫu; quyết định **PUT**, không để implementer chọn method. |
| `POST` | `/speakers/{key}/voiceprint/purge` | Xóa voiceprint, giữ Speaker profile và bindings. |
| `GET/PUT` | `/agents/{key}/speaker-policy` | `mode='off'|'observe'` + revision/ETag; bỏ `required_available`, `required_blockers`, qualification. |
| `GET` | `/agents/{key}/speakers` | Agent candidates và `usable` suy ra từ profile/voiceprint. |
| `PUT/DELETE` | `/agents/{key}/speakers/{speaker_key}` | Gắn/gỡ Speaker trực tiếp Agent; `PUT` body `{}` + `If-Match` agent revision; không còn `template_keys`. |
| `GET` | `/speakers/{key}/bindings` | Agent list; không trả Template grants. |

Loại bỏ routes Speaker-specific không còn caller:

```text
POST /providers/{key}/test/speaker
POST /providers/{key}/speaker-captures
POST /speaker-recognition/reload
POST /speakers/{key}/enrollments
GET/DELETE /speakers/{key}/enrollments/{id}
PUT/DELETE /speakers/{key}/enrollments/{id}/samples/{slot}
POST /speakers/{key}/enrollments/{id}/validate
POST /speakers/{key}/enrollments/{id}/finalize
PUT/DELETE /templates/{key}/providers/speaker
```

- Generic `/providers`, `/provider-adapters`, Prepare/Test **vẫn hoạt động cho VAD/ASR/LLM/TTS**; mọi create/update/list/filter không tiếp nhận `speaker` như một mutable Provider. `POST /providers` với `type='speaker'` trả validation error `unsupported_provider_type`; không tạo instance.
- Không thêm API đăng ký builtin engine, upload nhiều mẫu, xác minh giọng hay query runtime riêng.
- `GET /speakers` và `GET /speakers/{key}` chỉ cần giữ `voiceprints` tương thích contract hiện có, mapper derived status không cần field DB mới. Không expose embedding, raw audio, model path hoặc token.
- Khi thay endpoint cần cập nhật **cùng lúc** `apps/admin-web/src/api/speakers.ts`, `types/speakers.ts`, `types/speaker-policy.ts`, `agents.ts`, API tests và `docs/api/00-all-apis.postman_collection.json`.

## 6. Web — giữ trang Speakers, rút ngắn wizard

### Trang Speakers / Speaker Detail

Giữ `SpeakersView.vue`, `SpeakerDetailPage.vue`, sidebar route, search, create metadata, edit, delete, purge, bindings và bố cục tổng thể. Thay nhãn:

- `Đã đăng ký` — có voiceprint tương thích.
- `Chưa có mẫu` — Speaker profile chưa có vector.
- `Cần thu lại mẫu` — embedding không tương thích model mặc định.

Không hiển thị `pending`, `provisional`, `passed`, `calibration`, holdout hoặc Speaker Provider selector/revision. Không cần một màn hình mới.

### Add Speaker (một wizard)

```text
Open Add Speaker
   -> Microphone: record one WAV
   -> POST /speakers/captures
   -> Server validation + extract; show "Trích xuất thành công"
   -> Name / Description
   -> POST /speakers/from-capture
   -> Speaker Detail (usable immediately for matching if linked to Agent)
```

- **Reuse recorder và form** hiện tại. Chọn `SpeakerEnrollmentWizard.vue` làm một luồng mặc định; chuyển phần capture/Save hữu ích từ `QuickSpeakerEnrollment.vue` rồi xóa Quick component khi không còn caller. Không tạo UI flow thứ ba.
- Không gọi `providersApi.list({type:'speaker'})`. Không persist profile trước khi Save thành công. Khi upload thất bại thì retry recording; khi Save thất bại giữ form/capture ID để retry trong TTL; disable double-submit.
- Re-enroll ở Speaker Detail dùng cùng recorder/capture, sau đó `PUT /speakers/{key}/voiceprint` với ETag. Rename không chạy inference.
- Status của Speaker được derive từ response `voiceprints` + `embedding_space_id` từ summary, hoặc mapper trả status hiển thị; **không tạo storage state mới**.

### Agent Detail / Providers / Templates

- `AgentSpeakerPolicy.vue`: toggle OFF/ON ánh xạ `off/observe`; checklist Speaker trực tiếp Agent; không nhập/chọn Template để cấp quyền Speaker.
- Agent ON nhưng Speaker engine unavailable: thông báo tác vụ nhận dạng không khả dụng; không tự đổi Agent policy.
- Providers page bỏ Speaker card/tab/adapter/test. Template create/edit/AI Pipeline bỏ slot Speaker. Mọi VAD/ASR/LLM/TTS pipeline vẫn nguyên.

## 7. WebSocket — Speaker chỉ là nhãn turn, không phải security

### 7.1 Admission và snapshot

1. Device authentication/WS admission/Template resolution vẫn theo code hiện có.
2. Đọc Agent speaker policy; `off` hoặc không có candidates → không chạy Speaker; **không chặn WS**.
3. `observe` → lấy `Arc<SpeakerRuntime>`/lease mặc định từ `AppState`, query **một lần** danh sách Agent candidates: `agent_speaker_candidates JOIN speakers JOIN speaker_voiceprints`, điều kiện `speakers.enabled=1`, `voiceprint.embedding_space = runtime.embedding_space_id()`, vector valid, bounded `max_candidates_per_agent`. Không join `agent_speaker_template_grants` hoặc lấy `template_id`.
4. `switch_template` không thay đổi candidates hoặc speaker runtime. Không yêu cầu Speaker provider lease từ Template; không tạo `SpeakerSwitchGuard` hay qualification authority mới.
5. Mutations speaker/candidate/Agent policy áp dụng cho **WS connections mới hoặc sau reconnect**. Session cũ tiếp tục dùng immutable candidate snapshot cho mục đích cá nhân hóa; do đó **không dùng Speaker match cho quyền truy cập**. Không thêm revocation registry, WS close 1008 hoặc query DB ở mỗi frame/turn vì riêng Speaker. Các kiểm tra revocation hiện có cho **Device/Admin/Tool security** vẫn giữ nguyên.
6. Log `speaker_enabled`, `engine_available`, `candidate_count` ở admission. Không log PCM, embedding, auth secrets.

### 7.2 Ghép ASR Final + Speaker result trước LLM

**Hiện trạng cần sửa:** `session/actor/observe.rs` chạy fire-and-forget; `actor/listening.rs::on_asr_event` gọi `commit_user_text` và `begin_speech_delivery` ngay khi ASR Final, nên Speaker result **không được đưa vào LLM**.

```text
Terminal utterance
  |--- ASR Final(text) ---------------|
  |--- SpeakerObserve.observe() ------|  join by ObserveIdentity
                                     |
                               SessionActor
                         matched / unknown / timeout
                                     |
                   commit_user_text(text) [unchanged]
                                     |
                   begin_speech_delivery() with
                   optional, turn-local speaker note
                                     |
                                LLM -> TTS
```

- Dùng **chính SessionActor**, `ObserveIdentity {operation_id,turn_id,generation}` và actor event channel đang có; không tạo `IdentificationOutcome`, `Coordinator`, `ResultBus` hay second cache.
- Actor giữ tối thiểu `pending_asr_final` và Speaker diagnostic của **current turn**. Nếu Speaker đến trước → lưu tạm trong turn; nếu ASR Final đến trước → đợi **tối đa 750ms** (hằng nội bộ thử nghiệm, không mở config mới), hết hạn thì gửi LLM không nhãn. Unknown/busy/no audio/unavailable dùng flow bình thường.
- Kiểm `operation_id`, `TurnId`, `generation` trước khi dùng diagnostic. Abort, cancel, disconnect, text detect, history-barrier hoặc late result phải loại stale identity; không chuyển A sang B.
- `SpeakerStatus::Verified` và `ObserveDiagnostic` **được tái sử dụng**. Có thể giữ string `verified` trên WS trong giai đoạn tương thích firmware; **nội bộ hiểu là matched, không phải authenticated**. Score chỉ là cosine similarity, threshold 0.50 là heuristic.
- Lấy tên từ hồ sơ Speaker của **candidate snapshot** (thêm `name` vào `ObserveCandidate` hiện có hoặc ánh xạ key→name ở snapshot; chọn một cấu trúc hiện có, không thêm DTO). Không tin tên từ client. Không log thông tin người nói vào LLM history lâu dài.
- Chỉ thêm **ephemeral, bounded text** ở bước build LLM messages: `Người nói trong lượt hiện tại có thể là Lan. Chỉ dùng để xưng hô; đây không phải xác thực.` Tên Speaker là dữ liệu, phải bound/escape, không được coi là chỉ dẫn. Không sửa STT payload hoặc nội dung User History.
- Sau timeout, native worker/lease vẫn hoàn thành và được giải phóng bằng worker logic hiện có. Không chặn toàn bộ audio/ASR/TTS khi Speaker chậm.

### 7.3 Logging

```text
ws_speaker_config: agent_key, speaker_enabled, engine_available, candidate_count
speaker_identification: turn_id, generation, outcome=matched|unknown|unavailable|timeout|insufficient_audio, speaker_key_if_matched, inference_ms
```

Không dùng tên `speaker_authenticated`/`speaker_authorized`. WS Bearer/Device authentication logs là luồng độc lập. Chỉ log kết quả đã đi qua stale-turn check, không công bố danh tính hay score ra client khi chưa có contract yêu cầu.

## 8. Dọn Required/Calibration và docs

- Xóa `SpeakerPolicyMode::Required` và các nhánh Required trong `session/speaker_observe.rs`, `speaker_gate.rs`, actor, WebSocket admission, template switch và API. Gỡ `SpeakerSwitchGuard` chỉ dùng cho Speaker; không xóa Template switch bình thường.
- Bỏ `speaker_calibration.rs`, qualification endpoints, `calibration_source`, `speaker_evaluation/*` nếu không còn consumer; xóa usages/tests liên quan và giữ history ADR.
- `database/tool_security.rs` có cơ chế tool security độc lập; **không xóa toàn bộ module**. Tháo phần Speaker-specific security dependency/close-on-mutation trong khi giữ các dependency thực sự cần cho tool authorization.
- Không cần `speaker_pilot` nếu chỉ phục vụ Speaker Required/qualification; kiểm tra consumer trước khi bỏ. Không thay đổi deployment profile hoặc quotas cho pipeline khác.
- ADR `0082` về validated-first và ADR `0012`/`0077` về provisional/Required được **superseded cho Speaker enrollment/auth**, nhưng không xóa lịch sử; thêm ADR mới hoặc update links tới tài liệu này.

## 9. Trình tự thực hiện (P0 → P4)

### P0 — Data + built-in runtime

- [ ] Snapshot DB + kiểm tra compatibility của embedding space.
- [ ] Forward migration cho `speaker_voiceprints` và **`speaker_quick_captures` hiện có**; giữ Agent candidates; migrate `required → off`; bỏ Template grants dependency.
- [ ] Bootstrap một `Arc<SpeakerRuntime>` + lease bằng lifecycle hiện hữu; không thêm engine wrapper hoặc global enabled flag.
- [ ] Bỏ Speaker khỏi provider CRUD/catalog/Template/load plan; regression VAD/ASR/LLM/TTS loading.

### P1 — Speaker HTTP enrollment

- [ ] Chuyển `speaker_quick.rs` capture route từ `/providers/{key}/speaker-captures` sang `/speakers/captures`; bỏ provider key/revision; giữ TTL 10 phút, idempotency và transaction.
- [ ] `speakers.rs`: giữ list/get/update/purge; bỏ draft/multi-sample/holdout dead code; implement PUT re-enroll bằng capture hiện có.
- [ ] `speaker_policy.rs`: `off/observe`, Agent direct candidates, `If-Match`; không có Template grant fields.
- [ ] API and contract tests cho create, retry, expiry, purge, re-enroll, provider-type=speaker rejection.

### P2 — Admin Web

- [ ] Giữ SpeakersView và Speaker Detail; cập nhật derived badge.
- [ ] Gộp Wizard + Quick thành **một flow thu âm**; không Provider dropdown/multi-sample/holdout.
- [ ] AgentSpeakerPolicy: toggle + chọn Agent candidates, bỏ Required và Template picker.
- [ ] Providers/Template UI bỏ Speaker slot; update TypeScript clients, i18n, component tests.

### P3 — Voice WS / personalization

- [ ] `resolve_speaker_observe`: Agent toggle + AppState runtime + Agent candidates; không Template speaker binding.
- [ ] Tận dụng `ObserveDiagnostic` + `ObserveIdentity`, actor join bounded với ASR Final **trước LLM prompt**.
- [ ] Test race, timeout, abort, new turn, template switch, text detect, worker cleanup và WS protocols.
- [ ] Log config/outcome; không Speaker-derived authorization.

### P4 — Cleanup + docs + verification

- [ ] Xóa dead Required/calibration/draft/template grant code sau khi không còn caller.
- [ ] Update `docs/api/00-all-apis.postman_collection.json`, `docs/speaker-provider.md`, `docs/flows/01-websocket.md`, `config.example.toml` và ADR supersession notes.
- [ ] Chạy backend tests/fmt/clippy, web typecheck/test/build, migration trên SQLite backup, WS smoke với thiết bị thật; ghi kết quả, không tự tuyên bố pass.

## 10. Acceptance / regression checklist

| Trường hợp | Phải đạt |
|---|---|
| Server boot | Một CAM++ runtime dùng chung ở startup; không có DB Speaker Provider row/Template binding/Admin Prepare. Failure riêng Speaker không chặn Voice pipeline. |
| Capture WAV hợp lệ | Accepted + `capture_id`; chưa tạo Speaker cho đến Save. |
| WAV quá ngắn/silent/clipped/malformed/oversize | 4xx rõ, không stage vector rỗng. |
| Save và retry | Một Speaker + voiceprint; retry cùng capture không tạo duplicate; không có pending/provisional/holdout. |
| Re-enroll cùng Speaker | Một vector mới cho active space, giữ key/name/Agent links, ETag đúng. |
| Migration SQLite | Giữ metadata/valid vectors/candidates; `required → off`; FK check sạch; incompatible vectors không bị match. |
| Agent ON/OFF | OFF không inference; ON chỉ so với Speaker của Agent; no candidates/engine unavailable vẫn hội thoại. |
| Match đúng/unknown | STT như cũ, LLM nhận label đúng turn hoặc không label, TTS bình thường; không cấp tool permission. |
| ASR-Speaker completion order | Both orders đúng; quá 750ms tiếp tục LLM; không stale identity. |
| Abort/cancel/reconnect/Template switch | Không leak identity sang turn khác; Template switch không thay candidate scope Agent. |
| Speaker/Agent mutation trong WS | WS cũ giữ immutable snapshot đến reconnect; WS mới lấy config mới. Không dùng snapshot để xác thực. |
| Generic Provider API | `type=speaker` bị reject; VAD/ASR/LLM/TTS không regression. |
| Admin Web | Giữ pages, recorder; một Wizard; không Provider selector, holdout/qualification. |
| Docs/Postman | Mô tả Speaker built-in, route capture mới và Agent identification only. |

Lệnh gợi ý (xác minh workspace/package scripts trước khi chạy):

```bash
cargo fmt --all --check
cargo test -p voice-agent-server
cargo clippy -p voice-agent-server --all-targets -- -D warnings
# Run actual scripts from apps/admin-web/package.json:
npm run typecheck
npm run test
npm run build
```

## 11. Definition of Done

- [ ] Không còn Speaker Provider Instance/CRUD, Template Speaker slot/grants hoặc calibration/Required.
- [ ] **Một** CAM++ `SpeakerRuntime` dùng chung, reuse worker/lease lifecycle hiện có; không `BuiltinSpeakerEngine` wrapper.
- [ ] **Giữ Speakers page và microphone.** Một mẫu hợp lệ → extract success → tên/mô tả → SQLite → sử dụng ngay khi được liên kết Agent.
- [ ] Giữ `speaker_quick_captures` và TTL 10 phút, idempotency; không bảng `speaker_captures` mới, không cấu hình `capture_ttl_ms`.
- [ ] Agent chỉ OFF/ON + Speaker candidates; không thêm authority/recognition registry.
- [ ] LLM nhận danh tính **chỉ ở current turn**; không phụ thuộc Speaker để chạy ASR/LLM/TTS hoặc cấp quyền.
- [ ] Migration bảo toàn dữ liệu, xử lý `required → off`, tương thích embedding space, và quy tắc mutation áp dụng sau reconnect.
- [ ] Backend + Admin Web tests, SQLite migration check, WS smoke, Postman và docs đã được cập nhật, ghi kết quả thực tế.

---

**Yêu cầu cho coding agent:** triển khai tuần tự theo P0–P4, thay thế code cũ tại seam đang có thay vì chồng module mới. Không mở rộng API/config/schema ngoài các thay đổi nêu rõ; phần Speaker chỉ là một nhãn nhận dạng best-effort, **không phải quyền hay bảo mật sinh trắc học**.
