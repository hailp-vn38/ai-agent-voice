# Voice Agent Studio — UI redesign & kết nối Rust server

Trạng thái: **P0/P1 foundation đã tích hợp ở branch `feat/voice-agent-studio-ui`**; Playground, Reports và realtime telemetry là **thiết kế chưa triển khai**.
Áp dụng cho: `apps/admin-web/` của `ai-agent-voice`, Vue 3 + Pinia + Tailwind 4.

## 1. Mục tiêu và ranh giới

Biến Web Admin dạng CRUD thành Voice Agent Studio có bản sắc AI/Voice mà **không thay đổi quyền sở hữu domain**:

- `Agent`: identity, linked Templates, default Template, Devices và chính sách External MCP/Speaker.
- `Template`: language, prompt, provider bindings; có thể dùng chung nhiều Agent.
- `Provider`: catalog và runtime do server quản lý. Một Template tham chiếu provider bằng key; khi thiếu binding server giải quyết `provider_defaults` tại admission.
- `Device`: đăng ký/được bật, thuộc Agent, có thể override sang Template đã liên kết.
- `Speaker`: voiceprint và điều kiện đủ tư cách riêng; capture success **không bằng** xác thực danh tính.
- Web **không** sở hữu session runtime, không tự quyết định tool execution và không tạo dữ liệu telemetries giả.

Không rewrite app, không copy API layer sang page, không chỉnh sửa database trực tiếp và không thêm dependency UI mới cho P0.

## 2. Phần code đã tạo/thay đổi

| File | Mục đích | Tình trạng |
|---|---|---|
| `src/components/studio/StudioStatCard.vue` | Card KPI tái sử dụng, không chứa logic API | Đã thêm |
| `src/components/studio/StudioTabs.vue` | Tab Agent Studio có keyboard/ARIA | Đã thêm |
| `src/components/studio/VoicePipelineStrip.vue` | Pipeline summary dựa trên Template bindings thực | Đã thêm |
| `src/views/OverviewView.vue` | Counts từ Pinia + ready, sessions, providers từ server API | Đã thêm |
| `src/views/DevicesView.vue` | Device fleet dùng read-model có sẵn, không giả online | Đã thêm |
| `src/config/navigation.ts` | Nhóm Workspace / Voice & Devices / Infrastructure | Đã sửa |
| `src/components/SidebarContent.vue` | Branding + nhóm điều hướng | Đã sửa |
| `src/router/index.ts` | `/` → `/overview`; route `/devices` | Đã sửa |
| `src/layouts/AdminShell.vue` | Workspace rộng tối đa 1600px | Đã sửa |
| `src/style.css` | Studio semantic tokens và panel/logo primitive | Đã sửa |
| `src/composables/useTheme.ts` | Dark-first, vẫn tôn trọng lựa chọn đã lưu | Đã sửa |
| `src/components/admin/AgentCard.vue` | Card AI Agent mới; giữ event `open` / `addDevice` | Đã sửa |
| `src/pages/agents/AgentDetailPage.vue` | Agent Workbench tab + pipeline summary; giữ dialog CRUD | Đã sửa |
| `src/i18n/messages.ts` | Labels mới en/vi và bỏ mô tả mock đã lỗi thời | Đã sửa |

### Trạng thái chưa triển khai

- `VoicePlayground.vue`, `VoiceSessionInspector.vue`, `AudioWaveform.vue`, `ReportsView.vue`, `RuntimeEventsPanel.vue`, `DeviceLiveState.vue`.
- Chưa thêm route `/playground`, `/reports` vào sidebar: không đưa link trống và không tự giả dữ liệu kết nối.
- Chưa sửa wire protocol hoặc thêm endpoints Rust trong PR này.

## 3. Đề xuất cấu trúc component giai đoạn tiếp theo

```text
apps/admin-web/src/
  components/
    studio/
      StudioStatCard.vue         # implemented
      StudioTabs.vue             # implemented
      VoicePipelineStrip.vue     # implemented
      VoiceWaveform.vue          # P2: actual levels only
      RuntimeStateChip.vue       # P2: loaded/stale/failed/unknown
    playground/
      VoicePlayground.vue        # P2: mic + playback, client test
      VoiceSessionInspector.vue  # P2: transcript, timeline, tool calls
      AudioDeviceSelector.vue    # P2: permissions, inputs
    devices/
      DeviceConnectionState.vue  # P3: backend connection telemetry
    reports/
      ConversationMetrics.vue    # P3: source-backed metrics
  views/
    OverviewView.vue             # implemented
    DevicesView.vue              # implemented
    PlaygroundView.vue           # P2
    ReportsView.vue              # P3
```

**Không thêm tất cả component upfront**: chỉ tạo P2/P3 khi API/wire semantics ổn định. Không kéo graph editor library vào chỉ để nối 4 node. Component `AiPipeline.vue` hiện tại là editor và giữ nguyên; `VoicePipelineStrip.vue` chỉ là summary.

## 4. Data-flow hiện tại — bắt buộc giữ

```text
AdminShell / Router
    ├─ App.vue → useAuthStore (sessionStorage admin bearer)
    ├─ useAdminStore (read models/cache)
    │    ├─ src/api/agents.ts
    │    ├─ src/api/templates.ts
    │    ├─ src/api/providers.ts
    │    └─ src/api/devices.ts
    └─ useServerStore
         └─ src/api/system.ts → /health, /ready, /api/admin/system
```

### Overview

| UI field | Nguồn | Cách diễn giải |
|---|---|---|
| Agent count | `useAdminStore.agents.length` | Số Agent đã tải |
| Template count | `useAdminStore.templates.length` | Số Template toàn cục |
| Device count | `useAdminStore.devices.length` | **Đã đăng ký**, không phải online |
| Provider count | `useAdminStore.providers.length` | Số entry catalog |
| Runtime Ready | `GET /ready` qua `useServerStore.readiness` | Readiness server; không phải VAD/ASR/TTS đều OK |
| Active Sessions | `GET /api/admin/system` → `sessions.active` | Tổng session active server báo |
| Loaded/Failed providers | `GET /api/admin/system` → `providers.loaded/failed` | Server aggregate, có thể null |
| Database | `GET /api/admin/system` → `database.status` | Trạng thái server báo |

Không hiển thị `0` khi API chưa trả số. Dùng em dash / Unknown cho `null` và lỗi. Overview hiện chỉ refresh snapshot khi mount hoặc yêu cầu refresh; không mô phỏng realtime.

### Devices

- `src/stores/admin.ts::toDevice` hiện đặt `status` bằng `device.enabled ? 'online' : 'offline'`. Đây **không phải trạng thái WebSocket**.
- UI Devices mới **dịch đúng ý nghĩa** thành Enabled/Disabled (quyền admission), không gắn chấm xanh `Online`.
- Thao tác add/edit/delete vẫn từ Agent Detail → `AgentDeviceList` và Admin APIs, dùng revision/`If-Match` có sẵn.
- Để có Online/Offline thật, bổ sung trường độc lập `connection_status` từ server; sau đó đổi view-model để không overload `status`. Việc chuẩn hóa tên field `DeviceStatus` là ticket backend/frontend riêng, không đổi public API trong P0.

### Agent Studio

- `/agents/:agentId?template=:templateId`: tab Studio hiển thị Template Switcher, Pipeline Summary, `AiPipeline`, Prompt/Template configuration.
- Tab External Tools: component `AgentToolAllowlist`; quyền review chỉ áp dụng **External MCP**, Device tools không cần review permission.
- Tab Speakers: `AgentSpeakerPolicy`; cấp quyền cho Agent và Template, tôn trọng `Off/Optional/Required` và qualification.
- Tab Devices: `AgentDeviceList` hiện hữu.
- `selectedTemplate` chỉ được lấy từ linked Templates, không chọn template ngoài Agent.
- Mutations chạy qua store/API hiện có: `If-Match` revision, 409 conflict → reload, không silent overwrite.
- Provider không có binding: giao diện tóm tắt ghi `Server default`, **không suy ra tên Provider hay khẳng định runtime đã loaded**.

## 5. Endpoint contract và môi trường

Không thay thế những API hiện hữu trong `apps/admin-web/docs/API_Integration_Guide.md` và `docs/api/00-all-apis.postman_collection.json`.

```http
GET /health                         # process liveness, text
GET /ready                          # voice readiness, text
GET /api/admin/system               # aggregate, Bearer admin
GET /api/admin/agents               # resource catalog
GET /api/admin/templates
GET /api/admin/providers
GET /api/admin/devices
```

- `src/api/*` là lớp duy nhất gọi HTTP. Không viết `fetch('/api/admin/...')` trực tiếp trong Vue page.
- `Authorization: Bearer <admin_token>` cho `/api/admin/*`. Token nằm ở auth store/`sessionStorage`; không commit token hoặc bỏ vào public `VITE_*` production.
- Prefer relative origin `/api/admin/*`; proxy Vite cho local dev.
- Nếu `GET /api/admin/system` thất bại: hiển thị Unknown, giữ các counts admin nếu vẫn load được.
- `If-Match: "<revision>"` cho mutation đã yêu cầu; 409 giữ draft, reload resource, báo xung đột.

## 6. Thiết kế kết nối Playground (P2, chưa có)

### Không được dùng ngay Admin token để mở voice WS

```text
Browser Playground
  │ chọn Agent + linked Template
  ▼
Admin API  ── yêu cầu cấp phiên test hợp lệ (endpoint **đề xuất, chưa có**)
  │ trả token ngắn hạn/scoped + session identifier
  ▼
Browser WS Client → Rust voice admission (test client identity)
  │ đúng Protocol V1: ClientHello → ServerHello → Ready
  │ audio uplink Opus 16k mono 60ms; downlink Opus 24k mono 60ms
  ▼
Server-owned session: ASR → LLM → TTS → device/client
  │
  ▼
Inspector: STT, LLM responses, playback, observed events
```

### API cần thiết kế riêng

- `POST /api/admin/playground/sessions` — **đề xuất**, chưa được triển khai: yêu cầu session test, validate Agent và linked Template, cấp ephemeral credential scope riêng; không expose Admin bearer cho client voice.
- `GET /api/admin/playground/sessions/{id}/events` — **đề xuất**: server-authorized SSE cho inspector. Chỉ sự kiện thuộc test session, không cho Web quyền tạo/cập nhật session state.
- `DELETE /api/admin/playground/sessions/{id}` — **đề xuất**: kết thúc/thu hồi test permission, đảm bảo giải phóng tài nguyên.

Trước khi viết endpoint phải review auth/admission, WebSocket token model, không giả định endpoint đã tồn tại. Không gửi PCM raw vào WebSocket đang yêu cầu Opus. `MediaRecorder` của browser thường không đảm bảo Opus 16 kHz/60 ms: cần codec worker/transcoding đã kiểm chứng và xử lý permission/abort/stop. Không log nội dung transcript/audio vào console, không giữ audio mặc định; lịch sử opt-in theo chính sách hiện hành.

### Lifecycle / error UX

```text
idle → requesting microphone → connecting → ready
     → listening → processing → speaking → ready
     → closing → idle
                         ↘ error (retry/close)
```

- Khi thay Agent/Template, đóng session test cũ rồi xin mới. Không đổi mutable profile giữa WS.
- Browser refresh/unmount: close session best effort; server TTL xử lý cleanup.
- Không hiển thị ước lượng latency giả. Metrics chỉ khi backend báo event timestamp đáng tin cậy.
- Không thêm autoplay âm thanh ngoài user gesture.
- Kiểm thử cancel/abort, network close, token expired, device busy và speaker gate.

## 7. Reports/Realtime (P3, chưa có)

- Reports chỉ hiển thị khi backend thực sự có aggregation APIs; không tự đếm từ list chưa paginate đủ hoặc từ frontend localStorage.
- Tách `enabled` admission khỏi `connected` runtime trên Device trước khi xây live fleet.
- Đề xuất API phải có schema, pagination/filter/time range và authorization policy trong OpenAPI/Postman trước khi frontend gọi.
- SSE subscriptions dùng route riêng hoặc xác thực thích hợp, không để token trong URL, không tự reconnect vô hạn.
- Provider statuses: `configured` không đồng nghĩa `loaded`, `failed` và `stale` phải hiển thị phân biệt.

## 8. Visual spec

- Dark-first; giữ light mode và persisted preference.
- Tokens: `--studio-violet`, `--studio-cyan` dùng cho AI/Voice emphasis; semantic status dùng existing `--success`, `--warning`, `--danger`.
- `studio-panel`, `studio-logo` tái sử dụng thay cho styles cục bộ.
- Không xây graph engine/canvas cho pipeline tuyến tính.
- Kiểm tra contrast, focus, keyboard, mobile 360px, reduce-motion và không gây horizontal scroll trừ table có overflow chủ ý.

## 9. Gate trước khi merge

```bash
cd apps/admin-web
npm ci
npm run typecheck
npm run test
npm run build
```

Manual checks:
1. Mở `/` chuyển `/overview`; browser back/forward, mọi navigation link hoạt động.
2. Admin token hợp lệ → overview nhận counts thật; token thiếu/401 → không hiển thị snapshot giả.
3. `/ready` lỗi, `/api/admin/system` lỗi → unknown, không crash.
4. Click Agent card → tabs Studio/Tools/Speakers/Devices; dialogs, Template switcher còn sử dụng được.
5. Unbound provider → text Server default, không tuyên bố Provider cụ thể.
6. Devices: Enabled/Disabled là quyền admission, không hiển thị Online giả.
7. Regression: add/edit/delete Agents, Templates, Providers, Device và Speaker enrollment.
8. Accessibility: keyboard tab navigation, focus visible, theme toggle, vi/en, màn hình 360px.
9. Không có route Playground/Reports hoặc network call mới tới endpoint chưa có.
10. Không thay đổi semantics WS, Admin auth, SQLite source-of-truth.

## 10. Kế hoạch còn lại

- **P1 UI polish:** refactor Template/Provider cards, làm rõ provider binding và runtime state; tránh thêm abstraction trùng.
- **P2 browser voice:** chốt test-session auth, protocol client Opus, inspector mới được implement Playground; tích hợp voice waveform với audio thực.
- **P3 telemetry:** backend reports/session stream và Device live state; khi đó thêm navigation `/reports` và Overview live widgets.
- Sau mỗi pha cập nhật `apps/admin-web/README.md`, tài liệu API, Postman và contract tests tương ứng.

**Nguồn sự thật:** code Rust + endpoint thực đang chạy. Tài liệu lịch sử `docs/*` nếu trái code/API mới phải được chỉnh rõ superseded, không copy mock shape vào giao diện.
