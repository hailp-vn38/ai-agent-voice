# Provider Server-Generated Key Refactor Guide

## 1. Mục tiêu

Thay đổi flow tạo Provider để **server tự sinh `providers.key`**.

Client/Web/Admin API **không được phép tự truyền `key` khi tạo Provider**.

Mục tiêu cuối cùng:

```text
Client/Web
   |
   | POST /api/admin/providers
   | { name, type, adapter, config_json, secret_ref }
   v
Server
   |
   |- validate request
   |- generate immutable provider key
   |- INSERT providers
   |- audit
   `- return 201 Created + generated key
```

`providers.key` vẫn tiếp tục là stable public/internal identifier dùng cho:

- `GET /api/admin/providers/{key}`
- `PATCH /api/admin/providers/{key}`
- `DELETE /api/admin/providers/{key}`
- `/prepare`
- `/test/*`
- `/capabilities`
- Template Provider Binding
- runtime lookup
- diagnostics
- audit/debug metadata

Thay đổi này chỉ chuyển trách nhiệm sinh key từ **client -> server**.

---

## 2. Hiện trạng cần thay đổi

Trong branch `dev-test`, `CreateProvider` hiện yêu cầu:

```rust
#[derive(Deserialize)]
struct CreateProvider {
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    secret_ref: Option<String>,
}
```

`create_provider()` đang:

1. nhận `body.key` từ client;
2. validate bằng `valid_key(&body.key)`;
3. bind trực tiếp `body.key` vào `INSERT INTO providers(...)`;
4. query lại provider bằng `provider_by(pool, &body.key)`.

Postman và tài liệu flow hiện tại cũng đang hướng dẫn user nhập `key`.

Đây không còn là contract mong muốn.

---

## 3. Thiết kế mới

### 3.1. Key do server sở hữu

Provider key phải được xem là **server-owned immutable identifier**.

Client chỉ gửi các dữ liệu nghiệp vụ:

```json
{
  "name": "OpenAI Primary",
  "type": "llm",
  "adapter": "openai",
  "config_json": {
    "base_url": "https://api.openai.com/v1",
    "model": "model-name",
    "timeout_ms": 30000,
    "max_tokens": 1024
  },
  "secret_ref": "OPENAI_API_KEY"
}
```

Client không gửi:

```json
{
  "key": "llm_openai"
}
```

### 3.2. Format key

Dùng format:

```text
{provider_type}_{uuid32}
```

Ví dụ:

```text
llm_c20963fea89d401e989de9f7a6851463
tts_39c9f32465cc4e7db79e8634af66c408
asr_0fc22e1c46fc43b5aa64a0c29521db73
vad_c2b12cc837054a2c8490016363538505
```

Không tạo key từ `name`.

Lý do:

- tránh collision do tên trùng;
- không phụ thuộc Unicode/slugification;
- rename Provider không làm thay đổi identity;
- không cần retry phức tạp theo suffix;
- phù hợp constraint `valid_key()` hiện tại;
- key vẫn dễ nhận biết loại Provider khi debug.

### 3.3. Hàm sinh key

Có thể đặt helper gần `create_provider()` hoặc trong module admin/provider helper:

```rust
fn generate_provider_key(kind: &str) -> String {
    format!("{}_{}", kind, Uuid::new_v4().simple())
}
```

Project đã có `uuid::Uuid` trong Admin module nên không cần thêm dependency mới.

---

## 4. Thay đổi backend

### 4.1. File chính

Sửa:

```text
crates/voice-agent-server/src/app/admin/providers.rs
```

### 4.2. Đổi `CreateProvider`

Từ:

```rust
#[derive(Deserialize)]
struct CreateProvider {
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    secret_ref: Option<String>,
}
```

Thành:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProvider {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    secret_ref: Option<String>,
}
```

### 4.3. Bắt buộc `deny_unknown_fields`

`CreateProvider` phải dùng:

```rust
#[serde(deny_unknown_fields)]
```

Mục đích:

- client cũ gửi `key` phải bị reject;
- không âm thầm bỏ qua input không còn hợp lệ;
- contract API rõ ràng;
- tránh user tưởng key họ gửi đã được dùng.

Ví dụ request cũ:

```json
{
  "key": "custom_key",
  "name": "OpenAI",
  "type": "llm",
  "adapter": "openai",
  "config_json": {}
}
```

phải trả `400 Bad Request` theo error contract JSON parsing hiện tại.

Không thêm compatibility mode cho `key`.

---

## 5. Update `create_provider()`

### 5.1. Validation

Xóa validation:

```rust
!valid_key(&body.key)
```

Validation còn lại phải giữ nguyên:

- `name`
- `type`
- adapter/type compatibility
- `secret_ref`
- typed provider config validation

Ví dụ:

```rust
if !valid_text(&body.name, 128, false)
    || !matches!(body.kind.as_str(), "vad" | "asr" | "llm" | "tts")
    || !adapter_matches_kind(&body.kind, &body.adapter)
    || body
        .secret_ref
        .as_ref()
        .is_some_and(|value| !valid_secret_ref(value))
{
    return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
}
```

### 5.2. Sinh key sau khi type đã hợp lệ

Sau validation `kind`, tạo:

```rust
let provider_key = generate_provider_key(&body.kind);
```

Không generate key trước khi validate `type`.

### 5.3. INSERT

Đổi:

```rust
.bind(&body.key)
```

thành:

```rust
.bind(&provider_key)
```

Ví dụ:

```rust
let result = sqlx::query(
    "INSERT INTO providers(key,name,type,adapter,config_json,secret_ref,created_at,updated_at) \
     VALUES(?,?,?,?,?,?,?,?)",
)
.bind(&provider_key)
.bind(&body.name)
.bind(&body.kind)
.bind(&body.adapter)
.bind(config)
.bind(&body.secret_ref)
.bind(now())
.bind(now())
.execute(&mut *tx)
.await;
```

### 5.4. Query response sau insert

Đổi:

```rust
provider_by(pool, &body.key).await
```

thành:

```rust
provider_by(pool, &provider_key).await
```

Response `201 Created` vẫn trả đầy đủ Provider, bao gồm generated key.

---

## 6. Collision handling

UUID v4 collision gần như không đáng kể, nhưng DB vẫn có:

```sql
key TEXT NOT NULL UNIQUE
```

Không bỏ UNIQUE constraint.

Có hai lựa chọn triển khai:

### Phương án khuyến nghị cho V1

Generate một lần rồi insert.

Nếu DB trả unique conflict, dùng error path hiện tại:

```text
409 resource_conflict
```

Không cần retry loop vì xác suất collision thực tế cực thấp.

### Không nên

- generate key từ timestamp;
- generate từ auto-increment ID rồi update ngược lại;
- query trước để kiểm tra key tồn tại;
- dùng random ngắn 6-8 ký tự;
- dùng provider name làm identity.

---

## 7. Database

### 7.1. Không cần migration

Schema hiện tại phù hợp:

```sql
CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    ...
);
```

Không thay đổi:

- column `key`;
- UNIQUE constraint;
- foreign key relationship;
- provider ID;
- revision semantics.

### 7.2. Provider cũ

Provider đã tồn tại giữ nguyên key cũ.

Không migrate key cũ sang UUID format.

Lý do:

- key đang được dùng trong URL/API/runtime/reference;
- đổi key cũ tạo breaking change không cần thiết;
- mục tiêu chỉ áp dụng rule server-generated cho Provider mới.

---

## 8. PATCH Provider

Endpoint:

```http
PATCH /api/admin/providers/{key}
```

Key tiếp tục immutable.

Hiện `PatchProvider` có field:

```rust
#[serde(default)]
key: Patch<String>,
```

và reject:

```rust
if !matches!(body.key, Patch::Absent) {
    return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
}
```

Có thể giữ nguyên behavior này để backward defensive validation.

Khuyến nghị tốt hơn nếu API muốn chặt hơn:

- thêm `#[serde(deny_unknown_fields)]` vào `PatchProvider` nếu tương thích với toàn bộ PATCH contract;
- sau đó có thể cân nhắc bỏ field `key` khỏi `PatchProvider` để request chứa key bị invalid JSON shape.

Tuy nhiên thay đổi này không bắt buộc trong scope hiện tại.

Scope bắt buộc:

> POST không nhận key; PATCH không cho đổi key.

---

## 9. Không refactor runtime

Không thay đổi các component runtime chỉ vì key được server generate.

Các API/module dưới đây tiếp tục dùng `provider.key` bình thường:

```text
/providers/{key}
/providers/{key}/prepare
/providers/{key}/test/llm
/providers/{key}/test/tts
/providers/{key}/test/asr
/providers/{key}/test/vad
/providers/{key}/capabilities
/providers/{key}/templates
```

Không đổi:

- `ProviderRuntimeManager`
- `DatabaseRuntimeSnapshot`
- runtime registry
- `DesiredProvider`
- diagnostics
- runtime leases
- template provider binding database relation
- prewarm flow

Key vẫn là `String`; chỉ nguồn tạo thay đổi.

---

## 10. Template Provider Binding

API hiện tại:

```http
PUT /api/admin/templates/{template_key}/providers/{provider_type}
```

Body:

```json
{
  "provider_key": "llm_c20963fea89d401e989de9f7a6851463"
}
```

Không thay đổi contract này.

Web phải lấy generated key từ response tạo Provider hoặc từ Provider list/detail.

Không cho user tự nhập provider key bằng text box.

Provider selector phải sử dụng danh sách Provider từ server.

---

## 11. API contract mới

### 11.1. Create Provider

```http
POST /api/admin/providers
Authorization: Bearer <admin_token>
Content-Type: application/json
```

Request:

```json
{
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  "config_json": {
    "base_url": "https://api.openai.com/v1",
    "model": "model-name",
    "timeout_ms": 30000,
    "max_tokens": 1024
  },
  "secret_ref": "OPENAI_API_KEY"
}
```

Response example:

```json
{
  "id": 12,
  "key": "llm_c20963fea89d401e989de9f7a6851463",
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  "config_json": "{...}",
  "enabled": 1,
  "revision": 1,
  "has_secret_ref": true,
  "runtime_status": "not_loaded",
  "runtime_matches_desired": false,
  "requires_restart": false
}
```

Response shape thực tế phải tiếp tục theo `managed_provider_response()` hiện tại; ví dụ trên chỉ minh họa generated key.

### 11.2. Request chứa `key`

Request:

```json
{
  "key": "custom_llm",
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  "config_json": {}
}
```

Expected:

```text
400 Bad Request
```

Do `CreateProvider` dùng `deny_unknown_fields`.

---

## 12. Update tests

Phải bổ sung/điều chỉnh tests cho Admin Provider API.

### 12.1. Create không cần key

Test request không có `key` vẫn trả:

```text
201 Created
```

Assert:

```text
response.key starts_with "llm_"
response.key matches valid_key rules
response.key length <= 64
```

### 12.2. Different provider gets different key

Tạo hai Provider cùng type/name hợp lệ.

Assert:

```text
provider_a.key != provider_b.key
```

Nếu name duplicate được DB/API cho phép thì giữ đúng semantics hiện tại; mục tiêu test là key không phụ thuộc name.

### 12.3. Client-supplied key bị reject

Gửi request có:

```json
"key": "user_controlled_key"
```

Assert:

```text
400 Bad Request
```

### 12.4. Generated key dùng được ngay

Sau `POST`, lấy `response.key` rồi gọi:

```http
GET /api/admin/providers/{generated_key}
```

Assert:

```text
200 OK
response.key == generated_key
```

### 12.5. Template binding dùng generated key

Tạo Provider -> lấy key -> bind vào Template.

Assert binding thành công và DB `provider_id` trỏ đúng row.

### 12.6. PATCH không đổi key

Giữ test hiện tại hoặc bổ sung:

```json
{
  "key": "new_key"
}
```

Expected:

```text
400 immutable_field
```

### 12.7. Regression runtime

Không cần test model thật chỉ cho thay đổi key.

Nhưng phải bảo đảm ít nhất compile/test các path:

- provider lookup by generated key;
- prepare route;
- diagnostics route lookup;
- template provider binding lookup.

---

## 13. Update `docs/api/00-all-apis.postman_collection.json`

File bắt buộc update:

```text
docs/api/00-all-apis.postman_collection.json
```

### 13.1. Create Provider request

Hiện đang có:

```json
{
  "key": "{{provider_key}}",
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  ...
}
```

Phải đổi thành:

```json
{
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  "config_json": {
    "base_url": "https://api.openai.com/v1",
    "model": "model-name",
    "timeout_ms": 30000,
    "max_tokens": 1024
  },
  "secret_ref": "OPENAI_API_KEY"
}
```

### 13.2. Update description

Description của Create Provider phải nói rõ:

```text
Provider key is generated by the server and returned in the 201 response.
Clients must not send key in the create request.
The returned key is immutable and is used by provider detail, prepare,
test, capabilities and template binding endpoints.
```

Có thể viết tiếng Anh để đồng bộ style Postman hiện tại.

### 13.3. `{{provider_key}}` vẫn giữ trong collection variables

Không xóa biến:

```text
{{provider_key}}
```

vì vẫn được dùng cho:

- get provider;
- patch provider;
- delete provider;
- prepare;
- test;
- capabilities;
- templates usage;
- template binding body.

Nhưng variable này không còn là input để create.

User/Postman phải copy generated `key` từ response create vào `{{provider_key}}` khi test thủ công.

### 13.4. Có thể thêm Postman test script

Khuyến nghị thêm test script cho request Create Provider để tự lưu generated key:

```javascript
const body = pm.response.json();
if (body && body.key) {
  pm.collectionVariables.set("provider_key", body.key);
}
```

Nếu collection đang dùng environment variable thay vì collection variable thì dùng đúng scope hiện tại.

Không tạo thêm hai biến cùng tên ở nhiều scope nếu không cần.

### 13.5. Update PATCH description

Giữ/nhấn mạnh:

```text
key is immutable and server-generated.
```

### 13.6. Kiểm tra toàn bộ file Postman

Search toàn file:

```text
"key": "{{provider_key}}"
```

Phân biệt:

- Create Provider -> phải xóa.
- Binding body `provider_key` -> giữ.
- URL `/providers/{{provider_key}}` -> giữ.

Không thực hiện global replace mù.

---

## 14. Update `docs/flow.md`

File hiện tại còn hướng dẫn UI nhập `key`.

Phải update.

### 14.1. Create form

Xóa trường:

```text
Key
```

Khối `Thông tin provider` chỉ còn tối thiểu:

```text
Tên hiển thị
```

`type` và `adapter` được chọn ở bước trước.

### 14.2. Xóa hướng dẫn generate key phía web

Xóa các nội dung dạng:

```text
Web có thể gợi ý key từ tên, nhưng cho người dùng sửa.
Validate key theo quy tắc...
```

Thay bằng:

```text
Provider key do server tự sinh khi tạo.
Web không hiển thị input key và không gửi key trong POST request.
```

### 14.3. Review step

Không hiển thị key trước khi Provider được tạo.

Có thể hiển thị:

```text
Key: Server tự tạo
```

hoặc bỏ hoàn toàn dòng key ở review create.

### 14.4. Sau create

Sau `201 Created`:

- nhận `response.key`;
- dùng key để route sang Provider detail;
- key có thể hiển thị read-only;
- có nút copy nếu UI muốn;
- không có edit key.

---

## 15. Web API contract

Nếu web repo/types nằm cùng project hoặc được update bởi agent khác, update các type liên quan.

Từ:

```ts
interface CreateProviderRequest {
  key: string
  name: string
  type: ProviderType
  adapter: string
  config_json: unknown
  secret_ref?: string
}
```

Thành:

```ts
interface CreateProviderRequest {
  name: string
  type: ProviderType
  adapter: string
  config_json: unknown
  secret_ref?: string
}
```

Response Provider vẫn có:

```ts
interface Provider {
  id: number
  key: string
  // ...
}
```

Không xóa `key` khỏi Provider response type.

---

## 16. UI behavior

### Create Provider

Không có field:

```text
Provider key
```

User chỉ nhập/chọn:

- name;
- type;
- adapter;
- adapter config;
- credential reference nếu cần.

### Provider list/detail

Vẫn có thể hiển thị:

```text
llm_c20963fea89d401e989de9f7a6851463
```

nhưng dưới dạng metadata kỹ thuật:

- read-only;
- copyable;
- không editable.

Tên Provider vẫn là thông tin chính trên UI.

---

## 17. Backward compatibility

Đây là intentional API breaking change đối với client đang gửi Provider key.

Không giữ compatibility bằng:

```rust
#[serde(default)]
key: Option<String>
```

và bỏ qua value.

Không làm như vậy vì:

- client tưởng key của họ được sử dụng;
- tạo contract mơ hồ;
- kéo dài behavior cũ không cần thiết.

Client cũ phải update.

Provider đã tồn tại không bị ảnh hưởng.

---

## 18. Error semantics

Giữ error contract hiện có.

Không cần thêm error code mới chỉ cho key generation.

Expected cases:

| Case | Expected |
|---|---|
| invalid type | `400 validation_failed` |
| invalid adapter/type | `400 validation_failed` hoặc contract hiện tại tương ứng |
| invalid provider config | `400 provider_config_invalid` |
| client gửi unknown `key` | `400 invalid_json` theo helper hiện tại |
| UUID unique conflict cực hiếm | `409 resource_conflict` |
| DB unavailable | error hiện tại |

Không echo generated key trong error telemetry nếu insert chưa commit.

---

## 19. Audit

Audit behavior không thay đổi.

Sau insert thành công:

```text
resource_type = provider
resource_id   = provider_id
action        = create
new_revision  = 1
```

Không cần lưu key riêng trong audit row.

Provider key có thể lookup từ provider ID khi cần.

---

## 20. Logging / security

Generated Provider key không phải secret.

Có thể log bounded metadata nếu project đang log provider identity.

Không thay đổi secret handling:

```text
secret_ref -> SecretResolver -> SecretValue
```

`config_json` tiếp tục credential-free.

Không đưa API key/token vào generated key.

---

## 21. Files cần kiểm tra/update

Backend bắt buộc:

```text
crates/voice-agent-server/src/app/admin/providers.rs
```

Tests liên quan Admin Provider API:

```text
crates/voice-agent-server/src/app/admin/*
```

Tìm các test/request fixture có Create Provider body chứa:

```text
"key"
```

Docs bắt buộc:

```text
docs/api/00-all-apis.postman_collection.json
docs/flow.md
```

Tìm thêm toàn repo:

```text
CreateProvider
POST /api/admin/providers
"key": "{{provider_key}}"
tts_maichi
llm_openai
```

Chỉ sửa các ví dụ đang mô tả **create provider**.

Không sửa các path sử dụng key sau khi Provider đã tồn tại.

---

## 22. Trình tự triển khai đề xuất

### Bước 1 - Backend contract

- bỏ `key` khỏi `CreateProvider`;
- thêm `deny_unknown_fields`;
- thêm `generate_provider_key()`;
- generate key trong server;
- insert bằng generated key;
- trả Provider bằng generated key.

### Bước 2 - Tests

- update fixture cũ;
- test create không cần key;
- test client gửi key bị reject;
- test generated key có prefix đúng;
- test generated key dùng được cho GET/binding.

### Bước 3 - Postman

Update:

```text
docs/api/00-all-apis.postman_collection.json
```

- remove key khỏi create body;
- update description;
- giữ `provider_key` variable cho các endpoint sau create;
- nếu phù hợp, auto-save response key bằng Postman script.

### Bước 4 - Docs flow

Update:

```text
docs/flow.md
```

- bỏ input Key;
- bỏ key suggestion/validation ở web;
- ghi rõ server-generated;
- Provider detail chỉ hiển thị key read-only.

### Bước 5 - Web

Nếu web nằm trong cùng scope triển khai:

- remove key input;
- remove slug generation;
- remove client-side key validation;
- remove `key` khỏi CreateProviderRequest;
- dùng `response.key` sau create.

### Bước 6 - Full regression

Chạy formatting, lint và tests theo workflow project hiện tại.

Tối thiểu phải verify:

```text
cargo fmt --check
cargo test
```

Nếu workspace có lint gate:

```text
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Chỉ chạy command phù hợp với CI hiện tại của repo; không làm thay đổi ngoài scope chỉ để pass unrelated failure.

---

## 23. Acceptance criteria

Implementation hoàn thành khi tất cả điều kiện sau đúng:

- [ ] `POST /api/admin/providers` không còn field `key` trong request contract.
- [ ] Client gửi `key` trong create request bị reject.
- [ ] Server tự sinh key format `{type}_{uuid32}`.
- [ ] Generated key đáp ứng constraint hiện tại.
- [ ] `providers.key` vẫn `UNIQUE NOT NULL`.
- [ ] Không có DB migration không cần thiết.
- [ ] Provider cũ giữ nguyên key.
- [ ] `201 Created` trả generated key.
- [ ] Generated key dùng được ngay cho `GET /providers/{key}`.
- [ ] Generated key dùng được cho Template Provider Binding.
- [ ] PATCH không cho đổi key.
- [ ] Runtime/diagnostic flow không bị refactor không cần thiết.
- [ ] `docs/api/00-all-apis.postman_collection.json` đã bỏ key khỏi Create Provider body.
- [ ] Postman vẫn giữ `{{provider_key}}` cho các endpoint sau create.
- [ ] `docs/flow.md` không còn hướng dẫn user nhập/generate Provider key.
- [ ] Web Create Provider không còn ô key nếu web nằm trong scope.
- [ ] Tests mới cho server-generated key pass.
- [ ] Existing provider/runtime/template tests không regression.

---

## 24. Non-goals

Không thực hiện trong thay đổi này:

- đổi key của Provider cũ;
- chuyển API sang provider ID;
- xóa `key` khỏi Provider response;
- đổi Template Provider Binding sang numeric ID;
- refactor runtime manager;
- thay đổi Provider revision semantics;
- thay đổi secret handling;
- thay đổi provider adapter descriptor;
- thêm DB migration chỉ để generate key.

---

## 25. Kết quả mong muốn

Sau refactor, flow phải đơn giản như sau:

```text
User
  |
  | name/type/adapter/config
  v
Web
  |
  | POST /api/admin/providers
  | no key
  v
Server
  |
  |- validates config
  |- generates immutable provider key
  |- persists Provider
  `- returns 201 + key
              |
              v
Web Provider Detail
  |
  |- display generated key read-only
  |- bind Template
  |- prepare runtime
  `- test Provider
```

Nguyên tắc cuối cùng:

> **Provider name là thông tin do user quản lý. Provider key là identity do server quản lý.**
