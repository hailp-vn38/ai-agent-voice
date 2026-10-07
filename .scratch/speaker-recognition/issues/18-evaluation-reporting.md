# 18: Báo cáo calibration và tải pilot có thể tái lập

**What to build:** Operator chạy evaluation/benchmark qua production runtime và nhận report có trial accounting, confidence bounds và latency cho đúng phạm vi pilot.

**Blocked by:** 10: Observe giọng ESP32 qua Voice WS.

**Status:** resolved

- [ ] Tái dùng production manager/worker/preprocessing. Corpus operator nằm ngoài server, có consent; tách theo lần/phiên thu và preregister protocol, số trial, stopping rule trước evaluation.
- [ ] 1:N dùng một utterance mới qua toàn candidate set; wrong identity tính genuine failure và misidentification, pure rejection báo riêng. 1:1 FAR/FRR riêng; không nhân hoặc chạy lại clip để tăng trial.
- [ ] Bốn cận trên exact binomial một phía 95%: FAR ≤ 1% mỗi đường, genuine failure 1:N ≤ 10%, FRR 1:1 ≤ 10%; không tuyên bố đồng thời 95%. Fixtures độc lập kiểm 298/299, zero/all-error cases.
- [ ] Báo tất cả attempts/exclusions, busy/timeout/runtime, short/quality, replay/overlap; tuning sau held-out yêu cầu held-out mới. Thiếu independence/evidence giữ preliminary.
- [ ] Đo 1 active WS + 1 native enrollment đã Ready, cùng ASR/TTS thật: inference p95 ≤ 200 ms/cửa sổ 4 giây, gate wait p95 ≤ 500 ms. Báo queue/quality/inference và ASR/barrier riêng.
- [ ] Pin máy, model revision, threads, worker/queue, workload, exact sets/Voiceprint revisions/audio, corpus/protocol versions. Report privacy-safe có PASS/FAIL/NOT_RUN thật; thiếu dữ liệu không fabricate qualification.

## Answer

Implemented on branch `speaker/18` (commit `e28efed`).

The deterministic, CI-verifiable core of the pilot report now lives in
`crates/voice-agent-server/src/speaker_evaluation/`:

- `binomial.rs` — exact one-sided 95% (Clopper-Pearson) upper bounds. Closed forms for
  `n == 0` (no bound), `k == n` (bound 1.0) and `k == 0` (`1 - 0.05^(1/n)`); every other case is
  solved by bisection on the exact binomial CDF in log space. `min_zero_error_trials` gives the
  298/299 boundary for 1% and 29 for 10%.
- `mod.rs` — `build_report(ReportInput) -> EvaluationReport`. Trial accounting separates
  misidentification from pure rejection and reconciles every attempt against exclusions
  (busy/timeout/runtime/short/quality/replay/overlap). Four checks (FAR 1:N, genuine failure 1:N,
  FAR 1:1, FRR 1:1) each get their own bound and `Pass`/`Fail`/`Preliminary`/`NotRun` status. The
  overall `QualificationStatus` is `Qualified` only from complete, uncontaminated evidence; a
  duplicated `sample_code`, a held-out corpus tuned after freeze, an evaluation session that
  overlaps enrollment, an unpinned scope or a failed latency run all block qualification instead
  of fabricating it. `assert_privacy_safe` rejects audio, PCM, embeddings, transcripts,
  credentials, absolute paths, file-like values, hex digests and base64 blobs before a report
  leaves the process.
- `tests.rs` — unit tests: independent binomial fixtures (299/298 zero-error, `n == 0`, all-error),
  misidentification vs pure rejection split, duplicate-trial rejection, held-out/session/scope
  blocking, latency failure, and privacy-safe/forbidden-material cases.
- `tests/speaker_evaluation.rs` — end-to-end test that drives a real `SpeakerRuntime` through a
  provider-runtime `ResourceLease` and scores with the production `ObservePlan`, then feeds the
  classified trials into `build_report`. No native model or network is needed.

Files changed: `src/speaker_evaluation/{mod,binomial,tests}.rs` (new),
`tests/speaker_evaluation.rs` (new), `src/lib.rs`, `src/session/mod.rs` (export
`OBSERVE_VERIFY_THRESHOLD`), `src/benchmark/stats.rs` (`MetricSummary` gains `PartialEq` so it can
be embedded in the report).

Tests:

```
cargo test -p voice-agent-server --lib speaker_evaluation   # 17 passed
cargo test -p voice-agent-server --test speaker_evaluation  # 1 passed
cargo test -p voice-agent-server --lib                       # 220 passed (no regression)
cargo clippy -p voice-agent-server --tests                   # no new warnings
```

Acceptance mapping:

- Bullets 2, 3, 4 and 6 are covered by the module and its tests: one utterance per trial through
  the full candidate set, wrong-identity misidentification reported separately from pure
  rejection, four independent one-sided bounds with independent fixtures, full
  attempts/exclusions accounting, held-out-after-tuning invalidation, and a privacy-safe
  PASS/FAIL/NOT_RUN report that never fabricates qualification from missing data.
- Bullet 1 (production scoring) is covered by the integration test, which uses the real
  provider-runtime lease, `SpeakerRuntime` worker and `ObservePlan` scoring; the real CAM++
  extractor and operator corpus stay outside CI.
- Bullet 5 (live 1 WS + 1 native enrollment Ready latency with real ASR/TTS) is operational and
  stays `NOT_RUN` until run on the pilot host; the report already models the split
  queue/quality/inference vs ASR/barrier summaries and the 200ms/500ms p95 targets.

Remaining gap: the operator-facing CLI that boots the full app runtime, resolves the exact set
from the catalog and walks an external corpus manifest is not included; the report builder and
the production scoring seam it needs are in place, and the live-latency scenario is documented as
`NOT_RUN` evidence rather than claimed.
