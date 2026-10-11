# Cải tiến thời gian kết nối WebSocket

Ngày: 2026-10-11. Triển khai trên baseline `596d340`, theo yêu cầu thực hiện và lựa chọn budget discovery MCP tổng 500 ms của người dùng. Chưa commit hoặc deploy.

## Thay đổi

- Giữ interface của profile resolver; chạy chuẩn bị runtime bắt buộc và discovery/review External MCP song song sau DB admission. Runtime lỗi vẫn từ chối upgrade và hủy discovery đang chờ. Không thay đổi accounting của native attempts hoặc immutable session snapshot.
- Dùng lại ProviderPrewarm: startup chọn enabled default Templates và Device overrides có assignment hợp lệ. Create Device, đổi Agent/override, re-enable hoặc claim enrollment enqueue selected Template sau DB commit. Query không fallback khi override không hợp lệ.
- Thêm chín phase WS vào RuntimeMetrics và log `ws_connected` sau khi writer thực sự send ServerHello thành công. Total bắt đầu tại handler, không gồm DNS/TCP/TLS hoặc xác nhận client nhận hello. Các stage chạy song song không được cộng để suy ra total.
- Local `docker/config.prod.toml` và tracked `docker/config.prod.example.toml` dùng discovery overall 500 ms, per-server 400 ms. File local bị gitignore nên không nằm trong patch tracked. Không đổi timeout thực thi tool hoặc global config defaults.
- [Flow WS](../../docs/flows/01-websocket.md) mô tả admission, prewarm, metrics và giới hạn budget mới.

## Đo lại

Probe dùng binary vừa build, Qualification providers, DB riêng và HTTP MCP giả lập trên localhost. Mỗi case có 10 kết nối; bảng là median TCP connect → nhận ServerHello. Cột config cũ/mới dưới đây so sánh budget trên cùng binary mới, không phải benchmark native load trước/sau.

| Case | Tổng |
| --- | ---: |
| Không bind MCP | 3,074 ms |
| MCP nhanh | 8,622 ms |
| MCP initialize chậm 600 ms, budget cũ | 624,665 ms |
| Cùng MCP chậm, budget production 400/500 ms | 410,439 ms |

Budget mới giảm khoảng 34% trong case MCP chậm này bằng cách exclude server vượt timeout. [measurements.json](measurements.json) lưu các samples; [implementation-probe.log](implementation-probe.log) lưu self-checks. Không dùng kết quả này làm p95/p99 production hoặc bằng chứng tăng tốc cold model thật.

## Kiểm tra

- 145 integration tests liên quan pass: WS protocol, External MCP admission, runtime manager, session profile, Admin API, lifecycle, enrollment và provider bindings. Log: [implementation-tests.log](implementation-tests.log).
- Suite regression kết nối/prewarm cuối cùng: 5 pass, gồm parallel discovery khi ASR còn cold, fail-fast required runtime, create Device, đổi override, claim enrollment. Trong đó 4 đã nằm trong lần chạy trên; claim enrollment được bổ sung sau. Log: [latency-tests.log](latency-tests.log).
- 3 unit tests prewarm pass, gồm query defaults/overrides và disabled/invalid assignments. Log: [prewarm-tests.log](prewarm-tests.log). Tổng 149 tests khác nhau trong các lần chạy liên quan.
- Test parallel discovery được chạy red trước sửa resolver, rồi green sau sửa.
- `cargo clippy --locked -p voice-agent-server --all-targets --all-features -- -D warnings`, `cargo fmt --all --check`, SQLx placement và `git diff --check` pass. Config TOML local/mẫu parse được và có đúng budget 400/500 ms.
- Qualification binary build thành công, probe 5 cases pass. Logs build/lint: [implementation-build.log](implementation-build.log), [clippy.log](clippy.log).

## Giới hạn

Chưa deploy, đo model thật, firmware, proxy/TLS hoặc p95/p99 production. Prewarm vẫn bounded và best-effort; chưa bảo đảm runtime Ready trước lần kết nối đầu tiên. MCP vượt budget không có tools từ server đó trong session này; chưa có số liệu production để biết tỷ lệ exclusions. Đo tại client và quan sát metrics mới cần thiết để xác nhận SLO thực tế.
