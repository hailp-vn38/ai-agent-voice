# 02: Bind Speaker Provider tùy chọn vào Template

**What to build:** Admin chọn hoặc unlink Speaker Provider trên Template, giữ nguyên core fallback và default assignment của các Template hiện có.

**Blocked by:** 01: Tạo và kiểm tra Speaker Provider CAM++.

**Status:** resolved

- [x] UI/API/DB/runtime-profile resolve hỗ trợ optional Speaker slot đúng type; không speaker default hoặc implicit fallback.
- [x] Core absent bindings giữ server fallback; explicit broken core binding vẫn fail; first assignment không dùng count bốn-slot sai khi thêm Speaker.
- [x] Bind/unlink CAS Template revision; provider usage/conditional deletion đúng; binding cold lưu desired, không build trong transaction.
- [x] Public API/profile regression kiểm partial core, core + Speaker, sai type, assignment đầu, unlink và no-restart semantics; cập nhật Template UI/API documentation.

## Answer

Implemented on branch `speaker/02`, merged to `integration/speaker-recognition` (commits f69147d, 44af30e, aad784a).

- `EffectiveProviderBindings.speaker: Option<String>` and `ResolvedAgentRuntimes.speaker: Option<Arc<SpeakerRuntime>>` added; `RuntimeCatalog::resolve` looks up the bound speaker (missing key → `Unknown{kind:"SPEAKER"}`, absent → `Ok(None)`).
- `ConfiguredTemplateProfile` / `from_assignment_internal` thread the optional speaker slot; no deployment default and no implicit fallback for speaker.
- `assign_template` promotes the first assignment unless an explicit core binding is broken (absent core slots keep the server fallback); speaker is never counted against the 4-slot completeness.
- Bind/unlink already CAS on the Template revision and record provider usage for conditional deletion; verified by tests.
- Tests: `tests/admin_api.rs` (+168 lines: core / core+speaker / wrong type / first assignment / unlink / conditional provider deletion), `provider_template_snapshot.rs`, and lib tests `session::runtime_profile` + `session::profile` (6 speaker tests). All green.
- Docs: `docs/speaker-provider.md` documents the optional slot, CAS, no-fallback, and no-restart semantics.
