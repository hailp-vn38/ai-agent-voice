# Docker: test và production

Hai container `web` (Vue static + Nginx) và `server` (Rust/Axum). Web proxy HTTP và
WebSocket tới `server:8000`; trình duyệt dùng cùng origin. SQLite chỉ có một server
sở hữu, không scale replicas. Production không compile `qualification-providers`.

## Công cụ quản lý

Sau khi chuẩn bị storage, thư mục và cấu hình như bên dưới, dùng:

```bash
bash scripts/compose.sh prod build
bash scripts/compose.sh prod up
bash scripts/compose.sh prod status
bash scripts/compose.sh prod logs server
bash scripts/compose.sh prod restart server
bash scripts/compose.sh prod down
bash scripts/compose.sh test run
```

`build`, `up`, `restart`, `logs`, `status` nhận lựa chọn `server` hoặc `web`;
bỏ lựa chọn để thao tác cả stack. `up` chờ readiness (prod tối đa 30 phút,
test 120 giây); có thể build/pull image thiếu nên luôn kiểm tra storage trước.
`restart` dùng container hiện có; sau khi sửa cấu hình Compose hoặc build image
mới, dùng `up` để áp dụng thay đổi. `config` chỉ validate, không in secrets;
`check` chỉ kiểm tra storage. Xem các lệnh bằng `--help`.

`down` giữ dữ liệu prod và volume test. `test run` chạy gate hiện có và xóa
volume test khi kết thúc; không chạy cùng lúc với một stack test cần giữ dữ liệu.
Công cụ dùng project cố định `voice-agent-prod`/`voice-agent-test`, chạy được từ
working directory bất kỳ và truyền nguyên exit code lỗi. Nó không tự sao chép
config, thay token, sửa ownership hoặc di chuyển Docker storage.

## Storage trước khi build

Cargo giữ target `/mnt/storage/ai-agent-voice/target` trong `.cargo/config.toml`.
Docker build dùng BuildKit cache tại cùng đường dẫn **bên trong container**;
nơi cache thật nằm trên host phụ thuộc Docker/BuildKit storage. Không bind host
target vào image build và không dùng chung target giữa host với runner Docker.

**Đặt Docker storage trên `/mnt/storage/` trước khi build hoặc pull image.**
Kiểm tra không tải image:

```bash
python3 scripts/check-docker-storage.py
```

Docker daemon cần `data-root` dưới `/mnt/storage/`, ví dụ
`/mnt/storage/docker`. Với containerd image store, `root` trong
`/etc/containerd/config.toml` cũng phải nằm dưới `/mnt/storage/`, ví dụ
`/mnt/storage/containerd`. Chỉ đổi Docker `data-root` không chuyển containerd store.
Tham khảo [Docker daemon](https://docs.docker.com/engine/daemon/) và
[containerd image store](https://docs.docker.com/engine/storage/containerd/).

Việc chuyển storage của daemon hiện có cần cửa sổ bảo trì: dừng workloads và
daemon, sao chép dữ liệu giữ ownership/permissions, sửa cấu hình rồi khởi động và
kiểm tra lại. Không xóa store cũ trước khi xác nhận image/volume/container còn đủ.
Repo không tự sửa cấu hình hệ thống hoặc di chuyển Docker data.
Checker chỉ hỗ trợ daemon local và containerd config chuẩn; nếu dùng custom
containerd `--config` hoặc builder riêng, kiểm tra storage của chúng trực tiếp.

Chuẩn bị thư mục host bằng user có quyền ghi `/mnt/storage/`:

```bash
mkdir -p /mnt/storage/ai-agent-voice/docker/prod/{data,models}
mkdir -p /mnt/storage/ai-agent-voice/docker/test-target
```

## Production

```bash
cp docker/config.prod.example.toml docker/config.prod.toml
cp docker/secrets.env.example docker/secrets.env
chmod 600 docker/config.prod.toml docker/secrets.env
```

Sửa token voice/admin, provider key/model và `server.public_ws_url` trong TOML.
Không có interpolation environment tự động trong TOML. `secrets.env` cung cấp
khóa mã hóa `VOICE_CREDENTIAL_KEY_<version>` cho credential Provider/MCP lưu trong database; các biến credential từng resource vẫn là nguồn dự phòng. Xem [hướng dẫn credential mã hóa](admin-managed-provider-mcp-credentials.md).
Không đưa secret vào Docker build args hoặc `VITE_ADMIN_TOKEN`; dùng form kết nối
sẵn có của web. Hai file thật được Git và Docker build context bỏ qua.

Runtime đóng gói [ONNX Runtime CPU Linux 1.23.2](https://github.com/microsoft/onnxruntime/releases/tag/v1.23.2);
Sherpa dùng static libraries từ dependency đã pin trong Cargo.lock. Model thiếu sẽ
được provider tải khi startup hoặc materialize, rồi lưu trong `/app/models`.
Boot đầu cần network, đủ disk/RAM và có thể lâu; healthcheck dành 30 phút cho startup.
Healthcheck không tự restart container unhealthy. Model không được đóng gói trong image.

```bash
export APP_UID="$(id -u)" APP_GID="$(id -g)"
python3 scripts/check-docker-storage.py
docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml config --quiet
docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml build
docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml up -d
```

Thư mục data/models và TOML phải truy cập được bởi `APP_UID:APP_GID`; mặc định
1000:1000. Mở `http://127.0.0.1:8080`, nhập admin token vào form. Server không
publish port ra host. Để truy cập từ trusted LAN, đặt `WEB_BIND` là địa chỉ LAN của
host và đổi `public_ws_url` tương ứng; `WEB_PORT` mặc định 8080.

Prod giữ data/models ở `/mnt/storage/ai-agent-voice/docker/prod/`, tách dữ liệu host
hiện có. Backup SQLite nhất quán trước upgrade có migration; có thể stop server
rồi backup toàn thư mục data. `stop_grace_period=30s` lớn hơn grace mặc định 15s;
nếu tăng `shutdown.grace_ms`, tăng thời gian Compose tương ứng. Server nhận SIGTERM
trực tiếp. Redeploy sẽ ngắt Voice Sessions hiện có.

Mẫu dùng Silero/Gipformer/ZeroTTS và OpenAI; không tự mang Kokoro G2P, GPU runtime
hoặc calibration riêng của host vào image. Nếu chọn các adapter cần chúng,
cần bổ sung runtime Linux và cấu hình/mount tương ứng. Profile hỗ trợ vẫn là
trusted LAN/VPN; Internet không thuộc V1 deployment profile.

## Test tự động

Yêu cầu Docker Compose v2 có `up --wait`, Python 3.11+ và Bash. Nếu user chưa có
quyền Docker socket, chạy bằng tài khoản có quyền Docker.

```bash
bash scripts/test-docker.sh
```

Script kiểm tra storage trước khi build, dùng project `voice-agent-test`:

- Rust fmt/clippy và test mặc định, rồi test với `qualification-providers`.
- Vue Vitest, typecheck và production build.
- Chạy qualification binary qua production entrypoint với provider deterministic;
  không cần model, credential thật hoặc hardware.
- Smoke qua Nginx: readiness, SPA fallback, admin auth, WebSocket upgrade bị từ
  chối khi thiếu bearer và ghi SQLite. Restart server rồi kiểm tra row còn tồn tại.

Qualification server dùng filesystem chỉ đọc và data volume riêng. Built-in CAM++
optional không thể tạo thư mục model nên unavailable ngay, không tải model thật;
smoke này không đánh giá speaker inference. Test không publish port ra host;
named volume SQLite được xóa khi teardown kể cả failure. Cargo test target giữ tại
`/mnt/storage/ai-agent-voice/docker/test-target` để reuse; runner chạy root nên các
file target có thể thuộc root. Không chạy hai test script đồng thời vì cùng project
name và target. Exit code bất kỳ gate lỗi sẽ làm toàn script fail.

Đây là kiểm tra đóng gói và regression deterministic. Luồng audio/provider thật,
full WebSocket dialogue và hardware calibration vẫn dùng các gate riêng của repo.
Qualification image không được deploy production. Test thủ công với provider thật
nên dùng image production, cấu hình và thư mục dữ liệu riêng.

Kiểm tra nhanh cấu hình mà không build:

```bash
docker compose -f compose.yaml -f compose.prod.yaml config --no-env-resolution --quiet
docker compose -f compose.test.yaml --profile checks config --quiet
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests
```
