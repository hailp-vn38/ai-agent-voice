# 10: Docker: git trong image và mount deploy key

Status: ready-for-human
Blocked by: 08
Spec: ../spec.md (user story 11)
Thiết kế: docs/wiki-desktop/git-sync.md#docker

## Mục tiêu

Container server chạy được git qua SSH với deploy key, không cần `HOME` hay cấu hình git toàn cục.

## Phạm vi

- `docker/server.Dockerfile` stage `runtime`: thêm `git openssh-client`.
- `compose.prod.yaml`: mount `./docker/wiki-deploy-key` và `./docker/github-known-hosts` read-only, `create_host_path: false`. Thêm hai file vào `.gitignore` nếu nằm trong repo.
- Hướng dẫn trong `docs/knowledge/` (hoặc `docs/providers/` nếu hợp hơn): tạo deploy key ed25519, bật "Allow write access" trên GitHub, `ssh-keyscan github.com` và đối chiếu fingerprint công bố, quyền file, bật `[wiki.sync]`.

## Tiêu chí xong

- [ ] Build image qua đường test Docker hiện có (`compose.test.yaml`) PASS; `git --version` và `ssh -V` chạy được trong container với user `1000`.
- [ ] Smoke trong container với bare repo qua `file://` (không cần mạng): sync chạy, không báo `dubious ownership`.
- [x] `git diff --check` PASS.

## Comments

- Added `git openssh-client` to the runtime image, read-only deploy-key and known-host mounts with `create_host_path: false`, ignored host secret paths, operator instructions, and a Compose smoke service that runs git operations as UID 1000 without `HOME` against a local bare repository.
- `scripts/agent-test.sh run -- docker compose -f compose.test.yaml --profile checks config --quiet`: PASS; `scripts/agent-test.sh run -- sh -n scripts/docker/wiki-git-smoke.sh`: PASS; `scripts/agent-test.sh run -- git diff --check`: PASS. Production Compose config also parsed successfully with a temporary empty ignored `docker/secrets.env`.
- Container build/smoke is NOT_RUN: Docker CLI is installed, but the Docker daemon socket is unavailable (`Cannot connect to the Docker daemon at unix:///Users/lamphuchai/.docker/run/docker.sock`). The image/runtime acceptance checks remain for a host with a running daemon.
