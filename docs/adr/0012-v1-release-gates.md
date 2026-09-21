# ADR 0012 — V1 chỉ hoàn thành qua ba release gate độc lập phần cứng

## Status
Accepted

Definition of Done V1 yêu cầu đồng thời Gate A Protocol Conformance (unit, integration, fixture và Reference Client), Gate B Independent Client Interoperability, và Gate C Real Voice Pipeline. Firmware `78/xiaozhi-esp32` chạy trên HIL Reference Profile là Reference Hardware Compatibility Test bổ sung; nó xác nhận interop firmware nhưng không là điều kiện duy nhất để tuyên bố tương thích protocol.
