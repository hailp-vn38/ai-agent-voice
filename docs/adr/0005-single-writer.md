# ADR 0005 — Single WebSocket writer

## Status
Accepted

## Decision
Chỉ một task được phép gọi WebSocket send. Mọi module khác gửi message qua các bounded control/audio queue do writer sở hữu.

## Consequences
Ordering dễ kiểm soát, giảm race giữa JSON control và binary audio, backpressure rõ ràng.
