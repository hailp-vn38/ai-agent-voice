# ADR 0035 — Uplink decoder sống theo Voice Session, không theo capture

`UplinkOpusDecoder` được tạo cho Uplink Audio Stream của một Voice Session và không reset ở `listen:start`, `listen:stop` hay `abort`; các event đó chỉ reset/discard `ManualCapture`. Voice Protocol V1 raw Opus không khai báo stream discontinuity ở conversational-turn boundary, còn reference client giữ microphone encoder liên tục, nên reset decoder theo capture có thể phá codec state đồng bộ hai đầu.
