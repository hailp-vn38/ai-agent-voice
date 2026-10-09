# HTTP, API và credentials

## Transport chung

`src/api/client.ts` ghép path với `VITE_API_BASE_URL` đã bỏ trailing slash. Base URL rỗng là cùng origin. `withQuery()` dùng URLSearchParams, bỏ `null`/`undefined`; public key/path parameters được resource clients encode bằng `encodeURIComponent()`.

`request()` dùng fetch, merge headers, thêm bearer token nếu có và thêm `If-Match: "<revision>"` nếu truyền revision. Header Authorization hiện được gắn cho **mọi request qua wrapper**, cả `/health` và `/ready`, không chỉ `/api/admin/*`. Không có cookie-auth flow trong client.

Các helper:

| Helper | Kết quả |
| --- | --- |
| `request` | Response hoặc ApiError nếu HTTP không thành công |
| `requestJson<T>` | Parse JSON; `204` trả `undefined` |
| `requestText` | Text, dùng cho health/readiness |
| `requestBlob` | Binary Blob |
| `requestAudio` | Yêu cầu Content-Type `audio/wav`, Blob và elapsed header |
| `jsonRequest` | JSON body, kiểm tra transport khi có top-level Provider `api_key` |

`RequestOptions` hỗ trợ revision, signal, headers và body. Wrapper chỉ nhận signal qua options; không có timeout/retry mặc định. Generic TypeScript types không runtime-validate toàn bộ JSON response.

Error envelope có dạng `{ "error": { "code": "...", "request_id": "..." } }`. `ApiError` giữ HTTP status, code và request ID; `formatApiError()` hiển thị thông báo, fallback HTTP hoặc code, kèm request ID nếu có. Proxy trả body không phải JSON vẫn được đổi thành `http_<status>`.

## Revision và conditional writes

Resource revision và Expected Revision thuộc backend. PATCH/PUT/DELETE revisioned gửi `If-Match`; stale revision trả `409 revision_conflict`, không last-write-wins. Không áp đặt rule này lên POST create, probes hoặc captures không có revision.

| Thao tác | Revision owner |
| --- | --- |
| Agent metadata, Template assignment/default, MCP binding | Agent |
| Template metadata và provider binding | Template |
| Provider metadata/config/enable/delete | Provider |
| Device metadata/Agent/override/enable/delete | Device |
| Speaker metadata/voiceprint/purge/delete | Speaker |
| Speaker policy | Policy response revision |
| Agent Speaker candidate binding | `agent_revision` trong binding page |
| External tool review | Tool entry `revision`, cùng observation revision + fingerprint |

Sau relationship write, reload owner/relationship để dùng revision mới cho thao tác tiếp theo. Form setup rethrow không tự reload conflict như `run()`; operator cần đọc lỗi và refresh. Hiện tại việc báo conflict của store có thể bị mất sau reload; xem [review](review.md).

## Resource client map

Các path dưới đây là path mà frontend gọi, không phải bản sao toàn bộ backend API specification.

| Module | API surface |
| --- | --- |
| `agents.ts` | `/api/admin/agents`; resource GET/POST/PATCH/DELETE; `/templates/{templateKey}` PUT/DELETE; `/default-template/{templateKey}` PUT; `/mcp-bindings`; `/speaker-policy`; `/speakers/{speakerKey}` |
| `templates.ts` | `/api/admin/templates`; CRUD; `/{key}/agents`; `/{key}/providers`; `/{key}/providers/{type}` PUT/DELETE |
| `providers.ts` | `/api/admin/providers`; CRUD; `/{key}/templates`, `/prepare`, `/capabilities`, `/test/{vad,asr,llm,tts}`; `/api/admin/provider-tests/{asr,llm,tts}` |
| `provider-adapters.ts` | `/api/admin/provider-adapters`; GET descriptor; `/{adapter}/capabilities/discover` POST |
| `devices.ts` | `/api/admin/devices`; CRUD theo public `device_id`; `/api/admin/device-enrollments/claim` POST |
| `mcp.ts` | `/api/admin/mcp-servers`; CRUD; `/{key}/test/{connection,discover}`; `/api/admin/mcp-tests/{connection,discover}` |
| `external-tools.ts` | `/api/admin/agents/{agentKey}/tool-allowlist` GET/PUT |
| `speakers.ts` | `/api/admin/speakers`; CRUD; `/captures`, `/from-capture`; `/{key}/voiceprint` PUT; `/voiceprint/purge` POST; `/bindings`; summary `/api/admin/speaker-recognition` |
| `history.ts` | `/api/admin/history` GET và `/purge` POST; chưa có route/page History |
| `system.ts` | `/health`, `/ready`, `/api/admin/system` |

Đối chiếu DTO cụ thể trong `src/api/types/`. Raw response có thể dùng boolean hoặc số `0/1` tùy resource; không suy từ `enabled` của resource khác.

## Pagination

`Page<T>` khai báo `total` nhưng một số backend responses, như Devices và MCP, không trả field này. Các consumer của chúng dùng độ dài page để xác định terminal page. Không lấy `total` làm bắt buộc nếu endpoint không cung cấp.

| Consumer | Cách tải/lọc |
| --- | --- |
| Admin cache | Trang đầu: Agent 50, links 50, Template/Provider/Device 100 |
| Devices catalog | 100 item/page, tối đa 100 page; short page kết thúc, chạm cap báo lỗi |
| MCP catalog | 50 item/page, Load more; search/filter chỉ trên items đã tải |
| Agent MCP picker | 200 item/page, tối đa 20 page; chạm cap báo lỗi |
| Speakers catalog | 50 item/page với previous/next và total; search chỉ trong trang hiện tại |
| Agent Speaker picker | Trang đầu 100 Speaker; chưa duyệt hết catalog |
| Provider detail usage | 50 Template/page và Load more theo total |
| Speaker bindings client | Lấy trang đầu 50 binding; hiện chưa có caller UI |

## Credentials

Admin token nhập lúc chạy ở sessionStorage; `VITE_ADMIN_TOKEN` là fallback build-time. Xóa session token không vô hiệu hóa fallback đã nhúng vào bundle. Hiện không có UI đổi token/logout.

Provider/MCP `api_key` là write-only, tách khỏi typed `config_json`. Read chỉ trả metadata đã mask; để trống field update sẽ giữ key hiện có. MCP đổi auth thành `none` xóa stored credential theo backend contract. Credentials được backend mã hóa trong SQLite; browser không giữ chúng trong storage. Encryption key là cấu hình server, không phải `VITE_*`.

Provider credential submission kiểm tra cả origin trang và API URL: cần HTTPS, ngoại lệ development trên `localhost`, `127.0.0.1`, `[::1]`. Draft tests có credential nested trong provider được check riêng trước khi gọi wrapper.

MCP create/edit cố ý dùng `allowHttpCredentials: true`; draft probes cũng cho phép HTTP, kể cả non-loopback, theo ADR 0083 hiện tại. Admin token request nói chung không được guard HTTPS bởi wrapper. Chọn transport/deployment phù hợp với contract thay vì giả định mọi bearer credential đều bị chặn trên HTTP.

Draft diagnostics có thể tham chiếu `saved_credential: { key, expected_revision }` nếu giữ adapter/auth tương thích; frontend không tải plaintext key về để test. Thay Provider credential không cập nhật runtime đã active; thay MCP credential có thể invalidation tool approvals. Theo dõi revision/runtime và review lại tool contract khi server yêu cầu.

## Diagnostic payloads

- Saved ASR: POST WAV binary với `Content-Type: audio/wav`.
- Draft ASR: multipart FormData với `provider` JSON và `audio` file; browser tự tạo boundary.
- Saved LLM: JSON `{ input }`; draft LLM: `{ provider, input: { text } }`.
- TTS: JSON text, optional voice/language; trả WAV và `x-provider-test-elapsed-ms`.
- Saved VAD: POST, trả JSON; panel chưa test draft VAD.
- MCP connection/discovery: saved resource hoặc draft `{ server }`; catalog incomplete bị hiển thị lỗi.
- Speaker capture: POST WAV → `capture_id`, quality và expiry; commit bằng create-from-capture hoặc replace voiceprint với revision.

Test thành công chỉ chứng minh request diagnostic đó; không tự bind resource, approve tool, prepare desired runtime hay mở Voice Session.

## Xóa resource

Backend conditional hard-delete từ chối resource còn reference và không tự purge history. Template còn provider bindings cũng bị chặn, kể cả khi chưa link Agent. Agent còn Device/MCP/history không xóa được. MCP còn Agent binding không xóa được. Speaker phải unlink khỏi Agent trước.

Provider có hai hành vi UI: detail page từ chối xóa khi usage còn; catalog gọi store để unlink lần lượt các Template rồi xóa Provider. Chuỗi này không atomic: lỗi cuối có thể để Provider tồn tại nhưng đã mất một số bindings. Đây là hạn chế có tác động dữ liệu, ghi ở review.
