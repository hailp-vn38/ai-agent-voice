# 04: Cold preparation dùng admission nguyên tử

**What to build:** Prepare, admission hoặc prewarm chỉ dựng native runtime khi voice pipeline và native enrollment đều rảnh; cold switch busy không làm đổi profile hiện tại.

**Blocked by:** 03: Giới hạn toàn pipeline và báo busy trên WS.

**Status:** resolved

- [x] Manager materialization acquire cùng admission state nguyên tử; không check-rảnh-rồi-load, không tạo model manager/cache riêng.
- [x] Giữ quyền qua build/readiness/warmup tới terminal acknowledgement; HTTP timeout/cancel không trả sớm; capture/enrollment mới busy trong thời gian đó.
- [x] Áp dụng startup/background prewarm, prepare, diagnostics, admission và switch; max_parallel_loads không thay gate. Ready backing runtime hot acquire giữ capacity hiện có.
- [x] Cold target khi session giữ slot trả busy không build, giữ profile; operator prepare trước session. Enrollment admission hook có sẵn để sample slice sử dụng.
- [x] Public prepare/WS race tests qua manager chứng minh atomic exclusion, hot reuse và late completion; Provider UI/API phân biệt busy với permanent unavailable.

## Answer

Cold materialization now consumes the ticket 03 pilot envelope atomically. `ProviderRuntimeManager`
holds the shared `PilotAdmission` (attached in `AppState::with_runtime_manager`), and
`acquire_version_until` claims `pilot.try_cold()` inside the same registry critical section that
decides to start a build. If voice or enrollment is active the build never starts and the caller
gets `RuntimeError::Busy` (mapped to `provider_runtime_busy`, distinct from permanent
`provider_runtime_unavailable`) while the old profile is retained. There is no check-then-load
window and no second loading cache; `max_parallel_loads` still only bounds loader concurrency.

The cold permit is moved into the detached build task and released in `complete` at the terminal
acknowledgement, before the terminal state is published. Build, readiness and warmup therefore all
run under the envelope, and a caller that times out or is cancelled cannot release it early — new
voice/enrollment stays busy until the native attempt reaches Ready/Failed/Quarantined. Ready
backing runtimes still hot-acquire without touching the pilot. Because startup, background prewarm,
explicit prepare, diagnostics, admission and managed switch all funnel through
`acquire_version_until`, every cold path is covered by one change. The enrollment hook
(`PilotAdmission::try_enrollment`, ticket 03) remains available on the shared envelope for the
sample slice.

Validation: `cargo test -p voice-agent-server --test provider_runtime_manager` passed 23/23,
including four new tests proving atomic exclusion (voice holds → cold returns Busy, materializer
untouched), the envelope held until terminal ack (voice sees busy mid-build, admitted after Ready),
hot reuse while voice holds, and late completion after a waiter timeout. `--lib` and the
provider_materializer/registry/load_plan/template_snapshot/admission_snapshot/device_admission
integration suites passed; `cargo clippy --all-targets --all-features` adds no new warnings and
`cargo fmt --all --check` passes. `session_profile` has one pre-existing failure unrelated to this
change (reproduced on the base commit). No real-model qualification or performance claim is made.
