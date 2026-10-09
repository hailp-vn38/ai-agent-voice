# Development, deployment và troubleshooting

## Môi trường và commands

Dùng Node.js **22.12+** theo `package.json`; dependency versions và lockfile thuộc ứng dụng này. `npm ci` cài từ lockfile, không cần thêm dependency để đọc/chỉnh tài liệu.

```bash
cd apps/admin-web
cp .env.example .env
npm ci
npm run dev
```

| Command | Tác dụng |
| --- | --- |
| `npm run dev` | Vite ở port 5173, host 0.0.0.0 |
| `npm test` | Vitest run, jsdom, `src/**/*.test.ts` |
| `npm run test:watch` | Vitest watch |
| `npm run typecheck` | vue-tsc strict, không emit |
| `npm run build` | typecheck rồi Vite production build vào `dist/` |
| `npm run preview` | Phục vụ local bản build; không development proxy |

Chưa có script lint/E2E trong package. Không dùng lệnh root/backend thay cho test riêng của ứng dụng này.

## Environment và proxy

`VITE_API_BASE_URL` bỏ trống để dùng cùng origin; khi có giá trị, URL được nhúng vào JS tại build time. Đổi `.env` rồi rebuild hoặc restart dev server để áp dụng. Không coi `VITE_*` là secret server-side.

Vite proxy `/api`, `/health`, `/ready`, `/mcp/vision/` và `/voice` (WebSocket) tới `VITE_DEV_PROXY_TARGET`, fallback `http://127.0.0.1:8000`. Proxy chỉ thuộc dev server. Backend Admin API phải bật (`api.enabled = true`) và có Admin token; disabled surface trả 404, credential thiếu/sai trả 401 theo ADR 0051.

Microphone cần secure browser context và permission: HTTPS hoặc localhost development. HTTP qua LAN IP có thể hiển thị UI nhưng không có `navigator.mediaDevices`/AudioWorklet. Provider API key submission còn guard HTTPS riêng; MCP credential submission theo policy hiện cho phép HTTP.

## Triển khai

Build artifact là static `dist/`. Router dùng history mode ở root path, nên server static phải fallback về `index.html` cho route ứng dụng như `/agents/example` khi mở trực tiếp hoặc reload. Không fallback request API sang HTML.

Hai mô hình:

- **Cùng origin:** reverse proxy `/api`, `/health`, `/ready` tới backend, phần còn lại phục vụ static + SPA fallback. Base API URL rỗng.
- **API origin riêng:** build với `VITE_API_BASE_URL`, backend/proxy cho phép đúng browser origin và preflight Authorization/If-Match. TLS endpoint phải phù hợp credential policy.

Ví dụ nginx cho mô hình cùng origin, backend ở `127.0.0.1:8000` và `dist/` được copy vào document root:

```nginx
location /api/ {
    proxy_pass http://127.0.0.1:8000;
}
location = /health {
    proxy_pass http://127.0.0.1:8000;
}
location = /ready {
    proxy_pass http://127.0.0.1:8000;
}
location / {
    try_files $uri $uri/ /index.html;
}
```

Đây là routing snippet, chưa cấu hình listener/TLS hay publish một Voice endpoint. Admin UI không cần mở Voice WS để render các trang quản trị. App/worklet đang dùng root-relative paths; deploy dưới subpath cần thay base/router/worklet URL trước, chưa có config sẵn cho trường hợp đó.

Ưu tiên HTTPS và runtime token input. `VITE_ADMIN_TOKEN` được nhúng vào bundle; dùng nó nghĩa là người có quyền tải static bundle có bearer credential. SQLite chứa encrypted resource credentials nên server key/backup là trách nhiệm backend deployment.

## Kiểm tra khi sửa code

Tests đặt cạnh API, store, components/pages, composables/lib. Dùng suite có sẵn và test nhỏ ở boundary thay đổi: request body/headers, reactive UI sau mutation, revision sau relationship write, cancel khi đổi resource, WAV và enrollment.

Ví dụ chạy focused tests:

```bash
npm exec vitest run src/stores/admin.test.ts
npm exec vitest run src/pages/devices/DeviceDetailPage.test.ts
npm exec vitest run src/components/providers/ProviderTestPanel.test.ts
```

API mocks/jsdom không xác nhận backend conformance, CORS/TLS, microphone permission, audio quality hoặc firmware enrollment. Khi thay contract, đối chiếu Rust handlers/DTO và kiểm tra thực tế với backend riêng.

Kết quả review 2026-10-09: 38 test files / 126 tests pass; typecheck và production build pass. Môi trường chạy có Node `20.19.2`, thấp hơn engines yêu cầu; kết quả này không thay thế kiểm tra trên Node 22.12+. Assert JavaScript/Vue kiểm riêng shallowRef và việc ghi đè AbortSignal; xem [báo cáo](review.md).

## Troubleshooting

### Token sai hoặc 401

UI giữ token trước khi API xác nhận, sau đó ẩn form Connect; Retry không đổi credential. Hiện workaround: DevTools → Application → Session Storage, xóa `voice-agent-admin-token`, reload rồi nhập đúng. Chỉ xóa token của ứng dụng này, không cần clear toàn bộ storage.

Nếu dùng `VITE_ADMIN_TOKEN`, xóa session token vẫn dùng fallback; bỏ/sửa env và restart/rebuild. Không nhầm với Voice/OTA token. Flow đổi token/logout cần sửa UI, đã ghi trong review.

### 404 hoặc response JSON lỗi

Kiểm tra API enabled, base URL và proxy target. Nếu `requestJson` nhận HTML thay JSON, reverse proxy có thể đang fallback API sang `index.html`. Với detail route 404 khi reload nhưng navigation trong UI hoạt động, cấu hình SPA fallback.

### 409 / revision conflict

Refresh resource để lấy revision mới, kiểm tra giá trị đã bị người khác đổi rồi thử lại. Tool review `contract_conflict` cần observation/fingerprint hiện tại. Không tự tăng revision ở browser hoặc retry write mù.

### Không xóa được resource

Đọc code lỗi `*_in_use`. Unlink active relationship trước; Template provider bindings và history cũng là references. History purge không có UI, không xóa database trực tiếp để vượt guard. Provider catalog delete có partial unlink risk, trong khi detail chặn khi usage còn.

### Microphone không hoạt động

Kiểm tra HTTPS/localhost, microphone permissions và input device. Xác nhận `/enrollment-worklet.js` được phục vụ dạng JS, không phải SPA HTML. Summary recognition phải available trước enrollment; backend reject clip quality/expiry thì thu lại.

### Provider đã lưu nhưng chưa ready

Mở Provider Detail xem desired/ready revisions, runtime match, failure và prepare/restart availability. Catalog badge không phải health guarantee. Diagnostic pass không tự prepare runtime. New credential không thay snapshot trong runtime/session đang active.

### Dữ liệu không hiện hoặc vẫn giá trị cũ

Store chỉ tải trang đầu của nhiều resource; cache detail/counts có thể thiếu. Các `upsert*()` dùng shallowRef + splice nên mutation có thể không render ngay. Reload workaround; sửa root cause theo review. Devices Detail GET raw tránh lỗi identity/revision của tab Device trong Agent.
