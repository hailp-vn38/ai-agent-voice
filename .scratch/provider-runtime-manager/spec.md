# Provider Runtime Manager

Status: resolved

Đặc tả: [three-phase guide](../../docs/provider-runtime-manager-three-phase-guide.md). Baseline: `b3ef953fdeb9f2a83d1513f7f01b700d3fae83a2`.

Triển khai tuần tự D1 → D2 → D3. API/WS, materializer có injection, worker lifecycle là các seam kiểm thử được đặc tả lựa chọn. Không đánh dấu một đợt complete trước các gate tương ứng. Deployment defaults và database identities có namespace riêng.

Theo dõi bằng từng issue trong `issues/`; evidence được ghi trong từng issue.

## Evidence

- `cargo test --workspace`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `cargo fmt --all -- --check` và `git diff --check`: PASS.
- Frontend: `npm test` PASS (10 files, 37 tests); `npm run build` PASS.
- Production qualification: `.scratch/provider-runtime-manager/evidence/managed-startup.json` ghi `/ready` 200, bốn native resource được accounting, diagnostic đúng revision, shutdown exit 0 và unload được acknowledge.
- Native readiness và TTS delivery: các JSON/stdout/stderr trong `.scratch/provider-runtime-manager/evidence/`.
- Review Standards/Spec thủ công hoàn tất; đã sửa các phát hiện về ready badge, content fingerprint, HTTP mapping và native-thread cleanup. Lần chạy review hai agent tự động không khả dụng vì giới hạn sử dụng của sản phẩm.
- Strict Clippy còn các cảnh báo baseline ngoài phạm vi; không còn cảnh báo mới thuộc thay đổi này.
