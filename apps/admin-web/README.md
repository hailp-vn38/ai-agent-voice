# Voice Agent Studio — Admin Web

Ứng dụng Vue độc lập để quản lý `voice-agent-server`: Agents, Templates, Providers, Devices, Speakers và External MCP. Đây là control plane; browser không sở hữu Voice Session state và không đọc/ghi trực tiếp `config.toml` hay SQLite.

## Chạy local

Yêu cầu Node.js **22.12 trở lên**, npm, backend đang chạy (mặc định `http://127.0.0.1:8000`) với Admin API được bật và một Admin bearer token. Token này khác token Voice/OTA.

```bash
cd apps/admin-web
cp .env.example .env
npm ci
npm run dev
```

Mở `http://localhost:5173`, nhập token vào form Connect. Vite listen trên `0.0.0.0`; dùng URL localhost cho development, đặc biệt khi thu microphone hoặc gửi Provider API key.

| Biến | Giá trị mặc định | Ý nghĩa |
| --- | --- | --- |
| `VITE_API_BASE_URL` | Rỗng | Request cùng origin; development đi qua Vite proxy |
| `VITE_DEV_PROXY_TARGET` | `http://127.0.0.1:8000` | Backend của development proxy |
| `VITE_ADMIN_TOKEN` | Rỗng | Token fallback được nhúng vào bundle; mọi người tải bundle có thể đọc được |

Ưu tiên nhập token lúc chạy. Token nhập được giữ trong `sessionStorage` với key `voice-agent-admin-token`, ưu tiên hơn fallback env. UI hiện chưa có thao tác logout/đổi token sau khi Connect; cách phục hồi khi nhập sai có trong [hướng dẫn vận hành](docs/development.md#token-sai-hoặc-401).

## Trang và khả năng hiện tại

| Đường dẫn | Chức năng |
| --- | --- |
| `/overview` | Số lượng trong cache, readiness và runtime aggregates từ server |
| `/agents`, `/agents/:agentId` | Quản lý Agent; Studio, External Tools, Speakers, Devices |
| `/templates`, `/templates/:templateId` | Template dùng chung, prompt, provider bindings, liên kết Agent |
| `/providers`, `/providers/:key` | Catalog, cấu hình theo adapter, diagnostics và prepare runtime |
| `/devices`, `/devices/:deviceId` | Enrollment claim, chỉnh thông tin/Agent/override, admission enable/disable, xóa |
| `/speakers`, `/speakers/:speakerKey` | Hồ sơ người nói, thu mẫu, thay/purge voiceprint, liên kết Agent |
| `/mcp`, `/mcp/:key` | Streamable HTTP server, credentials, connection/discovery diagnostics |
| `/system` | Health probe mỗi 30 giây và thông tin/cache quản trị |

`/` và `/dashboard` chuyển đến `/overview`. Route không tồn tại cũng chuyển về Overview. Chi tiết hành vi và giới hạn ở [luồng sử dụng](docs/workflows.md).

Template API chỉ có bốn slot **VAD / ASR / LLM / TTS**. Speaker là nhận dạng advisory với policy `off`/`observe`, không phải slot của Template. Vision có mặt trong một số kiểu/component UI nhưng chưa có Template binding tương ứng. Chưa có Playground, Reports, trang History hay trạng thái kết nối Device trực tiếp.

## Kiểm tra và build

```bash
npm test
npm run typecheck
npm run build
npm run preview
```

`build` đã chạy typecheck trước Vite. `preview` chỉ phục vụ artifact local, không có development proxy. Deploy `dist/` với SPA fallback và reverse proxy API, hoặc cấu hình API base URL khi build; xem [triển khai](docs/development.md#triển-khai).

## Tài liệu chi tiết

- [Mục lục và phạm vi](docs/README.md).
- [Kiến trúc và mô hình dữ liệu](docs/architecture.md).
- [HTTP/API, revisions và credentials](docs/api.md).
- [Luồng sử dụng theo trang](docs/workflows.md).
- [Development, deployment và troubleshooting](docs/development.md).
- [Review mã nguồn ngày 2026-10-09](docs/review.md): lỗi đã xác nhận, ưu tiên sửa, kiểm tra và giới hạn review.

Stack hiện tại: Vue 3, Vue Router, Pinia, TypeScript, Vite, Tailwind CSS 4, Lucide; UI primitives do repo sở hữu theo conventions shadcn-vue. `src/i18n/messages.ts` chứa catalog EN/VI, theme mặc định dark. Một số màn hình mới và thông báo lỗi vẫn hard-code tiếng Việt.

Review lần này chỉ cập nhật tài liệu. Test/build pass không đồng nghĩa các lỗi trong báo cáo đã được sửa.

## Cấu hình server và cấu trúc source

Bổ sung vào cấu hình server hợp lệ để bật Admin API:

```toml
[api]
enabled = true
admin_token = "replace-with-your-admin-token"
```

Server vẫn cần SQLite và Provider configuration phù hợp. Xem [config.example.toml](../../config.example.toml) và [README server](../../README.md); web không tự thay cấu hình server.

```text
src/
  api/          HTTP clients, wire DTOs, errors và API tests
  stores/       Auth, admin read model, server probes
  domain/       View models
  router/       Routes và redirects
  views/        Catalog, Overview, System
  pages/        Detail pages và Template catalog
  components/   UI primitives và thành phần theo feature
  composables/  i18n, theme, microphone recorder
  i18n/         English/Vietnamese messages
  lib/          WAV, endpoint redaction, formatting
  layouts/      Shell và mobile navigation
  config/       Sidebar navigation
public/         Static assets, enrollment-worklet.js
docs/           Tài liệu ứng dụng
```

Kiểm chứng ngày 2026-10-09: **38 test files / 126 tests pass**, typecheck và production build pass. Chưa kiểm live backend, browser E2E hoặc microphone/hardware thật. Các lỗi được review vẫn cần sửa, đặc biệt binding persistence và Device revision/identity.
