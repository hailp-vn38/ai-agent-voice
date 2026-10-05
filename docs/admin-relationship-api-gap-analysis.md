# Đánh giá gap Admin API và kế hoạch Web Admin

> **Cập nhật contract:** [ADR-0073](adr/0073-required-database-and-device-admission.md)
> thay thế mọi clause optional database/admission trong tài liệu lịch sử này.
> Database và Device admission luôn bật; xóa hai config key cũ khỏi mọi ví dụ trước
> khi sử dụng. [Flow 08](flows/08-database-device-enrollment.md) là hướng dẫn hiện hành.


**Trạng thái:** P0 relationship read/unlink hoàn thành tại commit `22d500b` (2026-10-01). P1
control-plane đã hoàn thành phần có contract cụ thể. P3 deletion đã hoàn thành theo ADR-0070;
Vision Provider sẽ trở thành Database Provider Instance nhưng còn cần contract chọn runtime theo
Device/Template trước khi implementation.

Tài liệu lưu đánh giá ban đầu đối chiếu **Postman collection đang làm việc** với router thật của branch `dev-test` tại commit `4a2a5f932bff181de0cf3e8da7686ddbdcf6bad7`. Collection hiện nằm ở `docs/api/` nhưng chưa được Git track, vì vậy không nên gọi nó là artifact đã thuộc commit/branch. Admin API chỉ mount khi `database.enabled=true && api.enabled=true`, dùng Bearer admin token và PATCH/PUT mutation dùng optimistic concurrency qua `If-Match`.

## Cập nhật thực thi P0

Đã triển khai, có router-seam regression test và không làm thay đổi `RuntimeCatalog` hoặc Voice Session đang admit:

- `GET /api/admin/agents/{agent_key}/templates` và `GET /api/admin/templates/{template_key}/agents`.
- `GET /api/admin/templates/{template_key}/providers` và `GET /api/admin/providers/{provider_key}/templates`.
- `DELETE /api/admin/agents/{agent_key}/templates/{template_key}`: soft-unlink, yêu cầu `If-Match`; có thể gỡ cả default Template, để Agent không còn assignment và dùng server defaults.
- `DELETE /api/admin/templates/{template_key}/providers/{provider_type}`: unlink transactionally, yêu cầu `If-Match`.

Mọi GET relationship trả revision của owner; inverse usage có pagination và `total` được đếm trước `LIMIT/OFFSET`. Test bao phủ `If-Match` thiếu/stale, revision conflict và relationship không đổi sau conflict.

## Cập nhật thực thi P1

Đã triển khai với desired-state semantics; thay đổi không materialize Provider và không làm thay
đổi Voice Session đã admit:

- `Device.template_key` là nullable override. Create/get/list/patch đều hiển thị field này; chỉ
  nhận Template enabled có assignment enabled tới Agent enabled của Device. `null` quay về
  default Template của Agent. Admission chọn override trong profile snapshot và fail-closed nếu
  graph không còn hợp lệ.
- `GET /api/admin/system` trả version, uptime monotonic, DB reachability, số Provider theo trạng
  thái runtime và số session active; không trả path, config hay secret.
- `GET /api/admin/providers` hỗ trợ `q`, `type`, `enabled`, `page`, `page_size`, `sort`; response
  giữ tương thích cũ và thêm `total`, `total_pages`, `facets` (`vad`, `asr`, `llm`, `tts`).
- `GET /api/admin/templates` hỗ trợ `q`, `language`, `enabled`, `page`, `page_size`, `sort`; thêm
  `total` và `total_pages`. `q` khớp key/name/description, còn language là exact filter.

Vision Provider chưa được đánh dấu hoàn thành: contract hiện chưa quy định ProviderType, adapter,
validation DB, materialization runtime và Template binding cho Vision.

## Cập nhật thực thi P2

- `POST /api/admin/providers/{key}/test/vad` không nhận body; chạy đúng một frame silence 512
  samples ở 16 kHz trên VAD runtime đã load khi startup. Response trả probability/range, elapsed
  time và runtime provenance. Diagnostic chỉ dùng capacity không reserved cho Voice, không đọc
  secret, không materialize Provider và không thay đổi capture cycle/Voice Session.
- `POST /api/admin/providers/{key}/test/vision` vẫn deferred: Vision chưa là Database Provider
  Instance nên route này chưa có contract đúng để triển khai.

## Đánh giá ban đầu

Với UI Agents → Templates → Providers mà ta vừa thiết kế, các API còn thiếu đáng chú ý là:

| Ưu tiên | API | Trạng thái / mục đích |
|---|---|---|
| **P0 hoàn thành** | `GET /api/admin/agents/{agent_key}/templates` | Lấy danh sách template đã link với Agent |
| **P0 hoàn thành** | `DELETE /api/admin/agents/{agent_key}/templates/{template_key}` | Soft-unlink Template khỏi Agent |
| **P0 hoàn thành** | `GET /api/admin/templates/{template_key}/providers` | Lấy Provider Binding của Template để dựng AI Pipeline |
| **P0 hoàn thành** | `DELETE /api/admin/templates/{template_key}/providers/{provider_type}` | Unlink Provider khỏi Template |
| **P0 hoàn thành** | `GET /api/admin/templates/{template_key}/agents` | Biết những Agent nào đang dùng Template |
| **P0 hoàn thành** | `GET /api/admin/providers/{provider_key}/templates` | Hiển thị `Used by N templates` trên Provider Catalog |
| **P3 hoàn thành** | DELETE Agent/Device/Template/Provider/MCP Server | Conditional hard-delete, `If-Match`, `409 *_in_use`, explicit unlink và không cascade/purge active relationship/history (ADR-0070) |
| **P1 hoàn thành** | Device Template Override API | Device dùng Template khác default của Agent, snapshot ở admission |
| **P1 hoàn thành** | `GET /api/admin/system` | System page: version, DB, runtime, uptime, provider state |
| **P1 hoàn thành** | Provider list filter/search | Search/type/enabled filter server-side, total/facets |
| **P1 hoàn thành** | Template list filter/search | Search key/name/description, language/enabled filter, total |
| **P1 hoàn thành** | Pagination totals/facets | `total`, `total_pages`, counts theo Provider type |
| **Cần chốt selection seam** | Vision Provider APIs | Vision sẽ là DB Provider/Template binding, nhưng Vision HTTP hiện chưa chọn runtime theo Device/Template |
| **P2 hoàn thành** | `POST /api/admin/providers/{key}/test/vad` | Test one canonical silence frame từ Provider Detail |
| **P2** | `POST /api/admin/providers/{key}/test/vision` | Test Vision nếu Vision trở thành Provider Instance |
| **P3 hoàn thành** | DELETE MCP Server / unlink MCP binding | Conditional delete và explicit unlink, cùng revision của Agent (ADR-0070) |

### 1. Thiếu API đọc Template bindings — đây là blocker lớn nhất

Server hiện có:

```http
PUT /api/admin/templates/{template_key}/providers/{provider_type}
```

với body:

```json
{
  "provider_key": "..."
}
```

nhưng **không có GET tương ứng để lấy provider bindings của Template**. API bind hiện hỗ trợ `vad|asr|llm|tts` và tăng Template revision.

Trong khi `GET /api/admin/templates/{template_key}` hiện chỉ trả các field Template cơ bản như:

```text
id
key
name
description
language
prompt
enabled
revision
created_at
updated_at
```

Nó không include Provider Bindings.

Vì vậy Agent Detail mới:

```text
Selected Template
       ↓
VAD → ASR → LLM → TTS
```

không thể dựng AI Pipeline chỉ với API hiện tại.

Tôi đề xuất:

```http
GET /api/admin/templates/{template_key}/providers
```

Response:

```json
{
  "template_key": "assistant_vi",
  "bindings": {
    "vad": {
      "provider_key": "vad_silero"
    },
    "asr": {
      "provider_key": "asr_zipformer"
    },
    "llm": {
      "provider_key": "llm_openai"
    },
    "tts": {
      "provider_key": "tts_zerotts"
    }
  }
}
```

Hoặc tốt hơn, `GET template` có thể trả expanded bindings luôn:

```json
{
  "key": "assistant_vi",
  "name": "Vietnamese Assistant",
  "language": "vi-VN",
  "prompt": "...",
  "revision": 4,

  "provider_bindings": {
    "vad": {...},
    "asr": {...},
    "llm": {...},
    "tts": {...}
  }
}
```

Hai hướng đều hợp lệ, nhưng chưa nên chốt bằng nhu cầu “một request”. Nên lấy relationship endpoint làm hợp đồng canonical trước; nếu đo được round-trip là vấn đề, có thể thêm `GET /templates/{key}?include=provider_bindings` mà vẫn giữ response cơ bản nhỏ và cache/revision rõ ràng.

---

### 2. Có link Template vào Agent nhưng không có unlink

Hiện đã có:

```http
PUT /api/admin/agents/{agent_key}/templates/{template_key}
```

và:

```http
PUT /api/admin/agents/{agent_key}/default-template/{template_key}
```

cả hai đều dùng Agent revision.

Nhưng thiếu:

```http
DELETE /api/admin/agents/{agent_key}/templates/{template_key}
```

Nên UI action:

```text
Unlink template from agent
```

chưa thể thực hiện bằng API thật.

Tôi đề xuất:

```http
DELETE /api/admin/agents/{agent_key}/templates/{template_key}
If-Match: "<agent_revision>"
```

Template default cũng có thể được unlink. Khi đây là assignment cuối cùng, Agent không còn
Template và Voice Session mới dùng server defaults.

---

### 3. Thiếu API list Templates của một Agent

`GET /api/admin/agents/{key}` hiện không trả linked templates.

Do đó Template Switcher:

```text
Template
[ Vietnamese Home ▾ ]
```

không có API trực tiếp để biết Agent được assign những Template nào.

Nên thêm:

```http
GET /api/admin/agents/{agent_key}/templates
```

Response có thể là:

```json
{
  "items": [
    {
      "key": "assistant_vi",
      "name": "Vietnamese Home",
      "language": "vi-VN",
      "is_default": true
    },
    {
      "key": "assistant_en",
      "name": "English Home",
      "language": "en-US",
      "is_default": false
    }
  ]
}
```

API này rất quan trọng cho Agent Detail.

---

### 4. Templates Page thiếu usage API

Templates Page cần:

```text
Vietnamese Home

Used by 3 agents
Home Assistant
Kitchen Assistant
Study Assistant
```

Nhưng hiện chưa có:

```http
GET /api/admin/templates/{template_key}/agents
```

Nên thêm:

```http
GET /api/admin/templates/{template_key}/agents
```

Response:

```json
{
  "items": [
    {
      "key": "home",
      "name": "Home Assistant",
      "is_default": true
    },
    {
      "key": "study",
      "name": "Study Assistant",
      "is_default": false
    }
  ],
  "total": 2
}
```

Đây cũng giúp Delete Template biết blast radius.

---

### 5. Provider Catalog thiếu Template usage

Provider page mới cần:

```text
OpenAI Primary

Used by 4 templates
Vietnamese Home · English Home · +2
```

Nhưng API provider hiện có chỉ là list/create/get/patch/capabilities.

Thiếu:

```http
GET /api/admin/providers/{provider_key}/templates
```

Tôi đề xuất:

```json
{
  "provider_key": "llm_openai",
  "total": 3,
  "items": [
    {
      "key": "assistant_vi",
      "name": "Vietnamese Home"
    },
    {
      "key": "assistant_en",
      "name": "English Home"
    }
  ]
}
```

Hoặc nếu muốn Provider Catalog tránh N+1 request, `GET /providers` nên trả luôn:

```json
{
  "key": "llm_openai",
  "name": "OpenAI Primary",
  "type": "llm",

  "usage": {
    "template_count": 3,
    "templates": [
      {
        "key": "assistant_vi",
        "name": "Vietnamese Home"
      }
    ]
  }
}
```

Cách thứ hai tốt hơn cho Provider Catalog.

---

### 6. Chưa có DELETE resource, nhưng phải chốt policy trước

Router hiện tại chỉ có:

```text
Agent       GET POST PATCH
Device      GET POST PATCH
Template    GET POST PATCH
Provider    GET POST PATCH
MCP Server  GET POST PATCH
```

Không có DELETE route.

UI sẽ cần thao tác xóa hoặc archive, nhưng không nên coi bốn route dưới đây là P0 CRUD độc lập:

```http
DELETE /api/admin/agents/{key}
DELETE /api/admin/devices/{device_id}
DELETE /api/admin/templates/{key}
DELETE /api/admin/providers/{key}
```

Các foreign key hiện hành buộc phải xác định semantics trước:

```text
Provider đang được Template bind  →  ON DELETE RESTRICT
Agent đang có Device hoặc history  →  ON DELETE RESTRICT
Device bị xóa                      →  history của Device ON DELETE CASCADE
Template bị xóa                    →  assignment/binding CASCADE, history.template_id SET NULL
```

Quyết định đã chốt tại ADR-0070: DELETE là conditional hard-delete có `If-Match`.
Resource còn active relationship hoặc history reference trả `409 *_in_use`; người dùng unlink
tường minh hoặc purge history scoped trước. Ngoại lệ là Agent ↔ Template: xóa Agent được phép,
vì FK cascade chỉ dọn assignment của Agent và không ảnh hưởng Template global. Không có cascade
unlink cho các relationship active khác, revision bump ngầm hay history purge ngầm.

Collection phải được cập nhật sau khi P3 hoàn thành để expose DELETE và MCP unlink.

---

### 7. Thiếu unlink Provider khỏi Template

Hiện có bind:

```http
PUT /api/admin/templates/{template_key}/providers/{provider_type}
```

nhưng không có:

```http
DELETE /api/admin/templates/{template_key}/providers/{provider_type}
```

Đây là API cần cho menu:

```text
View provider
Edit provider
Unlink from template
```

Nên dùng:

```http
DELETE /api/admin/templates/{template_key}/providers/{provider_type}
If-Match: "<template_revision>"
```

Không cần gửi `provider_key`, vì `(template, provider_type)` đã xác định binding.

---

### 8. Device chưa có Template override

Device API hiện chỉ hỗ trợ:

```text
device_id
agent_key
name
description
metadata_json
enabled
```

Create/Patch Device không có `template_key`.

Trong UI ta đã thiết kế:

```text
Living Room
Template: Vietnamese Home · Default

Bedroom
Template: Kids Assistant · Override
```

Server hiện chưa hỗ trợ domain này.

Có thể mở rộng PATCH Device:

```http
PATCH /api/admin/devices/{device_id}
If-Match: "<revision>"
```

Body:

```json
{
  "template_key": "kids_assistant"
}
```

Clear override:

```json
{
  "template_key": null
}
```

Semantics:

```text
template_key == null
        ↓
use Agent default template
```

Backend cũng cần migration thêm cột/foreign key, validate Template đã được assigned cho Agent, và mở rộng admission để chọn template override khi tạo `EffectiveSessionProfile`. Đây không phải chỉ là field PATCH; session đã admit vẫn giữ profile immutable, còn thay đổi runtime-affecting chỉ effective theo lifecycle/restart đã công bố.

---

### 9. Vision đang là một gap domain/API

Đây là điểm quan trọng.

UI trước đó có:

```text
VAD
ASR
LLM
TTS
Vision
```

nhưng DB Provider API hiện chỉ chấp nhận:

```text
vad | asr | llm | tts
```

Collection và schema hiện chỉ hỗ trợ 4 type đó. Template Provider Binding cũng chỉ chấp nhận `vad|asr|llm|tts`.

Do đó nếu muốn Vision trở thành provider giống UI đang thiết kế, cần mở rộng backend:

```text
ProviderType += vision
Provider Adapter Catalog += vision adapters
Template Provider Binding += vision
Database validation += vision
Runtime materialization += vision
```

Sau đó API:

```http
PUT /api/admin/templates/{key}/providers/vision
```

mới hợp lệ.

Hiện tại không nên để Web gửi nó.

---

### 10. Provider test vẫn thiếu VAD/Vision

Server hiện có:

```http
POST /api/admin/providers/{key}/test/llm
POST /api/admin/providers/{key}/test/tts
POST /api/admin/providers/{key}/test/asr
POST /api/admin/providers/{key}/test/vad
```

TTS trả WAV và ASR nhận WAV mono 16 kHz ≤30s.

VAD không nhận caller PCM: một request rỗng chạy một silence frame canonical 512 samples/16 kHz
trên runtime đã load, trả probability/range và provenance. Do đó đây là readiness/inference probe
bounded, không phải endpoint VAD segmentation cho audio người dùng.

Nếu Provider Detail muốn `Test` cho mọi card, còn thiếu:

```http
POST /api/admin/providers/{key}/test/vad
POST /api/admin/providers/{key}/test/vision
```

Tuy nhiên `test/vision` chỉ nên thêm sau khi Vision chính thức trở thành Provider Instance.

---

### 11. Provider search/type filter chưa có server-side

Hiện:

```http
GET /api/admin/providers?page=1&page_size=50
```

chỉ hỗ trợ pagination.

Provider Catalog mới cần:

```text
Search
All / VAD / ASR / LLM / TTS / Vision
Status
```

Nên mở rộng:

```http
GET /api/admin/providers
  ?page=1
  &page_size=50
  &type=tts
  &enabled=true
  &q=zero
  &sort=name
```

Ở quy mô nhỏ frontend có thể filter client-side, nhưng về API design nên có.

---

### 12. List API chưa trả total

Các API list hiện thường trả:

```json
{
  "items": [],
  "page": 1,
  "page_size": 50,
  "max_page_size": 200
}
```

nhưng không có:

```json
{
  "total": 128,
  "total_pages": 3
}
```

Điều này gây khó cho:

```text
All 8
VAD 1
ASR 2
LLM 1
TTS 3
```

và pagination UI.

Tôi khuyên chuẩn hóa:

```json
{
  "items": [],
  "pagination": {
    "page": 1,
    "page_size": 50,
    "total": 128,
    "total_pages": 3
  }
}
```

Với Providers có thể thêm:

```json
{
  "facets": {
    "vad": 1,
    "asr": 2,
    "llm": 1,
    "tts": 3
  }
}
```

---

### 13. System page mới chỉ có health/readiness cơ bản

Hiện có:

```http
GET /health
GET /ready
```

`/health` chỉ phản ánh process liveness, còn `/ready` cho readiness nhận voice connection.

System page mà ta muốn:

```text
Server
● Running

Version
Uptime
Database
Provider runtime
Loaded providers
Active sessions
```

cần endpoint mới, ví dụ:

```http
GET /api/admin/system
```

Response:

```json
{
  "status": "ready",
  "version": "0.1.0",
  "uptime_seconds": 58302,

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
    "active": 2
  }
}
```

Không cần trả filesystem path hoặc secret.

---

## Thứ tự bổ sung backend

Nếu mục tiêu là đưa Web Admin ra khỏi mock/localStorage, trạng thái ưu tiên hiện tại là:

1. **Template relationships — hoàn thành**
   - GET Agent Templates
   - DELETE Agent Template link
   - GET Template Provider Bindings
   - DELETE Template Provider Binding

2. **Usage — hoàn thành**
   - GET Agents using Template
   - GET Templates using Provider / hoặc expand usage vào Provider list

3. **Conditional deletion — hoàn thành**
   - `409 *_in_use`, `If-Match`, audit cùng transaction và explicit unlink theo ADR-0070.
   - MCP unlink bump Agent revision; history chỉ purge qua endpoint scoped.

4. **Device Template Override**
   - Migration + admission/session-profile, không chỉ thêm field PATCH.

5. **System Status**

6. **Search/filter/count**

7. **Vision DB Provider integration**

8. **VAD diagnostic — hoàn thành; Vision diagnostic cần Vision Provider contract**

P0 read-model đã hoàn thành: phần **Agents + Template Switcher + AI Pipeline + Templates Page + Provider Catalog** có thể hoạt động bằng server API mà không cần frontend tự giữ relational state. Đây là desired configuration: UI phải hiển thị `requires_restart`/runtime status khi mutation chưa effective, và không được hứa hot-reload session đang chạy.

Bản Postman đang làm việc đã có nền tảng create/update và P0 relationship đầy đủ cho Agent/Device, Template và Provider; phần còn lại chủ yếu là **Vision Database Provider integration**. Postman collection cần được đồng bộ riêng theo P3 deletion contract.
