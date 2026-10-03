# ADR 0068 — Qualification Providers là compile-time production-path adapters

## Status

Accepted

Mandatory Qualification dùng feature server `qualification-providers`, mặc định tắt, để compile bốn adapter deterministic `qualification_vad`, `qualification_asr`, `qualification_llm` và `qualification_tts` vào đúng binary `voice-agent-server`. Qualification build vẫn chạy production `main`, typed configuration, ProviderConfigValidator, database desired configuration, Template binding, process restart, Provider Load Plan, Runtime Catalog, public Provider Test API và Voice Session. Không có runtime switch, injected `ProviderSet`/`AppState`, network, credential, SecretRef, model acquisition hoặc randomized output. Default/release build không chứa hay advertise các adapter này; CI chứng minh cả absence ở default build và presence ở qualification build.

Quyết định này ưu tiên một production-process wiring gate deterministic, không phụ thuộc external environment, thay cho hai lựa chọn không đáp ứng đồng thời mục tiêu: dùng test constructor in-process thì không chứng minh process/startup path, còn ép default release artifact dùng provider thật sẽ biến Mandatory Qualification thành environment-dependent smoke test. Report có thể ghi `server_build_profile = qualification` như observation không nhạy cảm, nhưng metadata đó không phải security authority hay bằng chứng độc lập về binary provenance.
