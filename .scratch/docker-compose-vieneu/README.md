# Local production Docker Compose with VieNeu

The deployment uses the repository production Dockerfiles and `compose.yaml`,
`compose.prod.yaml`, `compose.vieneu.yaml`, plus `compose.runtime.yaml` here.
The local helper selects the isolated Docker engine and passes the secret env file.

```sh
bash .scratch/docker-compose-vieneu/manage.sh ps
bash .scratch/docker-compose-vieneu/manage.sh logs --tail 50 server vieneu
bash .scratch/docker-compose-vieneu/manage.sh up -d --no-build
bash .scratch/docker-compose-vieneu/manage.sh stop
```

LAN web: http://192.168.1.157:8080/ ; local web: http://127.0.0.1:8080/ . Use the existing Admin token from `config.toml`.
The inner web port binds all interfaces inside the engine container; its outer
host publications bind loopback and LAN IP `192.168.1.157` on port 8080.

`config.toml` selects `vieneu_default`; `docker/config.prod.toml` is its copied
Docker version, with server bind, public WebSocket URL, ONNX library and VieNeu
service URL adjusted. Compose mounts it read-only at `/app/config.toml`.
Credentials stay in ignored configuration/env/key files. The encryption key file
is mounted read-only and excluded from build context.

Database and copied native model files live under
`/mnt/storage/ai-agent-voice/docker/prod/`. This deployment uses its own new SQLite
database; it does not migrate the host server database. Private pre-change config
backups are in that directory's `config-backups/`.

The system Docker storage is outside `/mnt/storage`. An isolated engine runs in
the `voice-agent-storage-engine` bootstrap container and stores images, BuildKit
cache and containerd data under `/mnt/storage/ai-agent-voice/docker/engine/`.
Existing system services continue using their original Docker engine.
If the bootstrap container is stopped, start it with the system engine:

```sh
env -u DOCKER_HOST docker start voice-agent-storage-engine
```

VieNeu uses the pinned upstream commit `85344322b7258b4e25479b692e8e3396baf9db34`,
CPU ONNX fp32, and the previously downloaded Hugging Face model cache. The root
config's host VieNeu URL continues pointing at the earlier local Python service
on port 18090; the Docker config uses `http://vieneu:8000/v1`.

`smoke.py` exercises the real public web/health/ready routes, creates a VieNeu
Provider Instance with a sealed credential through the Admin API, and requests
real Vietnamese TTS twice with a six-second idle gap. It writes `smoke.json` and
`vieneu-docker.wav`. Running it again creates another diagnostic Provider Instance.
