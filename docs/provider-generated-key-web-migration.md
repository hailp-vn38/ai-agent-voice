# Provider key do server sinh — hướng dẫn web cập nhật

Áp dụng cho web/admin console gọi Admin API của `voice-agent-server`.

> Nguồn đầy đủ: `docs/provider-server-generated-key-guide.md`
> Collection Postman đã cập nhật: `docs/api/00-all-apis.postman_collection.json`

---

## 1. Tóm tắt

Trước đây web tự sinh `key` rồi gửi lên khi tạo Provider. Nay **server tự sinh `key`**.

Hệ quả trực tiếp:

| Việc | Trước | Sau |
|---|---|---|
| Web gửi `key` khi tạo Provider | Bắt buộc | **Không được gửi** |
| Web tự sinh key từ tên (slug) | Có | **Bỏ hẳn** |
| Web validate key trước khi gửi | Có | **Bỏ hẳn** |
| Web hiển thị ô nhập Key | Có | **Bỏ hẳn** |
| Web lấy `key` để dùng tiếp | Tự biết trước | **Lấy từ response `201`** |

Đây là **breaking change có chủ đích**. Client cũ gửi `key` sẽ bị từ chối, không bị bỏ qua âm thầm.

---

## 2. `POST /api/admin/providers` — thay đổi

### 2.1. Request

Bỏ trường `key`. Chỉ còn dữ liệu nghiệp vụ:

```http
POST /api/admin/providers
Authorization: Bearer <admin_token>
Content-Type: application/json
```

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

### 2.2. Response `201 Created`

`key` **vẫn có** trong response, do server sinh ra:

```json
{
  "id": 1,
  "key": "llm_6eb737d745d74285ab916b723eed3671",
  "name": "OpenAI LLM",
  "type": "llm",
  "adapter": "openai",
  "config_json": "{\"base_url\":\"https://api.openai.com/v1\",\"model\":\"m\",\"timeout_ms\":30000,\"max_tokens\":1024}",
  "enabled": 1,
  "revision": 1,
  "created_at": 1791118119,
  "updated_at": 1791118119,
  "has_secret_ref": true,
  "runtime_status": "not_loaded",
  "runtime_matches_desired": false,
  "requires_restart": true
}
```

Ghi chú:

- `key` có dạng `{provider_type}_{uuid32}`, ví dụ `llm_6eb737d745d74285ab916b723eed3671`.
  Không suy diễn format này để tạo key phía web — chỉ đọc và hiển thị.
- `config_json` là **chuỗi JSON**, không phải object. Parse một lần nếu UI cần đọc.
- `enabled` là `0 | 1`, không phải boolean.
- `secret_ref` không bao giờ được trả về; chỉ có `has_secret_ref`.

### 2.3. Lỗi khi web vẫn gửi `key`

Request có `key` (hoặc bất kỳ field lạ nào) bị từ chối ở tầng parse:

```http
HTTP/1.1 400 Bad Request
```

```json
{
  "error": {
    "code": "invalid_json",
    "request_id": "b9df463f-adaf-4f8c-ad92-8b82ce3d8173"
  }
}
```

Lưu ý: `deny_unknown_fields` áp dụng cho **mọi** field không khai báo, không riêng `key`.
Nếu web gửi thừa field nào đó cũng sẽ nhận `invalid_json`.

### 2.4. Bảng error của create

| Trường hợp | Status | `error.code` |
|---|---|---|
| Gửi thừa field, gồm cả `key` | 400 | `invalid_json` |
| `name` rỗng/quá 128 ký tự | 400 | `validation_failed` |
| `type` không thuộc `vad\|asr\|llm\|tts` | 400 | `validation_failed` |
| `adapter` không khớp `type` | 400 | `validation_failed` |
| `secret_ref` không hợp lệ | 400 | `validation_failed` |
| `config_json` sai cấu trúc adapter | 400 | `provider_config_invalid` |
| Trùng `key` (cực hiếm, UUID v4) | 409 | `resource_conflict` |
| Database không sẵn sàng | 503 | `database_unavailable` |

Error contract **không đổi** so với trước. Không có error code mới.

---

## 3. `PATCH /api/admin/providers/{key}` — không đổi contract

Gửi `key` trong body vẫn bị từ chối:

```json
{ "key": "new_key" }
```

```http
HTTP/1.1 400 Bad Request
```

```json
{ "error": { "code": "immutable_field", "request_id": "..." } }
```

`key` là **immutable**: đổi tên Provider (`name`) không đổi `key`.

---

## 4. Endpoint nào **không** đổi

Tất cả endpoint sau vẫn dùng `provider.key` như trước:

| Method | Path |
|---|---|
| GET | `/api/admin/providers` |
| GET | `/api/admin/providers/{key}` |
| PATCH | `/api/admin/providers/{key}` |
| DELETE | `/api/admin/providers/{key}` |
| POST | `/api/admin/providers/{key}/prepare` |
| POST | `/api/admin/providers/{key}/test/llm` |
| POST | `/api/admin/providers/{key}/test/tts` |
| POST | `/api/admin/providers/{key}/test/asr` |
| POST | `/api/admin/providers/{key}/test/vad` |
| GET | `/api/admin/providers/{key}/capabilities` |
| GET | `/api/admin/providers/{key}/templates` |

Template Provider Binding **không đổi**:

```http
PUT /api/admin/templates/{template_key}/providers/{provider_type}
```

```json
{ "provider_key": "llm_6eb737d745d74285ab916b723eed3671" }
```

Chỉ có giá trị `provider_key` trong body này là lấy từ response `201` hoặc từ provider list/detail.

---

## 5. Provider đã tồn tại

Provider cũ **giữ nguyên key cũ**. Không migrate sang format UUID.

Nghĩa là web phải xử lý được cả hai dạng key:

- cũ, do người dùng đặt tay: `llm_main`, `tts_maichi`
- mới, do server sinh: `llm_6eb737d745d74285ab916b723eed3671`

Không validate format, không parse `key` thành type + tên. Dùng `provider.type` cho loại,
`provider.key` chỉ là opaque identifier.

---

## 6. TypeScript

### 6.1. Request type

```diff
  interface CreateProviderRequest {
-   key: string
    name: string
    type: ProviderType
    adapter: string
    config_json: unknown
    secret_ref?: string
  }
```

### 6.2. Response type — không đổi

```ts
  interface Provider {
    id: number
    key: string
    name: string
    type: ProviderType
    adapter: string
    config_json: string
    enabled: number
    revision: number
    created_at: number
    updated_at: number
    has_secret_ref: boolean
    runtime_status: 'not_loaded' | 'unavailable' | 'loaded'
    runtime_matches_desired: boolean
    requires_restart: boolean
    runtime?: ProviderRuntimeInspection
  }
```

`key` **vẫn bắt buộc có** trong `Provider`. Không xóa.

### 6.3. Gọi API

```diff
- const key = slugify(form.name)
- const provider = await api.createProvider({
-   key,
-   name: form.name,
-   type: form.type,
-   adapter: form.adapter,
-   config_json: form.config,
-   secret_ref: form.secretRef,
- })
+ const provider = await api.createProvider({
+   name: form.name,
+   type: form.type,
+   adapter: form.adapter,
+   config_json: form.config,
+   secret_ref: form.secretRef,
+ })
+ // provider.key là key do server sinh — dùng ngay cho mọi request sau đó
+ router.push(`/providers/${provider.key}`)
```

---

## 7. UI

### 7.1. Form tạo Provider

Bỏ hoàn toàn:

- trường **Key**
- logic sinh key từ tên (slugify)
- validate key phía client (chữ thường, `_`, tối đa 64 ký tự)

Còn lại: `name`, `type`, `adapter`, config theo adapter, `secret_ref` nếu cần.

Ở bước review trước khi tạo, không hiển thị key. Nếu cần dòng key thì hiển thị
`Key: Server tự tạo`.

### 7.2. Sau khi tạo

- Lấy `response.key`, dùng làm định tuyến sang trang chi tiết Provider.
- Hiển thị key ở dạng metadata kỹ thuật: **read-only**, có nút copy, **không sửa được**.
- `name` vẫn là thông tin chính trên UI.

### 7.3. Provider selector khi gắn template

Lấy danh sách từ `GET /api/admin/providers`, hiển thị theo `name`.
**Không** có ô text để nhập `provider_key`.

---

## 8. Checklist migration

- [ ] Xóa `key` khỏi `CreateProviderRequest` và khỏi mọi payload gửi `POST /api/admin/providers`.
- [ ] Xóa hàm sinh key từ tên (slug) trong web.
- [ ] Xóa validate key phía client.
- [ ] Xóa trường Key khỏi form tạo Provider.
- [ ] Bước review: không hiển thị key trước khi tạo.
- [ ] Sau `201`: lấy `response.key`, dùng để route + hiển thị read-only.
- [ ] Provider selector cho template binding lấy từ API, không nhập tay.
- [ ] Không parse `key` thành type/tên ở bất kỳ đâu.
- [ ] Test: tạo 2 Provider trùng `name` → vẫn tạo được, `key` khác nhau.
- [ ] Test: gửi `key` cũ → nhận `400 invalid_json`, hiển thị lỗi hợp lý cho người dùng.
- [ ] Test: `GET /api/admin/providers/{key}` ngay sau khi tạo → `200`, `key` khớp.
- [ ] Test: gắn template bằng `response.key` → thành công.

---

## 9. Ghi chú vận hành

`provider.key` **không phải secret**. Nó xuất hiện trong URL, audit log và metadata debug.

`secret_ref` tiếp tục trỏ tới tên biến môi trường; `config_json` tiếp tục không chứa credential.

Không có DB migration. Cột `providers.key` vẫn là `TEXT NOT NULL UNIQUE`.
Sau khi deploy, web cũ chưa cập nhật sẽ hỏng ở bước tạo Provider — cần cập nhật web
trước hoặc cùng lúc với server.