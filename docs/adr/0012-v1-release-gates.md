# ADR 0012 — V1 chỉ hoàn thành qua ba release gate

## Status
Accepted

Definition of Done V1 yêu cầu đồng thời Gate A CI-compatible (unit, integration, mock và fixture), Gate B Xiaozhi firmware-compatible (HIL trên Firmware Baseline và HIL Reference Profile), và Gate C real AI pipeline smoke test. Fake client chỉ chứng minh Gate A, không đủ để tuyên bố tương thích firmware hay hoàn thành V1.
