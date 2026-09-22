# 07: Compile-time provider registry

**What to build:** Thay startup large `match` bằng compile-time VadFactory/AsrFactory registry, để typed configuration chọn provider built into binary và provider nhận Resolved Model thay vì direct path.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT; 06: Model Preparation lifecycle.

**Status:** ready-for-agent

- [ ] Provider Registry đăng ký factory compile-time; adapter không build vào binary bị reject lúc startup và thay đổi selection cần restart.
- [ ] Typed config chọn adapter, Logical Model Identity và runtime options dưới provider-specific table; bỏ direct encoder/decoder/joiner/tokens paths.
- [ ] VadFactory/AsrFactory build Silero/Zipformer qua cùng factory seam, inject Resolved Model theo artifact role và không tự acquire model.
- [ ] Không hỗ trợ `dlopen`, ABI plugin, hot install, runtime code discovery hoặc dynamic provider plugin.
- [ ] Tests cover unknown/uncompiled adapter, typed config selection, Resolved Model role validation và migration Silero/Zipformer không đổi SessionActor/worker boundary.
