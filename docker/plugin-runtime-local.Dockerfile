# Local host-built binaries require the same glibc generation as the build host.
FROM python:3.12-slim-trixie
COPY --from=ghcr.io/astral-sh/uv@sha256:4de5495181a281bc744845b9579acf7b221d6791f99bcc211b9ec13f417c2853 /uv /uvx /usr/local/bin/
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/plugin-runtime"]
