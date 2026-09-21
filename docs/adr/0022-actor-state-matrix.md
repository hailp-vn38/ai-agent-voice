# ADR 0022 — Actor xử lý message bằng state matrix

## Status
Accepted

Actor áp dụng matrix Ready/Listening/Processing/Speaking: chỉ Listening accept binary audio; `listen:start` ở Listening reset collector, còn ở Processing/Speaking hủy turn rồi vào Listening; `listen:stop` ngoài Listening bị ignore; `abort` idempotent. Valid message ở sai phase chỉ metric/log có kiểm soát và ignore; malformed protocol được phân loại riêng.
