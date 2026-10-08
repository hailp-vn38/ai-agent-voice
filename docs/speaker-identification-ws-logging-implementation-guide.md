# Speaker Identification V1 — Implementation Guide (Revised)

> **Project:** `hailp-vn38/ai-agent-voice`  
> **Baseline:** `main`, reviewed 2026-10-08; confirm HEAD before implementation.  
> **Revision:** v2 — incorporates Ponytail Review and Codebase Design review.  
> **Deliverable:** Coding-agent implementation guide, **not** a claim that repository code has been changed.

## Tóm tắt cập nhật sau review

Bản v2 thay thế hướng dẫn cũ, ưu tiên **đơn giản hóa** nhưng giữ nguyên tính an toàn của chế độ `required`:

- **Không cần `calibration.json`** trong luồng đăng ký và nhận dạng thông thường (`observe`); vẫn giữ calibration và fail-closed cho `required`.
- **WebSocket log rõ hai loại xác thực:** `bearer_auth_enabled` cho WS Bearer và `speaker_authentication_enabled` cho xác thực bằng giọng nói. `speaker_recognition_enabled` và `speaker_recognition_ready` được ghi riêng, kể cả khi Agent tắt nhận dạng.
- **SessionActor sở hữu kết quả theo từng turn:** ghép ASR + Speaker với timeout hữu hạn, không truyền danh tính cũ, không ghi nhầm kết quả đến muộn.
- **Template Switch tuyệt đối không giữ Speaker Plan cũ** khi chuyển sang Template thiếu Provider hoặc không tương thích; riêng `required` không được tự hạ quyền.
- **UI phân biệt Configured / Provider Ready / Session Ready**, không suy luận trạng thái WS từ thông tin database đơn thuần.
- **Tận dụng interface/module hiện có**, không tạo thêm coordinator, database, worker hay API chỉ để bật/tắt; giảm event và checklist trùng lặp.

Các mục dưới đây là hợp đồng kỹ thuật dành cho coding agent. **Chưa triển khai hay xác nhận test pass trong repository.**

## 0. Decisions and non-goals

**Target flow:** Web microphone -> Speaker Provider -> accepted capture -> speaker profile/embedding in SQLite -> enable in Agent and grant to Templates -> Device WebSocket -> speaker identification per audio turn -> optional turn-local LLM personalization.

1. `calibration.json` and calibration qualification are **not prerequisites for `observe`/identification**. Do not delete legacy calibration storage, reload route or `required` gate in this change.
2. Voice **identification** (`observe`, best-effort) is not voice **authentication/authorization** (`required`, gate). Neither identification nor a name in an LLM prompt gives Device/MCP/tool/private-data permissions.
3. Speaker Provider remains owned by **Template binding**; Agent owns policy and explicit Speaker grants. Reuse existing CAM++, `speaker_voiceprints`, SQLite, quick capture and session/actor pipeline. Do not add a new user table, duplicate worker, generic repository, coordinator, Redis or event bus.
4. UI standard path: toggle `off`/`observe`. An existing Agent in `required` stays explicitly in its legacy restricted state. No silent downgrade; changing this policy needs deliberate audited Admin action.
5. Always log whether **WS bearer authentication** is enabled and whether **speaker recognition** and **speaker authentication** are enabled. Log actual matched/unmatched outcomes for the correct audio turn, not speculative connection-time identity.
6. `required` must fail closed whenever runtime, DB, provider, grants, calibration qualification or required speaker guard cannot be resolved. Preserve existing policy behavior for text turns and WS status compatibility.
7. A Template switch in `observe` may yield recognition Ready or Unavailable, but **must never leave the previous Template's Observe plan active**.

## 1. Existing seams and file map

| Existing module | Relevant behavior / change |
|---|---|
| `crates/voice-agent-server/src/app/admin/speaker_quick.rs` | Keep WAV capture -> embedding -> `pending` voiceprint. A sample accepted for audio quality is **not** authenticated identity. |
| `.../app/admin/speaker_policy.rs` | Keep `off/observe/required`, CAS `If-Match` and audit. Ordinary UI operates only on `off/observe`. Qualification only gates `required`. |
| `.../session/speaker_observe.rs` | Reuse `resolve_observe_plan()`, cosine scoring, provider lease, `ObserveIdentity`, `ObserveDiagnostic`. Extend outcome/ownership; do not introduce a parallel identification engine. |
| `.../session/actor/observe.rs` | Today best-effort spawn, `observe_in_flight`, INFO logging before stale check; make completion an actor-owned turn outcome. |
| `.../session/actor/listening.rs` and `delivery.rs` | ASR final currently calls `commit_user_text()` and `begin_speech_delivery()` immediately for `observe`. Introduce bounded turn-local join **here**, preserving other modes. |
| `.../session/actor/speaker.rs`, `.../session/speaker_gate.rs` | Retain the `required` gate, speaker lock and fail-closed decisions. Do not use its `Accept` semantics for identification. |
| `.../session/switch_authority.rs`, `.../session/actor/tools/builtin_actions.rs` | Qualification/locked identity are for `required`; switching `observe` must resolve/install/clear the target's plan without stale carryover. |
| `.../app/websocket.rs` | Existing resolver returns `(None,None)` early for missing runtime/DB/provider. Policy must be known **before** those returns. Add admission and auth logs. |
| `apps/admin-web/src/components/agents/AgentSpeakerPolicy.vue` | Replace the common three-mode controls with simple toggle and grants; keep explicit legacy `required` warning. |
| `apps/admin-web/src/components/speakers/QuickSpeakerEnrollment.vue` | Reuse microphone capture wizard; show provisional enrollment result and Agent activation link. |

Known baseline: `OBSERVE_VERIFY_THRESHOLD=0.5` is hard-coded for Observe; `SPEAKER_GATE_MARGIN=0.15` belongs to Required. Neither is proof of production accuracy. The Observe resolver permits `pending` quick voiceprints; this is a provisional profile, **not** evidence for `required`.

## 2. Deep module interfaces: keep the existing seams

### 2.1 Admission result: one sum type, no boolean state explosion

Adapt `app/websocket.rs` and existing `SpeakerObserve` construction. One internal result describes the admitted Speaker mode; the exact names below are illustrative, not a requirement for an extra pass-through layer.

```rust
// Accepted states only. Required resolution failure returns Err, never a Ready-looking fallback.
enum SpeakerSessionSetup {
    Disabled,
    ObserveUnavailable(SpeakerSetupReason),
    ObserveReady(Arc<SpeakerObserve>),
    RequiredReady {
        observe: Arc<SpeakerObserve>,
        guard: Arc<SpeakerSwitchGuard>,
    },
}
```

- Derive `speaker_mode`, `speaker_recognition_enabled`, `speaker_recognition_ready`, and `speaker_authentication_enabled` **from the resolved policy/state**, rather than maintaining independent writable booleans. `speaker_authentication_enabled = (mode == required)`; this field means **voice-gated authorization**, **not** HTTP bearer/Device admission.
- Existing `resolve_observe_plan()` / `SpeakerObserve::observe()` remain the core interfaces. A thin typed admission result is acceptable if it removes duplicated early-exit decisions; do not introduce `SpeakerIdentityManager` or a second query facade.
- For `required`, every missing dependency or DB failure is `Err(AdmissionReject)` and rejects upgrade. The enum never represents `RequiredUnavailable` as a usable session.
- `observe` errors are represented as `ObserveUnavailable(reason)` and allowed to continue anonymously. Unknown policy values must not be silently treated as a weaker policy; reject corrupt/unsupported persisted modes.
- Reuse cancellation/revocation tokens and original Session ID. No DB query for each Opus frame.

### 2.2 Turn identification: existing diagnostic with actor ownership

Re-use `ObserveIdentity { operation_id, turn_id, generation }` and `ObserveDiagnostic` where possible. An extra `TurnSpeakerResult` type is needed **only if** it makes actor turn ownership or privacy invariants clearer; do not mirror every field from `ObserveDiagnostic` in a second DTO.

The worker owns extraction and cosine scoring. **SessionActor** owns valid result selection, bounded wait, deadline, cancellation, LLM context, and one terminal log event. `matched`, `unknown`, `ambiguous`, `insufficient_audio`, `unavailable`, `timeout` or `skipped` are result codes, **not** credentials.

## 3. SQLite, enrollment, and Agent API

Reuse schema and existing API; no new database or file-based voiceprint:

```text
speakers                          Speaker profile: key, name, description, enabled
speaker_voiceprints               provider, embedding_space, dims, vector(BLOB), status
agent_speaker_candidates          candidates approved for this Agent
agent_speaker_template_grants     allowed matching candidates per Template
agent_speaker_policies            mode + revision
Template provider binding          selects Speaker Provider; no Agent-level duplicate
```

Quick enrollment API (already present; verify exact request/response schema in HEAD):

```http
POST /api/admin/providers/{provider_key}/speaker-captures
Authorization: Bearer <admin_token>
Content-Type: audio/wav
If-Match: "<provider_revision>"

<PCM16 mono 16 kHz WAV>
```

```http
POST /api/admin/speakers/from-capture
Authorization: Bearer <admin_token>
Content-Type: application/json

{"capture_id":"...", "name":"Nguyễn Văn A", "description":"..."}
```

Web: **Speakers -> Add -> Select Speaker Provider -> Record -> Server validates -> Name -> Save -> Activate in Agent**. Capture acceptance only means adequate audio and embedding extraction. Voiceprint remains `pending`/provisional until explicitly validated under the existing advanced process. Do not auto-grant upon Save.

For normal identification: match only enabled Speakers with a voiceprint in the selected Provider's exact embedding space **and** explicit Agent + Template grants. Preserve WAV quality bounds, sample TTL, idempotency, data deletion/re-enroll and audit. Display name from `speakers.name`; no implicit user-account or tool permission.

Admin UI can keep existing `GET/PUT /api/admin/agents/{key}/speaker-policy` and Agent Speaker grant routes. Toggle OFF -> `mode=off`; ON -> `mode=observe` using current revision and `If-Match`. If current mode is `required`, **do not** map it through the toggle; show a restricted-state warning and require a separate explicit, audited action to change policy.

## 4. WebSocket admission and authentication logging

### 4.1 Order of operations

1. Validate header/protocol shape, configured WS Bearer token, Device ID/Client ID and enrollment routing **as current server already does**. Device authentication and speaker recognition are independent.
2. Emit structured `ws_auth` at the completed authentication/admission decision (including denial). Set `bearer_auth_enabled = !config.auth.token.is_empty()`, `bearer_auth_result = accepted|denied|not_required`, `device_admission_result = admitted|denied|pending_enrollment|unavailable|not_checked`, `result`, `reason_code`. Do not log bearer values or raw identity headers. On denial before session creation, correlate by existing HTTP tracing request span; `session_id` may be absent.
3. Pending Device enrollment WS routes do **not** pretend a voice-session Speaker policy was resolved; `ws_auth` records `pending_enrollment` and Speaker config is `not_applicable` unless that route explicitly builds a Voice Session.
4. For admitted Voice Session, resolve Agent/Template, look up policy **before** early returns for `resolved.speaker`, runtime lease, DB handle, or candidate set. Distinguish `off` from `observe` enabled-but-unavailable. Do not use `Option<SpeakerObserve>` alone to infer enabled state.
5. `off`: disabled, skip Speaker model and sample collection. `observe`: try Template runtime/lease/candidates; missing dependency -> unavailable(reason), WS still works anonymously. `required`: all prerequisite failures -> HTTP reject before upgrade, never install an ungated session.
6. Emit one `ws_speaker_config` per **Voice WS admission attempt after policy resolution**, with outcome `accepted|rejected`. For accepted connections include `session_id`; for failed upgrade include request correlation if Session ID was not yet assigned. Do not demand `ws_speaker_config` for requests denied before Agent policy can be read; `ws_auth` covers those.
7. Install immutable initial snapshot before audio arrives; revocation invalidates existing snapshot/session as in current security mechanism. Template switch is a deliberate boundary (§6).

### 4.2 Admission matrix

| Policy | Dependencies | WS result | recognition_enabled | recognition_ready | authentication_enabled |
|---|---|---|---:|---:|---:|
| `off` | any | accept | false | false | false |
| `observe` | Provider, lease, compatible grants present | accept, identify | true | true | false |
| `observe` | missing provider/candidates/lease or Speaker DB query failure | accept, anonymous + WARN | true | false | false |
| `required` | qualified, valid provider/lease/grants/guard | accept, preserve gate | true | true | true |
| `required` | any missing/error/qualification failure | **reject, fail closed** | true | false | true |

Preserve database startup dependency, Admin Bearer, existing WS Device admission and `required` text-turn contract. Do **not** reinterpret `observe` as a secure verifier.

## 5. Per-turn Observe and LLM personalization

### 5.1 SessionActor is the only turn finalization owner

- Decode uplink Opus to canonical 16 kHz PCM already used by VAD/ASR; reuse bounded Observe retention (at most 6 s); no extra raw audio storage.
- At utterance terminal, capture `operation_id + TurnId + generation + Template/profile revision` **before** spawning `SpeakerObserve::observe()`. The current implementation has `observe_in_flight`; if busy, empty audio or no ready plan, submit a **terminal non-match outcome** to the actor rather than silently returning.
- Worker completion returns via existing actor-owned completion channel (`gate_tx/gate_rx` can be adapted/shared with correct routing). One completion enters the actor, where `TurnId`, generation, profile/template revision and cancellation token are checked **before** a match is accepted or logged. Required diagnostics continue to feed `SpeakerGate`; Observe never authorizes.
- ASR final and identification may finish in either order. For `observe`, keep only the needed small pending state *within the current turn*; wait at most a **pilot 1.5 s** after ASR final if Observe hasn't completed, then start anonymous LLM. No unbounded queue, separate task manager or optional user config at this stage.
- If recognition returns `matched` in budget and still belongs to this turn, use identity in that turn's prompt. After timeout/unknown/ambiguous/busy/unavailable, proceed anonymous. Late results are DEBUG dropped and never alter the current or next turn, final log, history or tools.
- On `abort`, cancel, reconnect, policy revocation, Template switch, generation change and close: invalidate pending identity; release any wait. Do not accidentally overwrite a newer turn or history-barrier turn. Text `detect` uses anonymous identity; never reuse last matched Speaker.
- Do not introduce waits for `off`. Keep `required` gate's existing authorization timing and denial semantics; do not run Observe's fallback-anonymous branch under Required.

### 5.2 Recognition decision and output

- Reuse `ObservePlan::score()` and `enrollment::cosine()`; **do not** call SpeakerGate to grant permission.
- `matched` only if top similarity >= Observe threshold and (when multiple candidates) top-runner-up >= ambiguity margin; otherwise `unknown` or `ambiguous`. Reuse existing constants as initial **pilot** values (`0.5`, margin `0.15`) only after tests confirm interpretation; they are **not calibrated security guarantees**.
- Bad/too-short audio -> `insufficient_audio`; no candidates/no plan -> `skipped` or unavailable with stable reason; extraction failure/worker busy -> `unavailable`; bounded wait expiry -> `timeout`.
- One terminal `ws_speaker_turn_result` per **eligible Observe voice turn** (including failed to start scoring); `off`/text turns do not emit one. Cancellation/stale result is DEBUG only, not a successful terminal match. No duplicate result if ASR and Speaker finish in opposite order.
- Preserve old `hello.features.speaker_status` opt-in: client status contains only `{ "type":"speaker", "state":"..." }`. Maintain current states (`verifying`, `verified`, `unknown`, `insufficient_audio`, `unavailable`); map `ambiguous` to `unknown`, `timeout` to `unavailable` for legacy clients. Never send Speaker key/name/score over WS.

### 5.3 LLM context without persistent identity

Implement prompt composition at `session/actor/delivery.rs::begin_speech_delivery()` (or the existing equivalent single prompt-building seam), not a new prompt manager. `SessionActor` holds turn-local optional Speaker display name resolved from **the same frozen admission snapshot**; do not query SQLite per turn to learn the name. If current `ObserveCandidate` stores only `key`, extend the admission snapshot minimally with a sanitized `display_name` field.

```text
Speaker identification (advisory; not authentication):
- display_name: Nguyễn Văn A
- use_for: form of address in this turn only
- grants_tool_permissions: false
```

- Whitelist allowed fields, length-limit and sanitize the display name against instruction injection. Do not put free-form `description`, vector, scores or private data into LLM prompt.
- Compose a **temporary per-turn** context/message, distinct from persisted dialogue history and without changing the underlying Template persona. Check existing total prompt byte limits after composition.
- Unknown/timeout/disabled -> no identity claim; future turns can match different people. This metadata never grants MCP, Device tools, session switch authority, access to others' history or privileged resources.

## 6. Template switch: no stale Speaker plan

The current `SwitchSpeakerAuthority::authorize()` requires `qualified` and same embedding space even for Observe, while `install_switch_speaker()` returns early when the target is absent/incompatible. Both behaviors require explicit separation.

**Core rule:** the *target* Template defines the next Speaker Provider and candidate set. Switch authority for `required` is separate from `observe` plan installation. Do not require calibration qualification for `observe`.

| Origin -> target | Observe result after target commit | Required behavior |
|---|---|---|
| Same compatible Provider + target grants | Install **target** plan, using matching runtime/lease | Enforce qualified target and locked-speaker grant |
| Target has no Speaker Provider/candidates | Set `ObserveUnavailable(reason)`; clear old observe/PCM/identity; continue anonymously | Reject switch |
| Target embedding space differs | Use target's prepared runtime/lease plus its own candidate plan **if supported**; otherwise unavailable and clear old state | Keep current security restriction; reject incompatible target |
| Initial session ObserveUnavailable -> target ready | Install new target plan from target preparation/snapshot; do **not** depend on previous `Some(observe)` | N/A |
| Cold/managed Template switch | Resolve target using existing prepare/commit seam; atomic install/clear with profile | Recheck security token + gate before commit |
| Target resolution failed | Observe can switch to anonymous if core Template switch is otherwise valid; log reason | Reject, no downgrade |

Implementation requirements:

1. At admission/preparation, build bounded per-target candidate snapshots using existing `TemplateSwitchCatalog` and existing DB/provider runtime ownership. There must be an explicit result for a target with no Speaker Provider. Do not rely on `None`-means-unchanged.
2. In `builtin_actions.rs` **both** warm `apply_template_switch()` and managed `drain_managed_switch_boundary()` must install the target's Speaker state or clear stale state **at the same effective profile change**. No old `SpeakerObserve` can survive an incompatible target. For Observe, clear pending recognition and PCM; do not carry identity across switch.
3. Preserve `required` admission-time authority and fail-closed target guards; no unaudited downgrade. Existing revocation/security token must stay effective at arm **and** apply.
4. Add Speaker fields to existing `session_profile_switched` log (do not create a parallel `ws_speaker_template_switched` event). Log `old_template_key`, `template_key`, `speaker_recognition_enabled`, `speaker_recognition_ready`, `speaker_authentication_enabled`, `speaker_reason`.
5. Test all origin/target combinations, especially **ObserveUnavailable -> ObserveReady** and **ObserveReady -> missing Provider**. If multi-Provider runtime switching is not possible in this release, explicitly choose anonymous rather than guessing or retaining an old plan.

## 7. Structured WebSocket logs (mandatory)

**Do not print** secret/token/header value, raw Device identifier, Speaker full name, transcript, WAV/PCM, embedding, prompt, unfiltered error or similarity score at INFO. Speaker key is optional in restricted homelab logs; default may be anonymized/pseudonymous. Stable `reason_code` is not raw `error.to_string()`.

| Event | Level | Required fields / when |
|---|---|---|
| `ws_auth` | INFO accepted, WARN denied | At WS bearer + Device routing/admission decision: `request_id` (or request span), `session_id?`, `bearer_auth_enabled`, `bearer_auth_result`, `device_admission_result`, `result`, `reason_code`. Includes failure before Speaker resolution. |
| `ws_speaker_config` | INFO disabled/ready, WARN unavailable/rejected | Once per Voice WS admission attempt **once policy is known**: `session_id?`, `agent_key`, `template_key`, `speaker_mode`, `speaker_recognition_enabled`, `speaker_recognition_ready`, `speaker_authentication_enabled`, `candidate_count`, `result`, `reason_code`, `speaker_provider_key?`. |
| `ws_speaker_turn_result` | INFO / WARN for inference unavailable | Terminal Observe voice turn after actor ownership check: `session_id`, `turn_id`, `generation`, `agent_key`, `template_key`, `result`, `matched`, `speaker_key?`, `inference_ms`. |
| `ws_speaker_authorization` | INFO accepted / WARN denied | **Required only**, emitted **after** `SpeakerGate` decision: `session_id`, `turn_id`, `generation`, `result=accepted|denied`, `reason_code`. |
| Existing `session_profile_switched` | INFO/WARN | Add new Speaker status fields; no new event name. |

- Connection logging occurs **before upgrade** where possible. If the request fails before profile/policy is known, only `ws_auth` (or existing early validation log) applies; do not fabricate `enabled=false`.
- `speaker_recognition_enabled = mode != off`. `speaker_recognition_ready` means usable model/lease/candidates, not authentication. `speaker_authentication_enabled = mode == required`, independent of recognition match. `bearer_auth_enabled` indicates WS bearer requirement, a separate dimension.
- Do not claim a Speaker identity upon connection: WS knows only candidate set until audio arrives. Match logs are emitted only for current completed voice turns; stale completion is DEBUG and never re-labeled success.
- `ws_speaker_authorization` is an event name, **not** an HTTP Authorization header. Test privacy by inspecting structured fields and values, not by rejecting every occurrence of substring `authorization`.

Representative expected output (illustrative, not existing production output):

```text
INFO event="ws_auth" request_id="req-1" bearer_auth_enabled=true bearer_auth_result="accepted" device_admission_result="admitted" result="accepted" reason_code="ok"
INFO event="ws_speaker_config" session_id="s1" agent_key="home" template_key="main" speaker_mode="off" speaker_recognition_enabled=false speaker_recognition_ready=false speaker_authentication_enabled=false candidate_count=0 result="accepted" reason_code="disabled"
INFO event="ws_speaker_config" session_id="s2" agent_key="home" template_key="main" speaker_mode="observe" speaker_recognition_enabled=true speaker_recognition_ready=true speaker_authentication_enabled=false candidate_count=2 result="accepted" reason_code="ready"
INFO event="ws_speaker_turn_result" session_id="s2" turn_id=14 generation=7 agent_key="home" template_key="main" result="matched" matched=true speaker_key="spk_abc" inference_ms=135
WARN event="ws_speaker_config" session_id="s3" agent_key="home" template_key="main" speaker_mode="required" speaker_recognition_enabled=true speaker_recognition_ready=false speaker_authentication_enabled=true candidate_count=0 result="rejected" reason_code="no_speaker_provider"
WARN event="ws_speaker_authorization" session_id="s4" turn_id=3 generation=2 result="denied" reason_code="mismatch"
```

Privacy rules apply to HTTP+WS logs, any operational reports and test snapshots. Reuse existing tracing subscriber and correlation; no additional UUID namespace for speaker events.

## 8. Web UI readiness: distinguish configured, provider-ready, session-ready

`Agent Detail > Speaker Recognition` displays: toggle, granted Speakers, Template associations, quick-enroll CTA and status. Existing `required` is rendered as **Legacy voice authorization active** with an explicit Admin-only change path, not a common toggle.

- **Configured:** Template has Speaker Provider binding plus at least one eligible Agent + Template grant. Compute from existing policy/grant/Template endpoints.
- **Provider Ready:** reuse `runtime_status` / `runtime_matches_desired` from the existing Provider endpoints; this is only runtime readiness, not proof WS admission will work.
- **Session Ready:** only the server's actual admission/switch snapshot determines it. Without a live authenticated session status source, UI must label this **"Session readiness will be checked on connection"**, not promise Ready from DB alone.
- When mode `observe` is enabled but provider/grants are missing, show `Enabled - setup incomplete`; no auto-disable or hidden downgrade.
- Use existing Admin HTTP routes with CAS / ETag. Do not add a second `speaker_enabled` database column or a new readiness endpoint just to show a toggle.
- Quick enrollment remains a 3-stage wizard; `accepted` means quality accepted, and resulting profile is provisional. Keep microphone/browser permission error guidance and link to Agent after Save.

## 9. Required/calibration compatibility and migrations

No automatic deletion of `speaker_calibration.rs`, `speaker_gate.rs`, calibration tables, reload route, `calibration_source` or the legacy `required` policy. Normal `observe` no longer calls qualification to decide whether identification can run; Required still requires its established qualification/evidence and gate.

Before release, verify there are no implicit `required -> observe/off` migrations, no old-session privilege retention on Admin policy change, and no fail-open branches for missing Speaker Provider, Template ID, DB handle, runtime lease or query errors. Keep forward-only DB migration practice; **this change normally needs no new SQLite table/migration**. Any future removal of Required/calibration is a separate explicit ADR and user-approved migration, not part of this guide.

## 10. Implementation plan: three PRs, no duplicate checklists

| PR | Existing seams / modules | Required change and acceptance |
|---|---|---|
| **1 — Admission + auth logs** | `app/websocket.rs`, policy resolver, security admission | Typed accepted setup; fail-closed Required; `ws_auth` for bearer/Device decisions, one `ws_speaker_config` once policy known, correct enabled/ready/authentication booleans. No prompt changes. |
| **2 — Turn result + LLM** | `speaker_observe.rs`, `actor/observe.rs`, `actor/listening.rs`, `actor/delivery.rs` | Actor-owned nonblocking bounded 1.5s join; one terminal outcome; no stale INFO match; optional matched display name in **current** prompt only; preserve Required and history semantics. |
| **3 — Template switching + Web + docs** | `switch_authority.rs`, `actor/tools/builtin_actions.rs`, Vue panels, `docs/api/00-all-apis.postman_collection.json`, ADR/config docs | No stale plan after switch; Observe permits anonymous target; Required protected; UI toggle/grants/accurate readiness; docs and E2E synced. |

Do not insert infrastructure, new engine lifecycle, generic policy framework or additional database tables. Keep the work localized to these existing modules and test through their observable interfaces.

## 11. Verification and acceptance tests

**Core cases:**

- `off`: WS authenticates as before; config log has recognition/authentication false, no Speaker inference or voice-turn result.
- `observe` ready: correct `candidate_count`; 2 alternating Speakers in one WS yield per-turn distinct identity; sanitized name appears in corresponding LLM prompt only.
- `observe` unavailable (missing Provider/candidates/lease/DB resolution): WS accepted anonymously; config log enabled=true, ready=false, authentication=false, stable reason.
- First-turn empty/too-short audio, worker busy, timeout, ASR-final-before-observe and observe-before-ASR-final: no hang, no double terminal event, no identity contamination.
- Abort/reconnect/revocation/generation change/history barrier: late results are discarded; old template/turn identity never reused.
- Text `detect`: no inherited speaker; does not trigger Observe identification from old audio.
- Required missing provider/lease/Template/DB/qualified profile/guard: refuse WS or switch as applicable, no ungated Voice Session; preserve Required gate decisions and text behavior.
- Template switching in Observe: ready->ready (same space), ready->unavailable (missing/incompatible Provider), unavailable->ready; **no old plan after target commit**. Test warm and managed/cold paths.
- Device pending-enrollment connection: `ws_auth` shows pending route, no fabricated Speaker mode/config.
- Admin toggle CAS conflict, `required` visible as legacy and not silently downgraded; Provider readiness does not falsely report Session Ready.
- Logging: structured tracing asserts on field values, privacy and event counts. `ws_speaker_config` exactly once after known policy; `ws_auth` present for auth decision; Observe terminal at most once per eligible turn; valid event `ws_speaker_authorization` allowed but **never** raw `Authorization` header/token.
- Runtime results: sample quality accepted != authenticated; `pending` is Observe-only/provisional, not a Required authorization shortcut.

Run from repository root (verify test target names against current HEAD before running):

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy -p voice-agent-server --lib --tests -- -D warnings
cargo test -p voice-agent-server --lib
cargo test -p voice-agent-server --test speaker_observe
cargo test -p voice-agent-server --test speaker_required
cargo test -p voice-agent-server --test agent_speaker_policy_api
cargo test -p voice-agent-server --test speaker_enrollment_api
cargo test -p voice-agent-server --test speaker_security_revocation_hooks
cd apps/admin-web
npm run test
npm run build
```

### Definition of Done

- [ ] Quick Web enrollment stores Speaker profile + embedding in SQLite without `calibration.json`; candidates/grants selectable by Agent/Template.
- [ ] UI ordinary mode is an `off/observe` toggle with correct readiness labels and legacy Required protection.
- [ ] Every applicable WS bearer/Device admission decision records `ws_auth`; every Voice admission after Speaker policy resolution records `ws_speaker_config` even when OFF.
- [ ] WS config log includes `speaker_recognition_enabled`, `speaker_recognition_ready`, `speaker_authentication_enabled` and reason; no confusion with `bearer_auth_enabled`.
- [ ] Every eligible Observe audio turn has a bounded terminal outcome; correctly matched name is used only in current LLM turn, not permissioned history/tools.
- [ ] Template switch never carries the previous Speaker plan into an incompatible/missing target; Required stays fail-closed.
- [ ] No raw audio, vector, transcript, speaker name, secrets, scores or raw Authorization header appear in INFO logs or WS status frames.
- [ ] Existing WS compatibility, Device auth, ASR/VAD/TTS, MCP policies, history barrier and cancellation behavior preserved.
- [ ] Tests executed and actual results recorded; Postman/docs/ADR aligned. Do not claim tests passed until run.

**Agent handoff report:** changed files/commits; before/after WS auth + speaker logs; table of Template-switch state transitions; any limitations; exact commands and test outcomes. Do not push/deploy without instruction.
