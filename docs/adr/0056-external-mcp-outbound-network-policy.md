# ADR 0056 — External MCP outbound network policy

## Status

Accepted; revised 2026-10-08 theo yêu cầu cho phép HTTP mà không cần cấu hình LAN/CIDR.

External MCP chấp nhận HTTP và HTTPS mặc định, bao gồm LAN và loopback. Không còn `allow_http_lan` hoặc `allowed_cidrs`; deployment dùng config cũ phải xóa hai field này trước khi khởi động vì config từ chối unknown fields.

`allowed_hosts` là allowlist hostname tùy chọn: rỗng thì mọi host được chấp nhận; có entry thì destination phải match exact hostname hoặc wildcard subdomain. Không áp IP/CIDR policy sau DNS resolution. Admin có thể chọn destination mà server truy cập được; HTTP truyền dữ liệu và credential không mã hóa. Đây là trade-off được chọn để thêm MCP trực tiếp qua Admin API mà không phải cấu hình LAN/CIDR ngoài ứng dụng.

HTTPS vẫn validate certificate-chain và hostname, không có insecure TLS bypass. URL không có userinfo/query/fragment; automatic redirects tắt. Logging chỉ giữ MCP server key, error kind, scheme. Auth chỉ là typed none/bearer/header với Secret Reference opaque; static headers được canonicalize, protected headers bị block, request assembly inject typed auth sau static/standard header và không có generic post-auth insertion.

## Metrics

External MCP là feature đầu tiên của service báo cáo bằng số chứ không chỉ bằng log. Telemetry seam là process-owned và **bounded**: caller chọn metric từ tập cố định và label từ bounded class, nên không có cách nào để destination, header value, credential, protocol session id, tool argument hay tool result trở thành label — chúng không phải tham số ở đó. Giá trị tự do duy nhất là `server_key`, vốn đã bị Admin API bound ở 64 byte `[a-z0-9_]`.

Hai điểm guide để ngỏ, đã chốt:

- **Resolve duration mang cùng label với counter đi cạnh.** `mcp_resolve_success_total{server_key,outcome}`,
  `mcp_resolve_failure_total{server_key,outcome,reason}` và `mcp_resolve_duration_ms{server_key,outcome}`
  dùng chung một tập label, nên một server chậm tìm được từ duration chứ không chỉ từ một failure.

- **Aggregate cap có counter riêng, không mượn counter per-server.** Cap tổng thuộc về cả snapshot chứ
  không thuộc về server nào, nên nó đếm một lần vào `external_mcp_session_tool_cap_exceeded_total`
  không mang label, và **không** phát `mcp_resolve_failure_total` hay duration per-server cho nó. Mỗi
  server đã discovery thành công vẫn được đếm là resolved, kể cả khi snapshot aggregate sau đó bị
  reject. Đếm nó thêm một lần nữa như một server failure sẽ báo ra một outcome thứ hai, và một
  duration cho công việc chưa từng xảy ra.

