# 10: Docker: git trong image và mount deploy key

Status: ready-for-agent
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
- [ ] `git diff --check` PASS.
