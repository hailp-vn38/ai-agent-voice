# External MCP Studio — thiết kế trang và hướng dẫn đồng bộ Vue ↔ Rust

Trạng thái: **đã bổ sung giao diện MCP catalog, Agent bindings và External Tool Review** trên nhánh `feat/voice-agent-studio-ui`. **Không thêm backend endpoint mới.**

## 1. Phạm vi

- Trang `/mcp` tại nhóm **Infrastructure**: danh mục MCP Server toàn cục, tìm kiếm trong dữ liệu đã tải, tạo, sửa, bật/tắt, xóa.
- Agent Detail → tab **External Tools**: liên kết/gỡ/bật/tắt MCP Server của Agent và duyệt từng External Tool đã được server quan sát.
- Sử dụng `src/api/mcp.ts`, `src/api/agents.ts`, `src/api/external-tools.ts` cùng `src/api/client.ts`; không `fetch` trực tiếp trong Vue component.
- UI thiết kế theo Voice Agent Studio: `studio-panel`, semantic status, dark/light, vi/en, kiểm soát revision.
- **Không** đổi `AgentTemplate`, không chuyển External MCP thành Template Provider, không gán quyền approval cho Device MCP tools.

## 2. Flow chính (đúng ownership)

```text
Admin
  │
  ▼
/mcp → POST /api/admin/mcp-servers
  │    Persist desired configuration (global), auth via server-owned SecretRef
  ▼
/agents/:id → External Tools
  │ PUT /api/admin/agents/:agent_key/mcp-bindings/:mcp_key
  │ If-Match: "<agent_revision>", { "enabled": true, "required": false }
  ▼
Voice WS admission (Rust-owned)
  │ resolve allowed MCP servers, validate network/secret; discovery hoàn tất
  │ observed external tool contracts trong SQLite
  ▼
Agent External Tool Review
  │ GET /api/admin/agents/:key/tool-allowlist
  │ PUT /api/admin/agents/:key/tool-allowlist, If-Match: "<tool_revision>"
  │ requires exact server_key, original_name, observed_revision, fingerprint
  ▼
Agent execution guard (Rust-owned)
  │ admission snapshot + before-outbound security recheck
  ▼
Tool call accepted / blocked
```

**Không có chuyện vừa thêm MCP Server → mọi Agent được sử dụng ngay.** `enabled` toàn cục ≠ `binding.enabled` ≠ `tool.allowed` ≠ kết nối online. Required luôn false trong V1; server trả `required_unsupported` khi gửi true.

## 3. Component/API map đã triển khai

| Component | Vai trò |
|---|---|
| `src/views/McpServersView.vue` | Danh mục toàn cục; filters trên entries đã tải; pagination incremental; mutation refresh |
| `src/components/mcp/McpServerFormModal.vue` | Form tạo/sửa, authentication SecretRef, header replacement explicit |
| `src/components/agents/AgentMcpBindings.vue` | Agent ↔ Server bindings, Agent revision và quyền bật/tắt |
| `src/components/agents/AgentToolAllowlist.vue` | Tool contract review đúng fingerprint/revisions; không hiện auth source |
| `src/api/mcp.ts` | MCP Server CRUD qua centralized HTTP |
| `src/api/types/mcp.ts` | Wire types sửa theo Rust actual JSON |
| `src/api/agents.ts` | Agent CRUD + `mcpBindings`, `bindMcpServer`, `unlinkMcpServer` |
| `src/api/external-tools.ts` | `GET/PUT tool-allowlist` typed, không gọi HTTP trong view |
| `src/config/navigation.ts`, `src/router/index.ts` | Link/route `/mcp` |
| `src/i18n/messages.ts` | vi/en message keys |

## 4. API wire contract đã kiểm tra trực tiếp từ Rust

### 4.1 MCP Server Catalog

```http
GET    /api/admin/mcp-servers?page=1&page_size=50
POST   /api/admin/mcp-servers
GET    /api/admin/mcp-servers/{key}
PATCH  /api/admin/mcp-servers/{key}
DELETE /api/admin/mcp-servers/{key}
```

Create payload:

```json
{
  "key": "weather",
  "name": "Weather Service",
  "url": "https://example.com/mcp",
  "headers": {},
  "auth": {"type": "bearer", "secret_ref": "MCP_WEATHER_TOKEN"},
  "connect_timeout_ms": 5000,
  "request_timeout_ms": 30000
}
```

`McpAuthInput` dùng khi ghi: `none`, `bearer+secret_ref`, `header+header_name+secret_ref`. Server kiểm tra key, name, URL theo network policy, headers, SecretRef và timeout. V1 chỉ `streamable_http`.

**GET che SecretRef**:

```json
{
  "key": "weather",
  "name": "Weather Service",
  "transport": "streamable_http",
  "url": "https://example.com/mcp",
  "headers": {},
  "auth": {"type": "bearer", "has_secret_ref": true},
  "connect_timeout_ms": 5000,
  "request_timeout_ms": 30000,
  "enabled": true,
  "revision": 1,
  "created_at": 1760000000,
  "updated_at": 1760000000
}
```

Ví dụ timestamp/response ở trên để mô tả **schema**, không phải response từ một server đang chạy. Do đó không dùng `McpAuthInput` làm GET type. Khi edit: `auth: undefined` → giữ secret; chỉ gửi `auth` khi người quản trị **chủ động** thay, nhập SecretRef mới. Form không prefill plaintext hoặc SecretRef đã che. `headers` cũng không prefill giá trị, chỉ cho thay thế có chủ ý, tránh sao chép credentials khi edit.

**Paging quan trọng:** Rust `GET /mcp-servers` hiện trả `{ items, page, page_size, max_page_size }` **không có** `total`/ `total_pages`. Rust query chỉ `ORDER BY key LIMIT/OFFSET`; `enabled` query hiện **không áp filter**. UI lọc client-side **trên những trang đã tải**, dùng nút Load more khi response đầy `page_size`, không hiện tổng giả hay `online` giả.

### 4.2 Agent binding

```http
GET    /api/admin/agents/{agent_key}
GET    /api/admin/agents/{agent_key}/mcp-bindings
PUT    /api/admin/agents/{agent_key}/mcp-bindings/{server_key}
DELETE /api/admin/agents/{agent_key}/mcp-bindings/{server_key}
```

GET binding trả:

```json
{
  "items": [
    { "server_key": "weather", "enabled": true, "required": false }
  ]
}
```

**Không có** `revision` trong GET bindings. Lấy revision từ `GET /agents/{agent_key}`. Mỗi PUT/DELETE binding tăng Agent revision → reload `GET /agents/{key}` sau mỗi mutation trước khi có mutation tiếp theo. Body PUT:

```json
{"enabled": true, "required": false}
```

Không gửi `required=true`; không chỉnh `Template` hoặc `Agent.providers`.

### 4.3 Tool review

```http
GET /api/admin/agents/{agent_key}/tool-allowlist
PUT /api/admin/agents/{agent_key}/tool-allowlist
If-Match: "<tool_approval_revision>"
```

PUT:

```json
{
  "server_key": "weather",
  "original_name": "get_forecast",
  "observed_revision": 1,
  "fingerprint": "<exact 64-byte hex value returned by GET>",
  "allowed": true,
  "sensitive": false
}
```

Với code thực phải dùng fingerprint **nguyên văn từ response**, không đặt literal mẫu. `If-Match` dùng **revision của observation approval** (`item.revision`), không dùng Agent revision hay MCP Server revision. Khi `contract_conflict` hoặc `revision_conflict`: refetch list, không retry tự động. Sensitive tool bị server chặn. Source observation có thể chứa auth reference; UI mới **không render source** mà chỉ hiển thị metadata/safe Input Schema.

Observation chỉ chứng minh server đã quan sát contract khi discovery hoàn tất — **không xác nhận MCP Server hiện online**. Server recheck allowlist trước outbound tool start.

### 4.4 Delete

- `DELETE /api/admin/mcp-servers/{key}` cần `If-Match: "<mcp_revision>"`.
- Khi còn `agent_mcp_bindings`, backend trả `409 mcp_server_in_use`: không auto-unlink/cascade; hướng dẫn người quản trị gỡ từng Agent rồi mới xóa.
- Khi URL/auth/headers hoặc enabled thay đổi, server có thể invalidate approvals. UI thông báo rõ, không tự khôi phục approve.
- Network policy (SSRF), SecretRef và tool security guard đều do backend enforce; web không vượt qua.

## 5. UX định nghĩa

Trang `/mcp`:

1. Header **MCP Servers**, tạo MCP, Refresh.
2. Flow 3 bước: **Configure → Bind to Agent → Review Tools**.
3. Search/filter trong dữ liệu đã tải, cards chứa tên, key, endpoint được loại query/credentials, Streamable HTTP, auth mode và cấu hình Enabled/Disabled.
4. Edit dialog gồm `name`, `url`, timeouts, `auth`, header replacement; không có nút Test vì backend chưa có test/discovery endpoint admin.
5. Delete confirmation; lỗi/race hiển thị `request_id` qua `formatApiError`.

Trong Agent Detail → External Tools:

1. `AgentMcpBindings`: select global server chưa liên kết, Link/Enable/Disable/Unlink, Agent revision.
2. `AgentToolAllowlist`: observed tools, fingerprints do server lưu; Approve/Revoke/Mark sensitive.
3. Tool review section refresh khi binding thay đổi, không lấy cached authorization làm runtime truth.

## 6. Tests & Quality gates

```bash
cd apps/admin-web
npm ci
npm run typecheck
npm run test
npm run build
```

Kiểm tra:

- Tạo MCP auth none, bearer, header; SecretRef chỉ ở request, không prefill trên GET.
- Edit không chọn đổi auth/headers: PATCH không chứa `auth`/`headers`.
- Pagination/trạng thái: không đọc `total` hoặc hiểu `enabled` là Online; tìm kiếm chỉ trên đã tải.
- Agent Bind: request dùng `If-Match` từ Agent GET; `required:false`; conflict 409 hiển thị và refetch.
- Tool Review: đúng observed_revision/fingerprint/approval revision; `contract_conflict` refetch; approval không tự mở Sensitive tools.
- Delete blocked nếu còn binding; không auto-unlink.
- Navigation `/mcp`, responsive mobile, vi/en, keyboard, contrast.
- Regression với Agent/Template/Provider/Device/Speaker hiện tại.

> **Chưa có:** MCP Server connect/test API trực tiếp từ Admin; live online status; manual discovery endpoint; history tool latency report. Để thêm chúng phải cập nhật Rust API/contract trước, không hiện trạng thái giả trong Vue.

## 7. Tài liệu liên quan

- `apps/admin-web/README.md`
- `apps/admin-web/docs/API_Integration_Guide.md`
- `apps/admin-web/docs/voice-agent-studio-redesign.md`
- `docs/api/00-all-apis.postman_collection.json`
- `docs/adr/0069-rmcp-external-mcp-protocol-engine.md`
