# Web Admin API Integration Guide

## Vue Web ↔ Rust Voice Agent Server

**Nguồn contract:** `docs/api/00-all-apis.postman_collection.json` của branch triển khai tương ứng và các Rust routes hiện tại. Không giả định collection cũ trên `dev-test` là nguồn duy nhất.

Tài liệu này ghi nhận việc đồng bộ Vue Web Admin với Admin API thật của Rust server. Migration từ mock/localStorage đã được thực hiện trong code; các ví dụ migration bên dưới là lịch sử, không phải hiện trạng. Phạm vi được chia theo các domain UI chính:

- System
- Agents
- Devices
- Templates
- Providers
- Provider Adapters
- MCP
- History

Mục tiêu là giữ UI hiện tại nhưng chuyển **source of truth** sang server/database.

---

> **Cập nhật 2026-10-08 — Voice Agent Studio:** Xem [component/UX/API map mới](voice-agent-studio-redesign.md). Trang `/overview` lấy management counts từ `useAdminStore`, kết hợp `GET /ready` và `GET /api/admin/system`; trang `/devices` hiển thị registered devices và Enabled/Disabled, **không hiển thị WebSocket online** từ `device.enabled`. Route `/playground` và `/reports`, ephemeral test session, SSE, realtime waveform **chưa có**. Không gọi các endpoint đề xuất như thể đã tồn tại.

# 1. Nguyên tắc bắt buộc

## 1.1 Server là source of truth

Không tiếp tục lưu Agent, Device, Template, Provider, Provider Binding hoặc Agent–Template relationship trong `localStorage` như database giả.

Frontend chỉ giữ:

- cache tạm của API response;
- loading/error state;
- selected filters;
- selected Template trên Agent Detail;
- form draft chưa submit;
- admin token cho session nếu chưa có auth flow khác.

Không tự tạo relationship phía client mà server không xác nhận.

---

## 1.2 Admin API authentication

Mọi `/api/admin/*` request yêu cầu:

```http
Authorization: Bearer <admin_token>
```

Không hard-code token vào component.

Không commit token vào repository.

Không build production bundle với secret cố định qua `VITE_ADMIN_TOKEN`, vì mọi biến `VITE_*` đều được bundle xuống browser.

Cho development/homelab có thể dùng một auth/settings store:

```ts
interface AdminAuthState {
  adminToken: string | null
}
```

Khuyến nghị ban đầu lưu token trong `sessionStorage`, không dùng `localStorage` mặc định.

API client lấy token từ auth store khi gửi request.

---

# 2. Optimistic concurrency / revision

Server dùng `revision` để chống lost update.

Các mutation PATCH/PUT/DELETE có optimistic concurrency phải gửi:

```http
If-Match: "<revision>"
```

Lưu ý revision phải được quote.

Ví dụ:

```ts
headers.set('If-Match', `"${revision}"`)
```

Không dùng:

```http
If-Match: 4
```

Phải là:

```http
If-Match: "4"
```

## 2.1 Sau mutation

Nếu server trả resource mới:

- replace resource trong store bằng response mới;
- dùng `revision` mới từ response;
- không tự `revision++` nếu response đã trả revision thật.

Nếu server trả `204 No Content`:

- xóa relationship/resource khỏi local cache;
- nếu mutation làm parent revision thay đổi nhưng response không trả parent mới, reload parent hoặc relationship endpoint trước mutation tiếp theo.

## 2.2 `409 revision_conflict`

Khi nhận:

```json
{
  "error": {
    "code": "revision_conflict",
    "request_id": "..."
  }
}
```

Frontend phải:

1. không retry mutation tự động;
2. refetch resource mới nhất;
3. cập nhật store;
4. thông báo cho user rằng resource đã thay đổi;
5. nếu đang edit form, giữ draft để user quyết định merge/save lại.

Không silent overwrite dữ liệu mới hơn trên server.

---

# 3. API client architecture

Khuyến nghị structure:

```text
apps/admin-web/src/
├── api/
│   ├── client.ts
│   ├── errors.ts
│   ├── system.ts
│   ├── agents.ts
│   ├── devices.ts
│   ├── templates.ts
│   ├── providers.ts
│   ├── provider-adapters.ts
│   ├── mcp.ts
│   ├── history.ts
│   └── types/
│       ├── common.ts
│       ├── system.ts
│       ├── agents.ts
│       ├── devices.ts
│       ├── templates.ts
│       ├── providers.ts
│       ├── mcp.ts
│       └── history.ts
│
├── stores/
│   ├── auth.ts
│   ├── agents.ts
│   ├── devices.ts
│   ├── templates.ts
│   ├── providers.ts
│   └── system.ts
```

Nếu project hiện đang dùng một `stores/admin.ts` duy nhất thì không bắt buộc tách store ngay. Tuy nhiên HTTP request phải được tách khỏi Vue component.

Không viết trực tiếp trong page:

```ts
fetch('/api/admin/providers')
```

Thay bằng:

```ts
providersApi.list(...)
```

và page gọi store/action tương ứng.

---

# 4. Shared HTTP client

Tạo một client trung tâm có khả năng xử lý:

- JSON response;
- text response;
- `204 No Content`;
- Blob/audio response;
- raw binary upload;
- Bearer token;
- `If-Match`;
- structured server error;
- abort signal.

Ví dụ API contract:

```ts
export interface RequestOptions {
  revision?: number
  signal?: AbortSignal
  headers?: HeadersInit
}

export interface ApiErrorBody {
  error: {
    code: string
    request_id?: string
  }
}
```

Không ép mọi response qua `response.json()`.

Các endpoint sau không phải JSON thông thường:

```text
GET  /health                             -> text
GET  /ready                              -> text
POST /providers/:key/test/tts           -> audio/wav Blob
DELETE ... thành công                    -> có thể là 204
```

---

# 5. Error mapping chung

UI phải map ít nhất các lỗi:

```text
400 invalid_query
400 validation_failed
400 invalid_if_match
400 invalid_json
400 invalid_content_type
401 unauthorized
404 not_found
409 revision_conflict
409 default_template_conflict
409 agent_in_use
409 device_in_use
409 template_in_use
409 provider_in_use
409 mcp_server_in_use
503 database_unavailable
503 database_busy
```

Không chỉ hiển thị:

```text
Request failed
```

Ví dụ:

```text
provider_in_use
→ "Provider này vẫn đang được Template sử dụng. Hãy unlink Provider khỏi các Template trước khi xoá."
```

Luôn log/surface `request_id` trong phần technical details để debug server log.

---

# 6. Environment và Vite proxy

Khuyến nghị browser gọi relative URL:

```text
/api/admin/...
/health
/ready
```

Development proxy:

```ts
server: {
  proxy: {
    '/api': 'http://127.0.0.1:8000',
    '/health': 'http://127.0.0.1:8000',
    '/ready': 'http://127.0.0.1:8000',
    '/voice': 'http://127.0.0.1:8000',
    '/mcp': 'http://127.0.0.1:8000',
  },
}
```

Production ưu tiên serve Web và Rust API cùng origin hoặc dùng reverse proxy.

---

# 7. System

## 7.1 APIs

```http
GET /health
GET /ready
GET /api/admin/system
```

`/health` dùng cho process liveness.

`/ready` dùng cho readiness nhận voice connection.

`/api/admin/system` là endpoint chính cho System page.

Response System hiện cung cấp aggregate an toàn tương đương:

```json
{
  "status": "ready",
  "version": "...",
  "uptime_seconds": 1000,
  "database": {
    "enabled": true,
    "status": "ready"
  },
  "providers": {
    "configured": 8,
    "loaded": 7,
    "stale": 1,
    "failed": 0
  },
  "sessions": {
    "active": 1
  }
}
```

Nếu database unavailable, provider counts có thể là `null`.

## 7.2 System page mapping

System page nên lấy:

```text
Server status       <- /api/admin/system.status
Version             <- version
Uptime              <- uptime_seconds
Database status     <- database.status
Providers configured<- providers.configured
Providers loaded    <- providers.loaded
Providers stale     <- providers.stale
Providers failed    <- providers.failed
Active sessions     <- sessions.active
```

Không fake OS/RAM/CPU nếu API chưa cung cấp.

Không lấy `/health` rồi diễn giải thành toàn bộ runtime healthy.

---

# 8. Agents

## 8.1 API list

```http
GET    /api/admin/agents?page=1&page_size=50&enabled=true&sort=key
POST   /api/admin/agents
GET    /api/admin/agents/{agent_key}
PATCH  /api/admin/agents/{agent_key}
DELETE /api/admin/agents/{agent_key}
```

Template relationship:

```http
GET    /api/admin/agents/{agent_key}/templates?page=1&page_size=50
PUT    /api/admin/agents/{agent_key}/templates/{template_key}
DELETE /api/admin/agents/{agent_key}/templates/{template_key}
PUT    /api/admin/agents/{agent_key}/default-template/{template_key}
```

MCP relationship:

```http
GET    /api/admin/agents/{agent_key}/mcp-bindings
PUT    /api/admin/agents/{agent_key}/mcp-bindings/{mcp_server_key}
DELETE /api/admin/agents/{agent_key}/mcp-bindings/{mcp_server_key}
```

---

## 8.2 Agent List page

Route UI:

```text
/agents
```

Load:

```http
GET /api/admin/agents?page=1&page_size=50&enabled=true&sort=name
```

Agent card chỉ lấy Agent-level information:

```text
name
description
enabled
revision
```

Không hiển thị Agent như owner của:

```text
language
prompt
providers
```

Các field đó thuộc Template.

Nếu UI cần template count trên Agent card, gọi relationship API khi card visible hoặc khi mở detail; không suy ra từ mock state.

---

## 8.3 Create Agent

```http
POST /api/admin/agents
Content-Type: application/json
```

Example:

```json
{
  "key": "home_assistant",
  "name": "Home Assistant",
  "description": "Voice assistant for home devices"
}
```

Sau create:

1. insert response vào Agent store;
2. điều hướng sang Agent Detail hoặc flow assign Template;
3. không tự tạo Provider/Prompt trên Agent.

Nếu UX yêu cầu Agent luôn có ít nhất một Template thì Web phải thực hiện flow rõ ràng:

```text
Create Agent
→ Create/Choose Template
→ Assign Template
→ Set Default Template
```

Đây là nhiều server mutations, không giả lập thành một mutation duy nhất.

---

## 8.4 Agent Detail

Route:

```text
/agents/:agentKey?template=:templateKey
```

Load sequence:

```text
1. GET /agents/:agentKey
2. GET /agents/:agentKey/templates
3. Resolve selected Template
4. GET /templates/:templateKey
5. GET /templates/:templateKey/providers
6. Hydrate Provider instances cần cho AI Pipeline
7. Load Devices rồi filter theo agent_key
```

Selected Template resolution:

```text
route.query.template nếu đang linked
        ↓
không có / invalid
        ↓
Template có is_default=true
        ↓
không có default
        ↓
first linked template / empty state
```

Không lấy default Template từ mock field nếu Agent response không cung cấp nó.

### Agent Template response

`GET /agents/{agent_key}/templates` trả các item có:

```text
key
name
language
enabled
is_default
```

Response cũng mang Agent `revision`; dùng revision mới nhất này cho các mutation Agent–Template tiếp theo.

---

## 8.5 Assign Template

```http
PUT /api/admin/agents/{agent_key}/templates/{template_key}
If-Match: "<agent_revision>"
```

Sau thành công:

```text
refetch Agent Templates
update Agent revision
optionally select template vừa assign
```

Không append relationship local trước khi server success.

---

## 8.6 Set default Template

```http
PUT /api/admin/agents/{agent_key}/default-template/{template_key}
If-Match: "<agent_revision>"
```

Template phải đã hợp lệ cho Agent.

Sau success:

- refetch Agent Templates;
- update `is_default` state từ server;
- không chỉ toggle boolean client-side.

---

## 8.7 Unlink Template khỏi Agent

```http
DELETE /api/admin/agents/{agent_key}/templates/{template_key}
If-Match: "<agent_revision>"
```

Nếu Template đang default, server trả:

```text
default_template_conflict
```

UI flow:

```text
Nếu default
→ bắt user chọn default khác
→ Set Default Template
→ nhận revision mới
→ Unlink bằng revision mới
```

Không reuse revision cũ sau `Set Default`.

---

## 8.8 Delete Agent

```http
DELETE /api/admin/agents/{agent_key}
If-Match: "<agent_revision>"
```

Server chỉ delete khi không còn:

- active Device;
- Template assignment;
- MCP binding;
- History reference.

Nếu còn dependency:

```text
409 agent_in_use
```

UI không được giả định delete cascade.

Confirmation dialog nên nói rõ Agent phải được detach dependency trước.

---

# 9. Devices

## 9.1 APIs

```http
GET    /api/admin/devices?page=1&page_size=50&enabled=true&sort=device_id
POST   /api/admin/devices
GET    /api/admin/devices/{device_id}
PATCH  /api/admin/devices/{device_id}
DELETE /api/admin/devices/{device_id}
```

---

## 9.2 Device ↔ Template semantics

Device hỗ trợ Template override thật trên server.

Create/Patch có:

```json
{
  "agent_key": "home",
  "template_key": "assistant_vi"
}
```

`template_key` là optional.

```text
template_key == null / omitted
→ dùng Agent default Template
```

Nếu có override, Template phải:

- enabled;
- đã assigned vào Agent tương ứng.

---

## 9.3 Create Device

```http
POST /api/admin/devices
```

Example:

```json
{
  "device_id": "67:28:43:1D:95:90",
  "agent_key": "home",
  "template_key": "assistant_vi",
  "name": "Living Room",
  "description": "Living room ESP32",
  "metadata_json": {
    "zone": "living_room"
  }
}
```

Nếu user chọn `Use Agent Default`:

- omit `template_key` khi create.

---

## 9.4 Edit Device

```http
PATCH /api/admin/devices/{device_id}
If-Match: "<device_revision>"
```

Để clear Template override:

```json
{
  "template_key": null
}
```

UI:

```text
Template
● Use agent default
○ Override template
```

Override selector chỉ list Template đã assigned cho Agent.

---

## 9.5 Agent Detail Device list

API Device list hiện không khai báo query `agent_key`.

Không gửi query param chưa được contract hỗ trợ.

Trong scope homelab hiện tại có thể:

1. load device pages;
2. cache vào Device store;
3. filter `device.agent_key === agent.key` phía frontend.

Nếu số lượng Device lớn, cần backend bổ sung filter riêng; không tự invent `/agents/:key/devices` nếu server chưa có.

---

## 9.6 Delete Device

```http
DELETE /api/admin/devices/{device_id}
If-Match: "<device_revision>"
```

Server block khi Device còn History reference:

```text
409 device_in_use
```

Không purge history tự động khi delete Device.

---

# 10. Templates

## 10.1 APIs

CRUD:

```http
GET    /api/admin/templates
POST   /api/admin/templates
GET    /api/admin/templates/{template_key}
PATCH  /api/admin/templates/{template_key}
DELETE /api/admin/templates/{template_key}
```

Relationships:

```http
GET    /api/admin/templates/{template_key}/agents?page=1&page_size=50
GET    /api/admin/templates/{template_key}/providers
PUT    /api/admin/templates/{template_key}/providers/{provider_type}
DELETE /api/admin/templates/{template_key}/providers/{provider_type}
```

Agent relationship endpoints cũng có thể được gọi từ Template UI:

```http
PUT /api/admin/agents/{agent_key}/templates/{template_key}
PUT /api/admin/agents/{agent_key}/default-template/{template_key}
```

---

## 10.2 Template List page

Route:

```text
/templates
```

List API hỗ trợ:

```text
page
page_size
enabled
q
language
sort
```

Sort:

```text
key
-key
name
-name
language
```

Response có:

```text
items
page
page_size
max_page_size
total
total_pages
```

Do đó search/filter/pagination phải dùng server query thay vì filter toàn bộ local array.

Example:

```http
GET /api/admin/templates?page=1&page_size=24&q=vietnamese&language=vi-VN&sort=name
```

---

## 10.3 Template usage: Agents đang sử dụng

```http
GET /api/admin/templates/{template_key}/agents?page=1&page_size=50
```

Response có:

```text
template_key
revision
page
page_size
max_page_size
total
items[]
```

Agent item:

```text
key
name
enabled
is_default
```

Dùng endpoint này cho:

- `Used by N agents` trên Template card;
- Template Detail `Agents using this template`;
- blast-radius warning trước edit/delete.

### Tránh N+1 quá mức

Nếu Template page có nhiều cards nhưng UI bắt buộc hiển thị usage count:

- chỉ hydrate usage cho cards trên page hiện tại;
- request `page_size=1` vì chỉ cần `total`;
- giới hạn concurrency, ví dụ 4 request cùng lúc;
- cache theo `template_key`;
- invalidate cache sau assign/unlink Template.

Không request usage cho toàn bộ catalog 200 items cùng lúc.

---

## 10.4 Create Template

```http
POST /api/admin/templates
```

Example:

```json
{
  "key": "assistant_vi",
  "name": "Vietnamese Assistant",
  "description": "Default Vietnamese template",
  "language": "vi-VN",
  "prompt": "Bạn là trợ lý giọng nói. Trả lời ngắn gọn, rõ ràng."
}
```

Template là global reusable resource.

Create Template từ Templates page:

```text
Create resource
→ không auto-link Agent
```

Create Template từ Agent Detail có thể thực hiện:

```text
Create Template
→ Assign Template to Agent
→ optionally Set Default
```

nhưng phải là explicit API sequence.

---

## 10.5 Template Provider Bindings / AI Pipeline

AI Pipeline của Template phải lấy từ:

```http
GET /api/admin/templates/{template_key}/providers
```

Response shape:

```json
{
  "template_key": "assistant_vi",
  "revision": 4,
  "bindings": {
    "vad": {
      "provider_key": "vad_silero",
      "enabled": true
    },
    "asr": {
      "provider_key": "asr_zipformer",
      "enabled": true
    },
    "llm": {
      "provider_key": "llm_openai",
      "enabled": true
    },
    "tts": {
      "provider_key": "tts_zerotts",
      "enabled": true
    }
  }
}
```

Đây là desired configuration persisted trong DB.

Không diễn giải endpoint này là loaded runtime state.

Dùng `revision` response này làm revision mới nhất của Template trước bind/unlink tiếp theo.

---

## 10.6 Bind Provider vào Template

```http
PUT /api/admin/templates/{template_key}/providers/{provider_type}
If-Match: "<template_revision>"
Content-Type: application/json
```

Body:

```json
{
  "provider_key": "llm_openai"
}
```

Current DB provider types:

```text
vad
asr
llm
tts
```

Không gửi:

```text
vision
```

vào endpoint này nếu backend contract chưa mở rộng.

Sau bind:

```text
refetch Template Provider Bindings
update Template revision
```

Nếu type đã có provider, UI phải xem PUT này như replace binding theo server semantics; confirmation nên hiển thị provider cũ → provider mới.

---

## 10.7 Unlink Provider khỏi Template

```http
DELETE /api/admin/templates/{template_key}/providers/{provider_type}
If-Match: "<template_revision>"
```

Action này:

- xóa một Provider Binding;
- tăng Template revision;
- không delete Provider Instance;
- không hot-reload runtime hiện tại.

Sau success:

- refresh bindings;
- update revision;
- Pipeline node chuyển thành empty provider slot.

---

## 10.8 Edit Template

```http
PATCH /api/admin/templates/{template_key}
If-Match: "<template_revision>"
```

Editable:

```text
name
description
language
prompt
enabled
```

`key` immutable.

Nếu Template được nhiều Agent dùng, UI nên load `/templates/:key/agents` và cảnh báo:

```text
Changes affect every Agent using this Template for future resolved configuration/session lifecycle according to server behavior.
```

Không duplicate Template tự động chỉ vì user edit shared Template.

---

## 10.9 Delete Template

```http
DELETE /api/admin/templates/{template_key}
If-Match: "<template_revision>"
```

Server chỉ delete khi không còn:

- active Agent assignment;
- Provider binding;
- Device override;
- History reference.

Nếu còn dependency:

```text
409 template_in_use
```

Quan trọng: API **không cascade unlink**.

UI delete flow phải yêu cầu user giải phóng dependency trước.

---

# 11. Providers

## 11.1 APIs

CRUD:

```http
GET    /api/admin/providers
POST   /api/admin/providers
GET    /api/admin/providers/{provider_key}
PATCH  /api/admin/providers/{provider_key}
DELETE /api/admin/providers/{provider_key}
```

Usage/capabilities:

```http
GET /api/admin/providers/{provider_key}/templates?page=1&page_size=50
GET /api/admin/providers/{provider_key}/capabilities
```

Diagnostics:

```http
POST /api/admin/providers/{provider_key}/test/vad
POST /api/admin/providers/{provider_key}/test/asr
POST /api/admin/providers/{provider_key}/test/llm
POST /api/admin/providers/{provider_key}/test/tts
```

---

## 11.2 Provider Catalog list

Provider list API hỗ trợ:

```text
page
page_size
enabled
q
type=vad|asr|llm|tts
sort=key|-key|name|-name
```

Response có:

```text
items
page
page_size
max_page_size
total
total_pages
facets
```

`facets` dùng trực tiếp cho counts của type tabs.

Không tự count chỉ trên current page.

Ví dụ UI:

```text
All 8   VAD 1   ASR 2   LLM 1   TTS 4
```

lấy count từ server facets.

Search input gọi server-side query `q`.

Debounce khoảng 250–400 ms.

Abort request cũ khi query thay đổi nhanh.

---

## 11.3 Provider status

Provider response có thể chứa runtime-related fields từ server như:

```text
runtime_status
runtime_matches_desired
requires_restart
```

UI phải dùng đúng server values.

Không map `enabled=true` thành `Ready`.

Ví dụ:

```text
enabled=true + runtime_status=loaded + runtime_matches_desired=true
→ Loaded / Ready-like state theo terminology server
```

Nếu:

```text
requires_restart=true
```

show rõ:

```text
Restart required
```

vì PATCH desired provider config không có nghĩa loaded runtime đã đổi ngay.

---

## 11.4 Provider usage / Templates sử dụng Provider

```http
GET /api/admin/providers/{provider_key}/templates?page=1&page_size=50
```

Response có:

```text
provider_key
revision
page
page_size
max_page_size
total
items[]
```

Item:

```text
key
name
provider_type
enabled
```

Dùng cho:

```text
Used by N templates
Template names preview
Provider detail usage list
Delete preflight
Edit blast-radius warning
```

Provider card cần usage count có thể request:

```text
page_size=1
```

chỉ để lấy `total`.

Giới hạn concurrency và cache giống Template usage.

---

## 11.5 Create Provider

Trước create, load adapter catalog:

```http
GET /api/admin/provider-adapters?type={provider_type}
```

Flow:

```text
Select Provider Type
→ load compatible adapters
→ select adapter
→ render adapter/config form
→ create Provider Instance
```

Create:

```http
POST /api/admin/providers
```

Example:

```json
{
  "key": "llm_openai",
  "name": "OpenAI Primary",
  "type": "llm",
  "adapter": "openai",
  "config_json": {
    "base_url": "https://api.openai.com/v1",
    "model": "model-name",
    "timeout_ms": 30000,
    "max_tokens": 1024
  },
}
```

Không gửi secret value hay `secret_ref` trong `config_json` hoặc payload API. Biến môi trường được suy ra từ provider key và adapter.

Không còn `secret_ref` trong Admin API; đọc `credential_env` (read-only) từ Provider detail, ví dụ `VOICE_PROVIDER_LLM_ABC_API_KEY`.

---

## 11.6 Edit Provider

```http
PATCH /api/admin/providers/{provider_key}
If-Match: "<provider_revision>"
```

`key` immutable.

`type` không patch được.

Patch desired config có thể yêu cầu restart trước khi loaded runtime khớp desired state.

Sau PATCH:

- replace store item bằng server response;
- inspect `requires_restart`;
- không tự show runtime đã reload.

---

## 11.7 Delete Provider

```http
DELETE /api/admin/providers/{provider_key}
If-Match: "<provider_revision>"
```

Server chỉ delete khi **không còn Template Provider Binding**.

Nếu còn usage:

```text
409 provider_in_use
```

Do đó UI cũ kiểu:

```text
Delete Provider
→ tự unlink khỏi tất cả Template
→ delete
```

KHÔNG còn đúng.

Flow đúng:

```text
View provider usage
→ user unlink khỏi từng Template
→ refresh Provider revision/usage
→ Delete Provider
```

Loaded runtime vẫn immutable cho tới restart theo server contract.

---

# 12. Provider Diagnostics

Provider Detail/Test UI phải route theo `provider.type`.

## 12.1 VAD

```http
POST /api/admin/providers/{provider_key}/test/vad
```

Không request body.

Server chạy một canonical silent VAD frame trên loaded runtime.

Hiển thị:

- probability;
- sample range;
- elapsed time;
- runtime provenance nếu response cung cấp.

Không cho upload arbitrary WAV vào endpoint VAD hiện tại.

---

## 12.2 ASR

```http
POST /api/admin/providers/{provider_key}/test/asr
Content-Type: audio/wav
```

Body là raw WAV bytes.

Expected:

```text
mono
16 kHz
PCM WAV
duration <= 30 s
body <= 5 MiB
```

Browser:

```ts
await api.requestJson(..., {
  method: 'POST',
  body: file,
  headers: { 'Content-Type': 'audio/wav' },
})
```

Không wrap WAV vào JSON/base64.

---

## 12.3 LLM

```http
POST /api/admin/providers/{provider_key}/test/llm
Content-Type: application/json
```

Body:

```json
{
  "input": "Xin chào, hãy trả lời một câu ngắn."
}
```

Không gửi unknown fields nếu server reject strict JSON shape.

---

## 12.4 TTS

```http
POST /api/admin/providers/{provider_key}/test/tts
Content-Type: application/json
```

Body:

```json
{
  "text": "Xin chào, đây là bài kiểm tra TTS.",
  "voice": "maichi",
  "language": "vi-VN"
}
```

`voice` và `language` optional typed overrides.

Success trả:

```text
audio/wav
```

Frontend phải đọc Blob:

```ts
const blob = await response.blob()
const url = URL.createObjectURL(blob)
audio.src = url
```

Khi đổi audio/unmount:

```ts
URL.revokeObjectURL(url)
```

---

# 13. Provider Adapter Catalog

## APIs

```http
GET  /api/admin/provider-adapters
GET  /api/admin/provider-adapters?type={provider_type}
GET  /api/admin/provider-adapters/{adapter}
POST /api/admin/provider-adapters/{adapter}/capabilities/discover
```

Dùng Adapter Catalog cho Create/Edit Provider form.

Không hard-code adapter list trong Vue nếu server đã expose compiled adapter descriptors.

Create flow:

```text
Provider Type
  ↓
GET adapters?type=...
  ↓
Adapter
  ↓
Adapter descriptor/config UI
```

Capability discovery:

```http
POST /api/admin/provider-adapters/{adapter}/capabilities/discover
```

Example:

```json
{
  "selection": {
    "model": "model-name"
  }
}
```

Không assume mọi adapter hỗ trợ discovery; server có thể trả conflict nếu unsupported.

---

# 14. Vision boundary

Hiện Admin DB Provider contract chỉ cho:

```text
vad
asr
llm
tts
```

Vision endpoint hiện nằm ở Voice/MCP surface:

```http
GET  /mcp/vision/explain
POST /mcp/vision/explain
```

Do đó Web Admin không được tự tạo:

```text
/api/admin/providers?type=vision
```

hoặc:

```text
/templates/:key/providers/vision
```

nếu contract server hiện chưa hỗ trợ.

Nếu UI AI Pipeline vẫn có Vision, phải render nó như capability/feature riêng hoặc hide/disable DB binding action cho tới khi server mở rộng provider type.

Không giả lập Vision binding bằng localStorage.

---

# 15. MCP

## 15.1 MCP Server APIs

```http
GET    /api/admin/mcp-servers?page=1&page_size=50
POST   /api/admin/mcp-servers
GET    /api/admin/mcp-servers/{mcp_server_key}
PATCH  /api/admin/mcp-servers/{mcp_server_key}
DELETE /api/admin/mcp-servers/{mcp_server_key}
```

Create example:

```json
{
  "key": "weather",
  "name": "Weather MCP",
  "url": "https://mcp.example.com/mcp",
  "auth": {
    "type": "none"
  },
  "connect_timeout_ms": 5000,
  "request_timeout_ms": 30000
}
```

Server transport cố định `streamable_http` theo contract hiện tại.

Auth support:

```text
none
bearer (không tham số secret)
header + header_name (không tham số secret)
```

Không lưu secret plaintext vào Web state dài hạn.

---

> **MCP Studio cập nhật 2026-10-08:** Trang `/mcp` đã được thêm vào navigation. [Hướng dẫn UX + Rust wire contract](mcp-studio-implementation-guide.md) là tài liệu hiện hành cho create/edit/bind/review. Lưu ý: `GET /mcp-servers` trả `items/page/page_size/max_page_size` **không có** `total`; query `enabled` hiện chưa lọc. `GET /mcp-servers/:key` trả `auth.type`, `credential_env` (read-only) và `headers: {}`; không còn `secret_ref`. Create/PATCH không nhận headers hoặc secret references.

## 15.2 Agent MCP Binding

> **Response thực từ Rust:** `GET /agents/:key/mcp-bindings` trả `{ "items": [{ "server_key": "weather", "enabled": true, "required": false }] }` (không phải `bindings`, không có `revision`). Lấy revision qua `GET /agents/:key`. Tool review dùng `GET/PUT /agents/:key/tool-allowlist`, `If-Match` từ **item.revision**, không dùng Agent revision. Chỉ External MCP cần allowlist.

```http
GET    /api/admin/agents/{agent_key}/mcp-bindings
PUT    /api/admin/agents/{agent_key}/mcp-bindings/{mcp_server_key}
DELETE /api/admin/agents/{agent_key}/mcp-bindings/{mcp_server_key}
```

PUT body:

```json
{
  "enabled": true,
  "required": false
}
```

Mutation dùng Agent revision.

`required=true` hiện bị server reject là unsupported.

Không render checkbox `Required` như fully supported feature nếu backend vẫn reject true.

Delete MCP Server chỉ thành công khi không còn Agent MCP Binding:

```text
409 mcp_server_in_use
```

---

# 16. History

## APIs

```http
GET  /api/admin/history
POST /api/admin/history/purge
```

Filters:

```text
session_id
device_id
agent_id
template_id
role=user|assistant
sort=created_at|-created_at|sequence|-sequence
```

Quan trọng:

```text
device_id / agent_id / template_id trong History filters là integer DB row IDs
```

không phải external `device_id` string hoặc resource key.

Không trộn hai identity loại này trong frontend types.

Có thể define:

```ts
type DbId = number
```

riêng khỏi:

```ts
type AgentKey = string
type TemplateKey = string
type ExternalDeviceId = string
```

---

## 16.1 Purge by session

```json
{
  "session_id": "..."
}
```

## 16.2 Purge by Device DB ID

```json
{
  "device_id": 1
}
```

## 16.3 Purge all

Dangerous:

```json
{
  "all": "all",
  "confirm": "PURGE_ALL_HISTORY"
}
```

UI phải có destructive confirmation riêng.

Không đặt `Purge all` thành icon action cạnh list bình thường.

---

# 17. Page → API matrix

## System page

```text
GET /health
GET /ready
GET /api/admin/system
```

## Agents page

```text
GET  /api/admin/agents
POST /api/admin/agents
```

Optional card relationship hydration:

```text
GET /api/admin/agents/:key/templates
```

## Agent Detail

```text
GET    /api/admin/agents/:key
PATCH  /api/admin/agents/:key
DELETE /api/admin/agents/:key

GET    /api/admin/agents/:key/templates
PUT    /api/admin/agents/:key/templates/:template
DELETE /api/admin/agents/:key/templates/:template
PUT    /api/admin/agents/:key/default-template/:template

GET    /api/admin/templates/:template
GET    /api/admin/templates/:template/providers
PUT    /api/admin/templates/:template/providers/:type
DELETE /api/admin/templates/:template/providers/:type

GET    /api/admin/agents/:key/mcp-bindings
PUT    /api/admin/agents/:key/mcp-bindings/:server
DELETE /api/admin/agents/:key/mcp-bindings/:server

GET/POST/PATCH/DELETE /api/admin/devices...
```

## Templates page

```text
GET  /api/admin/templates
POST /api/admin/templates
GET  /api/admin/templates/:key/agents
```

## Template Detail

```text
GET    /api/admin/templates/:key
PATCH  /api/admin/templates/:key
DELETE /api/admin/templates/:key
GET    /api/admin/templates/:key/agents
GET    /api/admin/templates/:key/providers
PUT    /api/admin/templates/:key/providers/:type
DELETE /api/admin/templates/:key/providers/:type
```

## Providers page

```text
GET  /api/admin/providers
POST /api/admin/providers
GET  /api/admin/providers/:key/templates
GET  /api/admin/provider-adapters
GET  /api/admin/provider-adapters?type=...
```

## Provider Detail

```text
GET    /api/admin/providers/:key
PATCH  /api/admin/providers/:key
DELETE /api/admin/providers/:key
GET    /api/admin/providers/:key/templates
GET    /api/admin/providers/:key/capabilities
POST   /api/admin/providers/:key/test/vad
POST   /api/admin/providers/:key/test/asr
POST   /api/admin/providers/:key/test/llm
POST   /api/admin/providers/:key/test/tts
```

## MCP page

```text
GET/POST /api/admin/mcp-servers
GET/PATCH/DELETE /api/admin/mcp-servers/:key
```

## History page

```text
GET  /api/admin/history
POST /api/admin/history/purge
```

---

# 18. Store/cache strategy

Không bắt buộc thêm TanStack Query trong ticket này nếu project đang dùng Pinia.

Có thể dùng Pinia với cache maps:

```ts
agentsByKey: Record<string, Agent>
templatesByKey: Record<string, AgentTemplate>
providersByKey: Record<string, Provider>
devicesById: Record<string, Device>
```

Relationship cache riêng:

```ts
agentTemplatesByAgentKey
templateBindingsByTemplateKey
templateAgentsByTemplateKey
providerTemplatesByProviderKey
agentMcpBindingsByAgentKey
```

Mỗi relationship cache nên có:

```text
loading
loadedAt
error
```

Invalidation rules:

```text
Assign/Unlink Agent Template
→ invalidate agentTemplates
→ invalidate templateAgents

Set Default Template
→ invalidate agentTemplates
→ invalidate templateAgents

Bind/Unlink Provider
→ invalidate templateBindings
→ invalidate providerTemplates

Create/Patch/Delete Provider
→ invalidate provider list

Create/Patch/Delete Template
→ invalidate template list

Create/Patch/Delete Device
→ invalidate device list
```

---

# 19. Avoid stale revision bugs

Đây là lỗi frontend dễ gặp nhất.

Ví dụ sai:

```text
Agent revision = 3
PUT set default with If-Match "3"
server revision -> 4
DELETE unlink another template with If-Match "3"
→ 409 revision_conflict
```

Flow đúng:

```text
Mutation A success
→ consume response/refetch relationship
→ store revision 4
→ Mutation B dùng revision 4
```

Tương tự Template:

```text
Bind Provider
→ Template revision tăng
→ Unlink provider tiếp theo phải dùng revision mới
```

Không giữ revision trong component form lâu hơn cần thiết nếu store đã refresh.

---

# 20. Delete semantics tổng hợp

API mới chủ yếu dùng **conditional hard-delete**, không cascade cleanup tùy ý.

## Agent

```text
DELETE blocked nếu còn Device / Template assignment / MCP binding / History
```

## Device

```text
DELETE blocked nếu còn History
```

## Template

```text
DELETE blocked nếu còn Agent assignment / Provider binding / Device override / History
```

## Provider

```text
DELETE blocked nếu còn Template Provider Binding
```

## MCP Server

```text
DELETE blocked nếu còn Agent MCP Binding
```

Web phải phản ánh đúng dependency này.

Không tự động cascade unlink/purge trừ khi có một explicit multi-step UI do user xác nhận.

---

# 21. UI loading patterns

## List pages

Dùng skeleton/card placeholders khi load lần đầu.

Khi filter/search đổi:

- giữ layout ổn định;
- abort request cũ;
- tránh flash toàn page nếu có cached data.

## Detail page

Load resource chính trước.

Sau đó load relationship sections độc lập:

```text
Agent core
Template switcher
AI Pipeline
Devices
MCP
```

Không block toàn Agent Detail chỉ vì một secondary relationship request fail.

Ví dụ Provider usage fail thì vẫn cho xem Provider configuration.

---

# 22. UI error boundaries theo section

Ví dụ Agent Detail:

```text
Agent load fail
→ page-level error

Template Provider Bindings fail
→ Pipeline section error + Retry

Devices fail
→ Devices section error + Retry

MCP fail
→ MCP section error + Retry
```

Không chuyển tất cả lỗi thành redirect.

---

# 23. Implementation phases

## Phase 1 — API foundation

- [ ] `api/client.ts`
- [ ] Bearer auth
- [ ] `If-Match`
- [ ] `ApiError`
- [ ] JSON/text/blob/204 support
- [ ] request abort support
- [ ] remove direct `fetch` from pages

## Phase 2 — System

- [ ] `/health`
- [ ] `/ready`
- [ ] `/api/admin/system`
- [ ] remove fake server stats

## Phase 3 — Agents + Templates relationship

- [ ] list/create/get/patch Agent
- [ ] Agent Detail uses server Agent
- [ ] list Agent Templates
- [ ] Template Switcher from API
- [ ] assign Template
- [ ] unlink Template
- [ ] set default Template
- [ ] delete Agent

## Phase 4 — Templates

- [ ] list/search/filter/pagination
- [ ] create/edit/delete
- [ ] Agents usage
- [ ] Provider bindings
- [ ] AI Pipeline from bindings
- [ ] bind/unlink Provider

## Phase 5 — Devices

- [ ] list/create/get/patch/delete
- [ ] Template override
- [ ] Agent default vs Device override labels

## Phase 6 — Providers

- [ ] list/search/filter/facets/pagination
- [ ] create/edit/delete
- [ ] Adapter Catalog
- [ ] usage by Template
- [ ] runtime status fields
- [ ] capabilities

## Phase 7 — Provider diagnostics

- [ ] VAD test
- [ ] ASR WAV upload
- [ ] LLM test
- [ ] TTS WAV playback

## Phase 8 — MCP

- [ ] MCP Server CRUD
- [ ] Agent MCP bindings
- [ ] delete constraints

## Phase 9 — History

- [ ] list/filter/pagination
- [ ] scoped purge
- [ ] dangerous purge-all confirmation

## Phase 10 — Remove mock persistence

- [ ] delete localStorage database code
- [ ] remove mock seed as production source
- [ ] retain optional fixture/mocks only for tests/Storybook if needed

---

# 24. Acceptance criteria

## Foundation

- [ ] Không page nào gọi Admin API trực tiếp bằng ad-hoc fetch.
- [ ] Admin Bearer token được centralized.
- [ ] Revision/If-Match được centralized.
- [ ] 204 response không gây JSON parse error.
- [ ] TTS Blob hoạt động.
- [ ] ASR binary upload hoạt động.

## Agents

- [ ] Agent list lấy từ API thật.
- [ ] Create/Edit Agent dùng API thật.
- [ ] Agent Detail lấy linked Templates từ API.
- [ ] Template Switcher lấy `is_default` từ server.
- [ ] Set Default dùng revision mới nhất.
- [ ] Unlink default Template bị UI xử lý đúng.
- [ ] Delete Agent không giả cascade.

## Devices

- [ ] Device lấy từ API thật.
- [ ] Create/Edit Device hỗ trợ `template_key`.
- [ ] `template_key=null` thể hiện Use Agent Default.
- [ ] Delete xử lý `device_in_use`.

## Templates

- [ ] Template list dùng server search/filter/pagination.
- [ ] `total`/`total_pages` dùng từ server.
- [ ] Agent usage count dùng API relationship.
- [ ] AI Pipeline dùng `/templates/:key/providers`.
- [ ] Bind/Unlink Provider dùng Template revision.
- [ ] Delete xử lý `template_in_use`.

## Providers

- [ ] Provider list dùng q/type/enabled/sort phía server.
- [ ] Type tab counts dùng `facets`.
- [ ] Provider usage lấy từ `/providers/:key/templates`.
- [ ] Create form dùng Provider Adapter Catalog.
- [ ] Edit hiển thị restart requirement nếu server trả.
- [ ] Delete xử lý `provider_in_use`.
- [ ] Không tự unlink tất cả Template khi delete.

## Diagnostics

- [ ] VAD test không gửi body.
- [ ] ASR gửi raw `audio/wav`.
- [ ] LLM gửi exact JSON shape.
- [ ] TTS đọc và play `audio/wav` Blob.

## System

- [ ] System page dùng `/api/admin/system`.
- [ ] Không fake CPU/RAM/version/provider status.

## MCP

- [ ] MCP CRUD dùng API thật.
- [ ] Agent MCP binding dùng Agent revision.
- [ ] `required=true` không được UI quảng bá như supported nếu server reject.

## Quality

- [ ] Không còn production source-of-truth trong localStorage mock DB.
- [ ] Không hard-code Provider adapter catalog nếu server cung cấp descriptor.
- [ ] Không hard-code Provider usage counts.
- [ ] Không fake runtime health.
- [ ] Không expose secret plaintext.
- [ ] Không gửi `vision` vào Template Provider DB endpoints hiện tại.
- [ ] `npm run typecheck` pass.
- [ ] `npm run build` pass.

---

# 25. Final domain flow

Web phải phản ánh đúng relationship server:

```text
Provider Instance
       ↑
       │
Template Provider Binding
       │
       ↓
Agent Template
       ↑
       │
Agent Template Assignment
       │
       ↓
Agent
       │
       └── Devices
              │
              └── optional Template override
```

UI mapping:

```text
Agents
→ identity + assigned Templates + Devices + MCP

Templates
→ reusable language/prompt + AI Pipeline + Agent usage

Providers
→ global AI infrastructure + runtime state + Template usage + diagnostics

System
→ server/readiness/database/provider/session aggregate
```

Không quay lại mô hình cũ:

```text
Agent owns prompt
Agent owns language
Agent owns provider bindings
```

Model đúng là:

```text
Agent
→ assigned Template
→ language + prompt + Provider bindings
```

Đây phải là boundary thống nhất giữa Vue Web, Admin API và Rust runtime configuration.
