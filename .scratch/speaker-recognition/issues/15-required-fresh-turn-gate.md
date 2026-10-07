# 15: Required xác minh mới ở từng voice turn

**What to build:** Agent Required chỉ accept lượt nói được phép: lần đầu nhận diện1:N, các lượt sau verify1:1 identity khóa, và không thể dùng Detect để vượt gate.

**Blocked by:** 10: Observe giọng ESP32 qua Voice WS, 12: Quan sát và approve Device tool contracts, 14: Reload calibration và kiểm exact candidate sets.

**Status:** ready-for-agent

- [ ] Guard enable/admission bằng ready binding/runtime/grants + exact qualified evidence; deterministic qualified fixtures chỉ trong qualification build, không production bypass.
- [ ] Common accept boundary chờ ASR nonempty + fresh speaker pass + History Barrier; exactly-once STT/history/LLM, identity/generation/runtime stale checks trước semantics.
- [ ] Lock Speaker perWS, mỗi lượt fresh1:1, short fail không inherited pass, change speaker reconnect; compare all compatible Agent candidates trước Template grant.
- [ ] Denied/unknown/ambiguous/short/unavailable zero STT/history/archive/LLM/tool/TTS; Required Detect audio-required denied; Device abort và same-session barge-in giữ protocol.
- [ ] Bounded cleanup/timeout/queue failure, mismatch counter3 với1008; busy/short/runtime không count. Actor checks security epochs trước accept/LLM continuation/tool.
- [ ] Agent UI, speaker status và public WS qualification kiểm result reorder, reject side effects, history/writer barrier, per-turn identity, stale outputs và control responsiveness.
