# Review Admin Web — 2026-10-09

What this repo does: `apps/admin-web` là UI quản trị cho Voice Agent Server, cấu hình resource qua Admin API và xem runtime diagnostics. Browser không sở hữu Voice Session state. Workload giả định là một hoặc vài operator trong homelab, catalog nhỏ nhưng có thể vượt một page, và API vẫn phải bảo vệ revision khi hai tab/operator cùng sửa.

Phạm vi: entry points, routes, config/dependencies, API clients, admin/auth/server stores, các trang chính, form/pipeline/diagnostics, Device claim, Speaker capture và tests liên quan. Đối chiếu ADR hiện tại và Rust Device handlers/lookup để xác nhận public identity. Đây là review implementation, không chỉ diff. Không sửa application code trong lần này.

## Must fix

### 1. Template edit form bỏ qua provider binding writes — P1

Nguồn: [admin.ts:564](../src/stores/admin.ts#L564), [applyBindingsInternal:599](../src/stores/admin.ts#L599). Callers: Template catalog/detail form và Agent Studio Template form.

- **What this is:** Form lưu metadata và danh sách Provider cần bind cho Template.
- **Problem:** `updateTemplateInternal()` gọi `upsertTemplate(template, providerBindings)` trước `applyBindingsInternal()`. Binding diff sau đó đọc chính desired bindings vừa ghi vào cache, thấy `next === previous` nên bỏ qua mọi PUT/DELETE. Ví dụ Template chưa có LLM, form chọn Provider `p`: cache hiện `p` nhưng server vẫn chưa bind; reload làm mất lựa chọn.
- **Fix:** Upsert metadata với bindings cũ; diff/gửi writes trước khi publish server-confirmed bindings. Dùng revision mới từ PATCH và reload bindings sau mỗi mutation như luồng direct link hiện có.
- **If we skip it:** Operator tưởng pipeline đã được lưu, nhưng phiên sau dùng cấu hình cũ hoặc thiếu Provider. Apply/remove/replace qua form đều có thể sai.

**Evidence:** Audit self-check mock Template PATCH thành công, chọn `llm: p`; bindProvider không được gọi dù cache trả `p`. Direct link test hiện tại không bao phủ đường form này.

### 2. Device từ initial load thiếu revision, sửa/xóa âm thầm không làm gì — P1

Nguồn: [loadAll:316](../src/stores/admin.ts#L316), [updateDevice:858](../src/stores/admin.ts#L858), [deleteDevice:894](../src/stores/admin.ts#L894).

- **What this is:** Tab Devices ở Agent gọi store để sửa/xóa Device đã tải khi Connect/reload.
- **Problem:** `loadAll()` reset revision maps rồi map Device list nhưng không ghi Device revisions. `updateDevice()`/`deleteDevice()` gặp revision undefined và return trước API request. Sau mở app rồi sửa/xóa Device có sẵn, form/dialog có thể đóng mà dữ liệu không đổi.
- **Fix:** Ghi revision theo DB ID khi ingest Device page; kiểm tra kết quả mutation ở caller trước khi đóng form/dialog. Giữ identity nội bộ tách khỏi public identity như finding 3.
- **If we skip it:** Quản trị Device trong Agent Detail không hoạt động sau mỗi full reload, không có lỗi giúp operator nhận biết.

**Evidence:** Audit self-check load một Device revision 1; `store.revisions.devices` là `{}`, delete không gọi API. Device Detail raw API không đi qua đường này.

### 3. Store gửi DB primary key thay public Device Identity — P1

Nguồn: [updateDevice:858](../src/stores/admin.ts#L858), [setDeviceTemplateOverride:884](../src/stores/admin.ts#L884), [deleteDevice:894](../src/stores/admin.ts#L894), [devices API](../src/api/devices.ts), [backend lookup](../../../crates/voice-agent-server/src/database/devices.rs#L23).

- **What this is:** View model giữ `id = String(DB id)` và `deviceId = public device_id`; store dùng ID nội bộ để tìm cache/revision.
- **Problem:** Sau claim, store đã có revision nên gửi PATCH/DELETE tới `/api/admin/devices/7` cho Device `{ id: 7, device_id: 'esp32-demo' }`. Backend lookup dùng `WHERE d.device_id=?`, vì vậy trả 404 thay vì sửa/xóa thiết bị đó.
- **Fix:** Giữ DB ID để lookup cache, nhưng truyền `device.deviceId` vào public API path ở cả update, override và delete. Thêm test với DB ID và Device Identity khác nhau.
- **If we skip it:** Sửa/xóa Device vừa claim vẫn lỗi dù finding 2 đã được xử lý; nếu public ID của một Device khác trùng chuỗi DB ID, request còn có thể nhắm nhầm resource.

**Evidence:** Audit self-check claim Device ID 7/public identity `esp32-demo`; delete gọi remove với `'7'`. Rust handler/SQL xác nhận path không nhận alias DB ID.

### 4. Update cache bằng splice không thông báo Vue render — P1

Nguồn: [upsertAgent:258](../src/stores/admin.ts#L258), [upsertTemplate:274](../src/stores/admin.ts#L274), [upsertDevice:283](../src/stores/admin.ts#L283), [upsertProvider:291](../src/stores/admin.ts#L291).

- **What this is:** Các catalog dùng `shallowRef` arrays để giữ read model, mutation thay phần tử trong array.
- **Problem:** Nhánh update dùng `splice()` trên array thường bên trong shallowRef. Sau Agent rename thành công, đọc trực tiếp store thấy tên mới nhưng watcher/render của tên vẫn giữ tên cũ tới khi dependency khác kích hoạt. Cùng pattern tồn tại cho Template/Device/Provider.
- **Fix:** Thay array bằng `map()` giống `syncAgent()`/`applyBindings()` hiện có. Không cần thêm store abstraction hoặc event bus.
- **If we skip it:** UI stale sau lưu thành công; operator có thể retry hoặc thao tác dựa trên giá trị cũ.

**Evidence:** Audit self-check watch Agent name: sau update + nextTick, direct getter là `After`, watcher chỉ nhận `Before`. Suite hiện có chỉ kiểm reactive default-template/provider-link paths đã dùng array replacement.

### 5. Duplicate Provider làm mất cấu hình adapter — P1

Nguồn: [createProvider:719](../src/stores/admin.ts#L719), [duplicateProvider:763](../src/stores/admin.ts#L763), caller [ProvidersView](../src/views/ProvidersView.vue).

- **What this is:** Overflow menu catalog cho phép duplicate một Provider Instance.
- **Problem:** Duplicate spread view model vào `createProvider()`, nhưng create dựng `config_json` lại từ model/description/endpoint. Config `{ model, base_url, timeout_ms }` thành `{ model, endpoint }`; adapter-specific settings bị mất và `base_url` bị đổi tên. Stored credential cũng không được clone qua masked metadata.
- **Fix:** Copy nguyên typed `source.configJson` vào existing create API. Nêu rõ credential cần nhập lại nếu không có backend clone contract; không dùng masked hint như secret.
- **If we skip it:** Bản sao có thể bị validation reject, dùng endpoint/default khác hoặc không gọi được provider, dù UI gọi thao tác là copy configuration.

**Evidence:** Audit self-check duplicate OpenAI Provider với base_url/timeout_ms; request body mất timeout_ms và không còn base_url.

### 6. Chọn Follow Agent không clear Device Template override — P2

Nguồn: [DeviceFormModal](../src/components/admin/DeviceFormModal.vue), [Agent saveDevice](../src/pages/agents/AgentDetailPage.vue#L202), [updateDevice:858](../src/stores/admin.ts#L858), [setDeviceTemplateOverride:884](../src/stores/admin.ts#L884).

- **What this is:** Form Device trong Agent có lựa chọn default/override; API cần `null` để xóa override.
- **Problem:** Form chọn default emit `templateId: undefined`; store giữ undefined trong payload, JSON.stringify bỏ field. Helper `setDeviceTemplateOverride()` cũng tạo undefined khi muốn clear. Sau khi identity/revision được sửa, request này vẫn giữ override trên backend.
- **Fix:** Phân biệt không chỉnh field với explicit clear; truyền `template_key: null` cho Follow Agent, giống Device Detail hiện có. Không thay tất cả omitted fields thành null.
- **If we skip it:** Thiết bị tiếp tục dùng Template override khi operator tưởng đã chuyển theo default của Agent.

**Evidence:** Audit self-check helper clear trên Device có revision; payload JSON là `{}` thay vì `{ "template_key": null }`. Helper hiện chưa có UI caller, nhưng DeviceFormModal/updateDevice đang có cùng lỗi omission.

### 7. Token sai làm mất đường nhập credential mới — P2

Nguồn: [App.vue:22](../src/App.vue#L22), [auth store](../src/stores/auth.ts).

- **What this is:** Connect lưu token, gọi loadAll và ẩn form khi token không rỗng.
- **Problem:** Nhập token sai vẫn làm `auth.adminToken` truthy. Load trả 401, form đã ẩn và chỉ còn Retry với cùng token; không có logout/đổi credential. Operator phải sửa sessionStorage hoặc env ngoài UI để phục hồi.
- **Fix:** Cho phép đổi/xóa token khi gặp authentication failure, dùng `setAdminToken()` hiện có và làm rõ fallback env. Không cần xây hệ thống account/login mới.
- **If we skip it:** Một lỗi nhập token khiến app không kết nối được từ chính UI.

**Evidence:** Static trace: connect set token trước await, visibility chỉ dựa vào token; banner Retry không đổi auth state. Workaround nằm trong [development](development.md#token-sai-hoặc-401).

## Should fix

### 8. Cache và selector-based detail pages cắt ở trang đầu — P2

Nguồn: [loadAll:316](../src/stores/admin.ts#L316), [agents.ts:19](../src/api/agents.ts#L19), [agents.ts:42](../src/api/agents.ts#L42).

- **What this is:** Cache cấp dữ liệu cho catalogs, Overview counts, Template/provider usage và Agent/Template details.
- **Problem:** Chỉ lấy 50 Agent enabled, 50 links mỗi Agent, 100 Template/Provider/Device; không loop pagination. Agent thứ 51 hoặc Template thứ 101 có thể hiển thị not-found qua deep link dù backend có resource. Disabled Agent bị ẩn ngay cả khi catalog nhỏ. Usage/delete prechecks và effective Template lookup cũng thiếu references ngoài page đã tải.
- **Fix:** Tải đủ các bounded pages hoặc phân trang UI và GET detail trực tiếp. Tái dùng pattern bounded pagination của Devices/MCP; báo rõ khi đạt cap. Xác định việc cố ý ẩn disabled Agent trước khi giữ filter hard-coded.
- **If we skip it:** Counts/filter kết quả không đầy đủ, resources hợp lệ không truy cập được ở một số trang và operator đánh giá sai blast radius.

**Evidence:** Static trace của fixed page queries và `getAgent()`/`getTemplate()` chỉ tìm cache. Backend delete guards vẫn là nguồn chân lý, không bị frontend precheck thay thế.

### 9. Catalog Provider delete có thể để lại partial unlink — P2

Nguồn: [deleteProvider:776](../src/stores/admin.ts#L776), đối chiếu [ProviderDetailPage](../src/pages/providers/ProviderDetailPage.vue#L202).

- **What this is:** Catalog unlink từng Template trước khi delete Provider; detail page chặn delete khi còn usage.
- **Problem:** Với Provider dùng bởi hai Template, unlink thứ nhất thành công nhưng request thứ hai/final DELETE thất bại vì network/conflict/reference mới. Provider còn tồn tại nhưng cấu hình Template đã bị đổi, thao tác không rollback. Cache chưa đầy đủ ở finding 8 làm tình huống final `provider_in_use` dễ gặp hơn.
- **Fix:** Dùng cùng hành vi chặn delete khi usage còn như detail page và yêu cầu unlink tường minh trước. Đây là thay đổi nhỏ, giữ conditional hard-delete theo ADR 0070, không cần thêm transaction protocol ở frontend.
- **If we skip it:** Một delete thất bại vẫn có thể gỡ Provider khỏi pipeline; operator cần tự khôi phục binding đã mất.

**Evidence:** Static trace: awaited unlink loop rồi GET/DELETE, catch chỉ hiển thị lỗi; không có rollback. Backend cố ý không cascade unlink.

### 10. BaseModal không giữ keyboard focus trong dialog — P2

Nguồn: [BaseModal.vue:30](../src/components/admin/BaseModal.vue#L30).

- **What this is:** Modal dùng cho form/confirmation, khai báo `aria-modal=true`, focus panel khi mở và restore khi đóng.
- **Problem:** Key handler chỉ Escape; không trap Tab hoặc inert background. Mở dialog và Tab qua control cuối có thể chuyển tới link/nút của trang bên dưới, dù UI đang hiển thị modal.
- **Fix:** Giữ focus trong modal và vô hiệu focus background; dùng primitive/pattern dialog có sẵn nếu repo bổ sung, hoặc native dialog khi phù hợp conventions. Giữ Escape/restore focus hiện có.
- **If we skip it:** Người dùng keyboard có thể tương tác nhầm trang phía sau và không điều hướng dialog theo semantics đã khai báo.

**Evidence:** Static inspection của Teleport/panel/key handler; không có focus guard/inert. Chưa chạy browser keyboard hoặc screen-reader verification.

## Kiểm tra đã chạy

| Kiểm tra | Kết quả |
| --- | --- |
| `npm test` | 38 files, 126 tests pass |
| `npm run build` | vue-tsc/typecheck pass; Vite production build pass |
| Sáu audit self-check ở temporary Vitest file | 6 pass, xác nhận observed faulty behavior của findings 1–6 |

Self-check assert hành vi lỗi đang có, không phải regression suite khẳng định implementation đúng. File tạm được xóa sau chạy để phạm vi thay đổi cuối chỉ là tài liệu. Khi sửa từng lỗi, thêm test permanent kỳ vọng hành vi đúng vào suite tương ứng.

Runtime môi trường review: Node `20.19.2`, npm `9.2.0`; Node thấp hơn yêu cầu `>=22.12.0` trong package. Dependency đã cài sẵn, không install/upgrade packages. Build pass trên môi trường này không hạ yêu cầu engines của ứng dụng.

Điểm đã có kiểm tra hữu ích: bearer/revision/error request contracts, direct Template bindings reactive updates, Device/Speaker details, credential forms, diagnostic cancellation/results, enrollment và WAV utilities. Những test này không bao phủ hết các đường mutation chung nêu trên.

## Các giới hạn khác đã ghi trong tài liệu

- Translation keys có type nhưng nhiều form/onboarding/recognition và API error copy hard-code tiếng Việt; chưa hỗ trợ EN/VI đầy đủ ở mọi flow.
- Speaker recognition chỉ `off`/`observe`; chưa có `required`, authentication bằng giọng hay calibration/qualification UI.
- History client có sẵn, chưa có History page. Vision có UI topology/types nhưng không có Template API slot.
- HTTP wrapper không có timeout mặc định; cancellation chưa đồng đều giữa Agent subpanels.
- Sau mutation conflict, store reload có thể clear thông báo lỗi; không giả định mọi form đã reload revision mới.
- WAV resampling dùng linear interpolation, có comment về giới hạn chất lượng; không đánh giá calibration thực tế bằng jsdom tests.

**Verdict:** Sửa provider-binding form và hai lỗi Device revision/identity trước; tiếp theo là reactive upserts và Provider duplicate. Documentation đã cập nhật theo implementation, các lỗi trên vẫn tồn tại.

**Not checked:** Backend live integration, browser E2E/visual/keyboard/screen reader, CORS/TLS deployment, microphone/audio thật, Provider/MCP remote services, firmware/hardware enrollment, security penetration testing và tải lớn. Review tập trung boundary có hậu quả cao; không audit mọi CSS/icon/translation string hoặc toàn bộ Rust backend.
