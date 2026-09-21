# ADR 0022 — Actor xử lý message bằng state matrix

## Status
Accepted

Actor áp dụng matrix Ready/Listening/Processing/Speaking: chỉ Listening accept binary audio. Phase 3 enter Processing ngay tại `listen:stop` (Manual) hoặc `SpeechEnd` (Auto), trước ASR finalization; Processing drop audio để không tạo utterance song song. Khi Phase 3 terminal sau STT/CompletedSilent/failure, Manual về Ready và Auto về Listening. `listen:start` ở Listening reset collector, còn ở Processing/Speaking hủy turn rồi vào Listening; `listen:stop` ngoài Listening bị ignore; `abort` idempotent. Valid message ở sai phase chỉ metric/log có kiểm soát và ignore; malformed protocol được phân loại riêng.
