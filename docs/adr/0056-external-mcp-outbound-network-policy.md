# ADR 0056 — External MCP outbound bắt buộc network policy

## Status

Accepted

External MCP destination phải match hostname/CIDR allowlist sau DNS resolution; HTTPS luôn dùng normal certificate-chain và hostname validation, không có insecure TLS bypass; HTTP cần explicit LAN enable. URL không có userinfo/query/fragment, automatic redirects tắt, và logging chỉ giữ MCP server key, error kind, scheme. Auth chỉ là typed none/bearer/header với Secret Reference opaque; static headers được canonicalize, protected headers bị block, request assembly inject typed auth sau static/standard header và không có generic post-auth insertion. Điều này ngăn Admin API biến database record thành arbitrary HTTP client.

## Loopback nằm trong LAN scope của HTTP exception

Loopback (`127.0.0.0/8` và `::1/128`) là một phần của private scope mà `allow_http_lan` mở ra, vì MCP server trong homelab thường chạy cùng host với agent. Điều này **không** tạo bypass nào: một HTTP loopback destination chỉ hợp lệ khi **cả hai** điều kiện đều đúng —

1. `mcp.external.network.allow_http_lan = true`; và
2. operator đã ghi tường minh destination đó vào allowlist: một IP literal phải match `allowed_cidrs`, một hostname phải match `allowed_hosts`.

Cùng một destination phải vẫn vượt qua validation sau DNS resolution, nên một hostname resolve ra ngoài allowlisted range bị từ chối ngay trước khi connect. Không có test-only bypass, và loopback không thay thế cho allowlist: allowlist rỗng thì loopback cũng bị từ chối.

Hệ quả thực thi: host phải được so khớp bằng **typed host** (`url::Host::Domain | Ipv4 | Ipv6`), không phải `Url::host_str`, vì `host_str` render IPv6 literal dạng `[::1]` và một chuỗi bracket không bao giờ match CIDR nào.

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

