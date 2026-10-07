# 18: Báo cáo calibration và tải pilot có thể tái lập

**What to build:** Operator chạy evaluation/benchmark qua production runtime và nhận report có trial accounting, confidence bounds và latency cho đúng phạm vi pilot.

**Blocked by:** 10: Observe giọng ESP32 qua Voice WS.

**Status:** ready-for-agent

- [ ] Tái dùng production manager/worker/preprocessing. Corpus operator nằm ngoài server, có consent; tách theo lần/phiên thu và preregister protocol, số trial, stopping rule trước evaluation.
- [ ] 1:N dùng một utterance mới qua toàn candidate set; wrong identity tính genuine failure và misidentification, pure rejection báo riêng. 1:1 FAR/FRR riêng; không nhân hoặc chạy lại clip để tăng trial.
- [ ] Bốn cận trên exact binomial một phía 95%: FAR ≤ 1% mỗi đường, genuine failure 1:N ≤ 10%, FRR 1:1 ≤ 10%; không tuyên bố đồng thời 95%. Fixtures độc lập kiểm 298/299, zero/all-error cases.
- [ ] Báo tất cả attempts/exclusions, busy/timeout/runtime, short/quality, replay/overlap; tuning sau held-out yêu cầu held-out mới. Thiếu independence/evidence giữ preliminary.
- [ ] Đo 1 active WS + 1 native enrollment đã Ready, cùng ASR/TTS thật: inference p95 ≤ 200 ms/cửa sổ 4 giây, gate wait p95 ≤ 500 ms. Báo queue/quality/inference và ASR/barrier riêng.
- [ ] Pin máy, model revision, threads, worker/queue, workload, exact sets/Voiceprint revisions/audio, corpus/protocol versions. Report privacy-safe có PASS/FAIL/NOT_RUN thật; thiếu dữ liệu không fabricate qualification.
