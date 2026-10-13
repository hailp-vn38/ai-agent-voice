ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG BASE_IMAGE
ARG SOURCE_SHA256
LABEL agent.egress.policy="1"
LABEL agent.egress.base="${BASE_IMAGE}"
LABEL agent.egress.source="${SOURCE_SHA256}"
COPY proxy.py /app/proxy.py
USER 65532:65532
ENV PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["python", "-I", "/app/proxy.py"]
