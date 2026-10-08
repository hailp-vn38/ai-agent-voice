# Quick Speaker Enrollment — Rust Server + Vue Web Implementation Guide

**Repository:** `hailp-vn38/ai-agent-voice`  
**Baseline:** `main`, reviewed 2026-10-08 (read-only code inspection; recheck the target branch/HEAD before implementation)  
**Status:** Revised architecture/implementation specification; no repository code changed by this document.  
**Scope:** Quick Speaker Enrollment only. The Device MCP / External MCP authorization refactor remains a separate workstream.

## 1. Decision and outcome

Make **Quick Enrollment** the default flow for a *new* Speaker:

```text
Speakers > Add Speaker
  1. Select an enabled Speaker Provider.
  2. Record 5–10 s using browser mic; send mono PCM16/16 kHz WAV.
     Server verifies clip quality and extracts an embedding.
     API returns `accepted` plus an opaque, short-lived capture_id.
  3. After acceptance only, enter name and optional description; Save.
     Server atomically creates Speaker + one provisional voiceprint,
     consumes capture, and responds with Speaker details.
  4. Web shows success and links to Speaker Detail.
```

Keep **Full Enrollment** (3–5 accepted clips, independent holdout, validation, finalize) for upgrading a provisional voiceprint. A Quick sample is **not verified identity** and must **never** satisfy `required` Speaker Policy by itself.

### Non-negotiable invariants

- Capture does **not** create a Speaker, voiceprint, Agent candidate, Template grant, or change policy. Save is explicit.
- `accepted` means quality and embedding extraction succeeded, not that an identity was recognized or calibrated.
- Server never stores original WAV; stage only a bounded, normalized embedding plus minimum provenance. Browser releases the WAV/preview on close.
- Quick voiceprint uses the existing `browser_validation_status='pending'`; only current Full Enrollment finalize publishes `'passed'`.
- `required` admission/qualification must reject provisional candidates on the **server**; no Web badge or disabled button is an authorization gate.
- Full Enrollment, qualified calibration evidence, Speaker/Agent/Template authority, and scoped session invalidation remain intact.
- A repeated commit with the **same** capture ID cannot create a second Speaker; concurrent requests are serialized or produce a safe retryable conflict.
- Both new routes use existing `/api/admin` Bearer auth, request bounds, audit, and error-envelope conventions.

## 2. Codebase Design: one deep module, two external operations

### Current ownership (confirmed on `main`)

| Existing module | Responsibility to retain |
| --- | --- |
| `app/admin/speakers.rs` | Speaker CRUD, Full Enrollment, versioning, audit, `speaker_resource_value`, catalog publication. |
| `audio/enrollment.rs` | WAV parser, `QualityProfile`, clip analysis, vector validation/normalization/encoding. |
| `services/provider_diagnostic/` and provider runtime manager | Bounded inference, runtime leases and provider revision checks. |
| `session/speaker_observe.rs` | Build effective runtime candidates for a session's Agent, Template, embedding space and Speaker Policy. |
| `database/speaker_candidate_set.rs` | Canonical exact-candidate-set projection/digest used by qualification. |
| `app/admin/speaker_calibration.rs`, `speaker_policy.rs` | Required-mode qualification, grant/policy verification and revocation. |
| `views/SpeakersView.vue`, `pages/speakers/SpeakerDetailPage.vue` | New-Speaker entry and existing Full Enrollment UI. |
| `composables/useMicrophoneRecorder.ts`, `lib/wav.ts` | Browser mic lifecycle and real WAV conversion. Reuse unchanged unless a bug is demonstrated. |

### Chosen seam and module

Create **one** module: `crates/voice-agent-server/src/app/admin/speaker_quick.rs`, registered from `app/admin/mod.rs`. Its external HTTP interface has only two operations:

```rust
// Illustrative; use actual Axum State/Path/Request types and project naming.
async fn create_capture(provider_key: String, request: Request) -> Response;
async fn create_speaker_from_capture(request: Request) -> Response;
```

The implementation hides WAV admission, extraction, SQLite staging, commit, idempotency and response assembly **inside this module**. It calls existing audio and runtime implementations. Reuse small existing Speaker helpers by exposing them to the sibling module as `pub(super)` where needed; do not copy validators, Speaker serialization, catalog helpers, or native extraction logic.

```text
Vue QuickSpeakerEnrollment.vue
   | POST WAV                          | POST name + capture_id
   v                                   v
app/admin/speaker_quick.rs  [single module; two operations]
   | reusable quality/extraction       | SQLite transaction + existing audit/catalog helpers
   v                                   v
 audio/enrollment.rs              speakers + speaker_voiceprints
 provider_diagnostics             speaker_quick_captures
```

**Do not add** `services/speaker_quick_enrollment.rs`, a second `database/speaker_quick_enrollment.rs`, generic `CaptureStore` trait, task queue, streaming transport, separate HTTP client abstraction, new runtime/worker pool, or an enrollment workflow engine. SQLite + local test runtime already provide usable test seams. Only extract shared internal helpers when *actual duplication* justifies it.

The **interface is the test surface**: test accepted capture, committed Speaker, stale provenance, idempotency and required eligibility using HTTP + real test SQLite, not by reaching into private helpers. Preserve and extend existing Full Enrollment tests.

## 3. Exact Admin HTTP contract

Both routes inherit existing Admin Bearer authentication, no CORS widening and no Voice token reuse.

### 3.1 Capture: `POST /api/admin/providers/{provider_key}/speaker-captures`

Request:

```http
POST /api/admin/providers/speaker_campp/speaker-captures
Authorization: Bearer <admin_token>
Content-Type: audio/wav
If-Match: "<provider_revision>"

<raw PCM16 mono 16 kHz WAV>
```

- Enforce existing `speaker_recognition.enrollment.max_audio_body_bytes` (default 512 KiB), clip duration (default 5–10 s), minimum speech (default 3 s) and extraction window (at most 6 s); obtain limits from resolved config, not UI constants.
- Verify provider exists, `type='speaker'`, enabled and exact provider revision **before** native work. Reuse `provider_diagnostics.extract_speaker_embedding()` so cold acquisition/limits are owned by the current runtime manager; do **not** make Prepare a required wizard step.
- Reuse `enrollment::parse_wav`, `QualityProfile`, `enrollment::analyze`, `validate_embedding`, normalization and encoding. Check finite values, dimension/embedding-space provenance and runtime revision.
- After successful extraction, stage a normalized vector in SQLite (subject to bounded quota and TTL) and only then report success. No SQLite write transaction held during inference.

`201 Created`:

```json
{
  "status": "accepted",
  "capture_id": "82d28fa3-1070-4b62-ad12-d28fd54fa08e",
  "quality": { "duration_ms": 7900, "speech_ms": 5800 },
  "expires_at": 1791441900
}
```

Quality details are **response-only**, not duplicated into staging rows. Do not return embedding, waveform, matched identity, cosine score, secret, model paths, or calibrated status. Do not stage rejected or timed-out inference.

### 3.2 Commit: `POST /api/admin/speakers/from-capture`

Request:

```http
POST /api/admin/speakers/from-capture
Authorization: Bearer <admin_token>
Content-Type: application/json

{"capture_id":"82d28fa3-1070-4b62-ad12-d28fd54fa08e", "name":"Chủ sở hữu", "description":"Giọng nói chính"}
```

- Require nonblank `name` (existing Speaker name limit 128) and optional `description` (existing limit 2048); deny unknown fields and never accept a caller-supplied embedding, key, provider revision, Agent/Template grant or assurance claim.
- Generate the Speaker `key` on the server (e.g. `spk_` plus random UUID hex), conforming to the repository's key validation. Keep manual `POST /speakers` for advanced profile-only creation.
- Verify accepted, unexpired, current-runtime capture and current enabled provider with **same ID/revision/embedding contract**, then atomically create Speaker + voiceprint.

First success `201 Created` (the `speaker` object is serialized by existing `speaker_resource_value()`):

```json
{
  "speaker": {
    "key": "spk_842cf06ae5d04a039e99132de217469d",
    "name": "Chủ sở hữu",
    "description": "Giọng nói chính",
    "enabled": true,
    "revision": 1,
    "voiceprints": [{"sample_count": 1, "browser_validation_status": "pending"}],
    "enrollment_drafts": []
  }
}
```

The snippet abbreviates the existing Speaker/voiceprint fields; **do not invent another Speaker response schema**. Display `Provisional` in Vue by mapping existing `browser_validation_status='pending'`. Idempotent retry returns the **same response shape** with `200 OK`, never creates a second Speaker.

### 3.3 Stable failure behavior

Use existing `{error:{code,request_id}}` envelope. Preserve typed diagnostics where already defined.

| Case | Server behavior | Web behavior |
| --- | --- | --- |
| Bad WAV/media type/size/length/silence/clipping | 400/415/413/422, no capture | Show actionable retry and remain at mic step. |
| Provider missing, disabled, wrong type or stale revision | 404/409, no extraction | Reload provider; record again if provenance changed. |
| Busy/timeout/unavailable model | Existing 429/503/504 mapping | Show failure; do not advance to details. |
| Unknown/expired/cross-runtime capture | 404/410/409, no Speaker | Clear capture; record again. |
| Invalid name | 400, accepted capture unchanged | Preserve details for correction. |
| Global Speaker quota full | 409, accepted capture unchanged | Show quota error; allow retry until TTL. |
| Duplicate commit after success | 200 same Speaker; no second insert | Show success. |
| Conflicting/busy concurrent transaction | Safe retryable error if not serialized | Retry **same** capture ID, never a new Speaker implicitly. |

No polling, SSE, WebSocket stream or client-side timer is necessary for the synchronous bounded capture request.

## 4. SQLite: smallest durable staging model

Create a **forward-only migration** after the highest migration in the target branch. Do not edit existing `0009_speakers.sql` or `0013_speaker_holdout_finalize.sql`.

Illustrative schema:

```sql
CREATE TABLE speaker_quick_captures (
    id TEXT PRIMARY KEY,
    provider_id INTEGER REFERENCES providers(id) ON DELETE SET NULL,
    provider_revision INTEGER NOT NULL CHECK (provider_revision > 0),
    runtime_id TEXT NOT NULL,
    embedding_space TEXT NOT NULL,
    dims INTEGER NOT NULL CHECK (dims > 0),
    vector BLOB,
    status TEXT NOT NULL CHECK (status IN ('accepted','committed')),
    speaker_id INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    committed_at INTEGER,
    CHECK (
        (status='accepted' AND vector IS NOT NULL AND speaker_id IS NULL AND committed_at IS NULL)
        OR (status='committed' AND vector IS NULL AND committed_at IS NOT NULL)
    )
);
CREATE INDEX idx_speaker_quick_captures_expiry
    ON speaker_quick_captures(status, expires_at);
```

No redundant `provider_key`, duration, speech time, raw WAV, PCM digest, claimant metadata, calibration assertion or new `enrollment_assurance` column. `provider_id`, revision, runtime incarnation, embedding space and dimension are the minimum provenance used for commit validation. A deleted provider leaves `provider_id=NULL` and cannot be committed; an already-committed tombstone may retain only the Speaker reference and non-biometric metadata.

### Reuse existing assurance semantics

**No new migration column for assurance.** `speaker_voiceprints.browser_validation_status` already supports `pending`, `passed`, `failed`:

| Stored value | UI term | Meaning |
| --- | --- | --- |
| `pending` | `Provisional` | Quick single sample, usable only for explicitly granted Observe diagnostics. |
| `passed` | `Validated` | Full Enrollment holdout/finalize passed; `required` still needs independent exact-set calibration qualification. |
| `failed` | `Not validated` | Not eligible for `required`; retain existing behavior for historical data. |

`Full Enrollment finalize` already writes `browser_validation_status='passed'` and replaces a selected embedding-space voiceprint. Ensure upgrading a Quick Speaker preserves the same Speaker key and keeps the existing catalog revision/invalidation behavior. Existing `passed` records require no data backfill. Do not create `mixed` or duplicate assurance state: show per-voiceprint status on detail and simple `Unenrolled / Provisional / Validated` where a summary is useful.

### Capture lifetime, quotas and idempotency

- Fixed internal **accepted TTL = 10 minutes**, committed non-biometric retry tombstone retention **24 hours**. Constants only unless a real operational need appears. Clean with the **existing** Speaker draft expiration/retention sweep and startup cleanup; no new timer, queue or daemon.
- Reuse the existing `speaker_recognition.enrollment.max_open_enrollments` **as one total active cap** across collecting Full drafts plus accepted Quick captures. Enforce it in a serialized SQLite write transaction and update both the Full draft creation and Quick capture insertion paths; do not just check two tables independently and call it a global limit. Native inference remains independently bounded by the existing runtime manager.
- An accepted row stores the normalized vector until commit/expiry. On commit, clear `vector=NULL`, set `status='committed'`, `committed_at`, `speaker_id`. Deleting/expiring accepted rows removes the biometric data; committed tombstones are swept after 24 hours.
- Server restart invalidates accepted rows pinned to a different `runtime_id`; no silent cross-incarnation replay. A committed tombstone may still resolve a lost-response retry if its Speaker exists and is inside retention.
- At commit, **obtain a write reservation first** through existing SQLite/SQLx patterns (e.g. a conditional `UPDATE ... WHERE id=? AND status='accepted'` within the transaction) before reading quota/inserting Speaker. A second concurrent commit must see the committed outcome or a retryable busy/conflict; no duplicate insert. Do not add a separate `processing` row state or speculative distributed lock.
- In the *same transaction*, re-check capture, provider, `max_speakers`, status/TTL, insert Speaker, insert `speaker_voiceprints` with `sample_count=1` and `browser_validation_status='pending'`, bump `speaker_catalog.revision` with the existing helper, mark capture committed/clear its vector, and record audit. Roll back all writes on failure. Do not hold this write transaction during model work.
- An idempotent retry with a different name/description **does not mutate** the committed Speaker. If the Speaker was deleted, return a stable consumed/gone error rather than recreating it.

## 5. One exact-set eligibility decision for Speaker `required`

**This is the critical correctness seam.** Currently `session/speaker_observe.rs::resolve_observe_plan` selects any granted voiceprint in the runtime embedding space, while `database/speaker_candidate_set.rs` hashes candidate voiceprints without their `browser_validation_status`. Merely labeling a Quick sample `pending` in the UI would allow it into the old runtime query.

### Minimal coherent implementation

1. **Candidate projection:** Extend the **existing** canonical `database/speaker_candidate_set.rs::candidate_set` to include `browser_validation_status` for each voiceprint, so the digest changes on validation-state changes. Keep all participants/calibration paths using this **same** digest implementation.
2. **Required eligibility predicate:** Add **one small eligibility check** at the candidate-set/qualification seam: an Agent's exact granted candidate set cannot qualify `required` if any included voiceprint is not `'passed'`. Check only Speaker candidates actually linked to the Agent (and their relevant granted voiceprints), not unrelated Speakers. Choose a conservative all-referenced-voiceprints rule for V1; do not silently remove a provisional entry from one caller's snapshot while leaving it in another caller's digest.
3. **Qualification:** Make `app/admin/speaker_calibration.rs::qualification` require both its existing qualified-profile + exact digest evidence **and** the shared eligibility predicate. `speaker_policy.rs` uses this result when enabling `required`. The result may expose a specific `provisional_candidate` blocker but must not create a second independent eligibility algorithm.
4. **Session admission:** In `session/speaker_observe.rs::resolve_observe_plan`, when mode is `required`, return `RequiredUnavailable` if any effective candidate is not `passed` (including mixed passed/pending sets). Load or project status alongside the vector; do not drop pending rows and accidentally admit a smaller, unqualified set. `observe` can use explicitly granted `pending` samples for non-authoritative diagnostics.
5. **Switch and revocation:** Reuse existing `SpeakerSwitchGuard`, candidate digest, catalog-generation update and `invalidate_speaker`/Agent invalidation hooks. A provisional voiceprint cannot gain `required` authority on switch. New grants, Full promotion, provider changes and calibration updates require exact-set requalification; old open sessions must not gain rights in place.

**Important nuance:** `passed` means enrollment validation, **not** deployment calibration qualification. Full Enrollment alone does not unlock `required`; a separately qualified calibration/evidence set remains necessary. Do not change Speaker Policy modes, the optional Template Speaker provider slot or MCP tool permissions.

## 6. Giao diện Web đề xuất — detailed Vue implementation specification

This section is the **implementation contract for the Web UI**, not an optional visual sketch. Deliver a simple, modern Speaker registration experience using the existing Vue design system. UI language is Vietnamese (`i18n/messages.ts`); technical identifiers stay in English. Do not create a new page, generic wizard framework, global state store, or a third Speaker-enrollment API.

### 6.1 Integration with the current Vue app

| Existing code | Required behavior |
| --- | --- |
| `apps/admin-web/src/views/SpeakersView.vue` | Primary action **Thêm người nói** opens Quick Enrollment. Keep **Tạo hồ sơ trống (nâng cao)** as a secondary menu/action invoking the existing manual form. Refresh the list after creation. |
| **New** `apps/admin-web/src/components/speakers/QuickSpeakerEnrollment.vue` | Single view-local module owns modal, provider selection, three-step presentation, recorder and capture/commit requests. Suggested parent interface: `v-model:open` plus `@created` carrying the returned Speaker; use the project's actual Vue conventions. |
| `apps/admin-web/src/components/admin/BaseModal.vue` | Reuse its dialog, header/footer, scroll area and default `max-w-2xl` width. Do not implement another modal. |
| `apps/admin-web/src/composables/useMicrophoneRecorder.ts` + `lib/wav.ts` | Reuse `start`, `stop`, `dispose`, `recording`, `elapsedMs`, `level` and WAV encoder. Fix the input-sample-rate bound described in §6.8 before browser acceptance. |
| `apps/admin-web/src/pages/speakers/SpeakerDetailPage.vue` | Preserve existing Full Enrollment; display current voiceprint validation status and link **Hoàn tất xác minh** to the same existing Full flow. |
| `apps/admin-web/src/api/{speakers.ts,types/speakers.ts,errors.ts}` | Add typed capture/commit methods and localized error mappings; preserve the existing Admin request client and `If-Match` conventions. |
| `apps/admin-web/src/i18n/messages.ts` | Add only new Quick Enrollment message keys; reuse current common/button/Speaker keys when appropriate. |

Do not store biometric audio, `capture_id`, or wizard form state in Pinia, localStorage, sessionStorage, IndexedDB, query strings or route parameters. The parent Speaker list need only know whether the dialog is open and which Speaker was created.

### 6.2 Layout and visual hierarchy

**Entry point (existing Speakers screen):** keep the existing `PageHeader`, refresh action, search and table. Make `Thêm người nói` visually primary. The empty-list state should have a clear **Thêm người nói** action, not just an explanatory sentence. The existing manual empty-profile action remains available but visually secondary. After a successful commit refresh the Speaker list and optionally navigate when the operator clicks **Xem chi tiết**.

**Modal (desktop 640 px and up):** use `BaseModal` with approximately its existing `max-w-2xl` panel, scrollable body and footer. Content order is (1) title and short explanatory sentence, (2) a compact three-step progress indicator, (3) active-step content, (4) footer actions. Keep page background, border, radius, typography, spacing and button variants from the existing design system; no new CSS library or hard-coded brand colors.

```text
+------------------------------------------------------------------+
| Đăng ký giọng nói                                             [X]|
| Tạo người nói mới từ một mẫu thu âm.                            |
+------------------------------------------------------------------+
|  [1 Ghi âm] ------ [2 Kiểm tra] ------ [3 Thông tin]             |
|                                                                  |
|  [Content changes for active step; see §6.3–§6.5]                |
|                                                                  |
+------------------------------------------------------------------+
|  [Huỷ / Quay lại]                         [Primary next action]   |
+------------------------------------------------------------------+
```

**Mobile below 640 px:** panel takes available viewport width (reuse `BaseModal` padding), body scrolls independently beneath header, and footer actions stack or become full-width. Step labels may shorten to `Ghi âm / Kiểm tra / Thông tin`. Keep all buttons at usable touch size; do not position audio controls behind the footer or rely on hover for essential instructions. No separate mobile page or responsive component is necessary.

**Hierarchy and visual elements:**

- Title `Đăng ký giọng nói`; subtitle `Thu âm và kiểm tra trước khi lưu thông tin người nói.`
- Small step indicator with current step highlighted and completed steps marked. It is a progress *display*, **not** a free-navigation tab: do not let users skip validation by clicking Step 3.
- Center the microphone control and use existing `Mic`, `Square`, `CheckCircle`, `AlertCircle` or available Lucide icons. Use `recorder.level` as a simple level bar or pulse; **do not add waveform libraries or persist raw samples**.
- Use standard success/error treatments from the project; reserve high-contrast warning text for failed captures and `Provisional` explanation, never suggest identity has been verified.
- Do not render `provider_key`, revision, embedding-space hash, native worker count, cosine score, `capture_id` or system quota as ordinary end-user fields. Provider name is useful; technical provenance can remain in existing provider/detail screens.

### 6.3 Step 1 — Chọn Provider và ghi âm

```text
| Speaker Provider                                                |
| [ CAM++ Speaker Recognition                              v ]   |
| Runtime chưa tải: sẽ chuẩn bị khi xử lý mẫu (optional note)      |
|                                                                  |
|               [ Mic icon + input-level meter ]                   |
|             Sẵn sàng ghi âm / Đang ghi âm 00:07                 |
|                                                                  |
|        Hãy nói tự nhiên 5–10 giây ở nơi ít tiếng ồn.             |
|                                                                  |
|         [Bắt đầu ghi âm]  ->  [Dừng ghi âm]                     |
|                 (Dừng chỉ bật sau min_clip_ms)                   |
```

Implementation rules:

1. **On open:** concurrently call `providersApi.list({type:'speaker', ...})` and `speakersApi.summary()`; use the list's enabled Speaker providers and the summary's resolved audio limits and optional runtime state. Handle pagination if the list exceeds one page. Show a skeleton/loading placeholder, then the provider select. If only one provider is enabled, preselect it; if several, require a selection.
2. **No provider / unavailable:** show `Chưa có Speaker Provider khả dụng` and a secondary navigation link to existing Providers page. Disable recording. If `summary.available=false`, show runtime unavailable rather than recording audio that cannot be processed. A `cold` provider is still selectable: do not insert **Prepare** into the wizard.
3. **Before recording:** show microphone/privacy hint and display `5–10 giây` (or actual `min_clip_ms/max_clip_ms`) from `summary.enrollment`, not fixed Web constants. Microphone permissions are requested only on a direct **Bắt đầu ghi âm** click, on HTTPS or localhost.
4. **Recording:** call existing `recorder.start(limits, stopRecording)`. Show live elapsed timer `mm:ss`, a visible recording indicator and level feedback from `recorder.level`. The user may select a Provider only *before* recording; freeze it until the recording/upload is discarded. Stop automatically at `max_clip_ms` using the composable.
5. **Stop:** disable **Dừng ghi âm** before `min_clip_ms` to avoid an expected too-short recording; ignore duplicate Stop/auto-stop callbacks. Convert the returned Blob to WAV through the existing composable, not a second audio encoder. If Blob is empty, return to the recorder with a message.
6. **Upload:** after Stop, send WAV immediately to `POST /providers/{key}/speaker-captures` with `If-Match` set to the selected Provider revision. Show `Đang gửi và xử lý mẫu giọng nói…`, a spinner/progress *indeterminate* indicator and disabled submit controls. Do not fake a percentage, use streaming, or call the separate Provider test endpoint first.
7. **Step transition:** when capture responds `201 accepted`, retain `{capture_id, expires_at, quality}` in memory, drop the WAV Blob and move to Step 2. For an error, show Step 2's error summary (or an inline recorder error) with **Ghi âm lại**; do not display the metadata form.

**Provider changes after a failed capture:** on retry the operator may select another Provider; refresh its revision first. Changing the Provider or recording again clears the previous `capture_id` locally (the old accepted capture expires server-side). Never attach one Provider's capture to another Provider's label.

### 6.4 Step 2 — Kết quả kiểm tra API

**Processing presentation** is only `uploading=true`; do not add a fourth wizard step or an async polling state. On completion show one of these two explicit outcomes:

**Accepted view:**

```text
|                         [check icon]                             |
|                 Mẫu giọng nói hợp lệ                              |
|            Server đã xử lý mẫu âm thanh thành công.              |
|                                                                  |
|             Thời lượng: 7.9 giây                                 |
|             Có tiếng nói: 5.8 giây                               |
|                                                                  |
|  Lưu ý: Đây là bước lấy mẫu, chưa xác minh danh tính người nói.   |
|                                                                  |
| [Ghi âm lại]                         [Tiếp tục nhập thông tin]    |
```

- Show `duration_ms` and `speech_ms` rounded to one decimal second, if returned by the API. Do not show raw embedding, internal threshold, provider confidence or a claim of a recognized speaker.
- **Tiếp tục nhập thông tin** is enabled only while `capture_id` exists and has not expired. No need for an always-running visible countdown; show `Mẫu có hiệu lực trong khoảng 10 phút` or a simple expiration message as appropriate.
- **Ghi âm lại** returns to Step 1 and discards the in-memory capture, with server cleanup left to TTL. A browser-local audio playback control is optional only while the original Blob exists; it must never block Next and must release its object URL. Prefer no preview for V1 because the server has already accepted the clip.

**Rejected view:**

```text
|                        [warning icon]                            |
|                    Mẫu chưa đạt yêu cầu                          |
|         Âm thanh quá ngắn / nhiễu / bị rè / không có tiếng nói.  |
|                                                                  |
|  Mẹo: đứng gần micro hơn, tránh quạt/TV, nói liên tục 5–10 giây. |
|                                                                  |
|                          [Ghi âm lại]                             |
```

- Match **typed error codes**, not English message fragments or only numeric HTTP status. Reuse `formatApiError` and add human-friendly mappings where absent: `speaker_audio_clipped`, `speaker_insufficient_audio`, `unsupported_audio_format`, `request_too_large`, `provider_disabled`, `provider_revision_conflict`, `provider_runtime_busy`, `provider_runtime_unavailable`, `speaker_inference_timeout`, `enrollment_quota_exceeded`.
- The busy/runtime errors should say to try again later; invalid audio errors should give recording guidance; revision conflict should refresh the Provider list and require a new capture. An unauthenticated response follows the existing Admin error behavior.
- An accepted response is **not** `Validated`, `Qualified`, or `Speaker recognized`; it is just a valid sample with a successfully extracted embedding.

### 6.5 Step 3 — Nhập thông tin người nói và lưu

```text
|                         [check icon]                             |
|             Mẫu giọng nói đã sẵn sàng                            |
|                                                                  |
| Tên người nói *                                                  |
| [ Ví dụ: Chủ sở hữu                                       ]       |
|                                                                  |
| Mô tả (không bắt buộc)                                          |
| [ Ví dụ: Người sử dụng thiết bị tại nhà                   ]       |
|                                                                  |
| Khoá Speaker được server tạo tự động.                            |
| Sau khi lưu: Provisional — có thể hoàn tất xác minh sau.         |
|                                                                  |
| [Quay lại]                                      [Lưu người nói]  |
```

- **Fields:** `name` required, trim before submit, 1–128 valid characters under the existing server validation; `description` optional, max 2048. Use the existing input/textarea styles and validation messages. There is **no** `key`, enabled toggle, Agent, Template, threshold, voiceprint state or grant field in Quick registration.
- **Commit:** call `speakersApi.createFromCapture({capture_id, name, description})`; disable **Lưu người nói** during request and prevent rapid duplicate submits. Do not run a preliminary `POST /speakers` followed by another call. The server atomically creates the real Speaker and its `pending` voiceprint.
- **Failed validation/quota:** remain on Step 3 with the entered fields intact; show the error beside the field or above the footer. Do not consume/re-record an accepted capture just because the user mistyped a name. Unknown/expired/cross-runtime capture must clear the capture, show `Mẫu đã hết hạn. Vui lòng ghi âm lại.` and return to Step 1.
- **Uncertain network/timeout after Save:** keep the **same capture ID** and metadata while this wizard is open, and allow **Thử lưu lại** with the same ID. The server's idempotent commit contract must return the existing Speaker if the first request actually succeeded; do not create another capture or Speaker automatically.
- **Success:** replace the Step 3 form content with a compact confirmed result, not a new route or wizard step. Show `Đã đăng ký người nói`, the name and `Provisional — chưa xác minh đầy đủ`, with primary **Xem chi tiết** (navigate via existing `speaker-detail` route using server-returned key) and secondary **Đóng**. Emit `created` **once immediately on confirmed commit** so the parent refreshes the Speaker list regardless of which success action the operator chooses. Never display success before a `201` or idempotent `200` commit response.
- The success summary may mention `Hoàn tất xác minh` as a separate action in Speaker Detail. **Do not** automatically create Agent candidates, Template grants or switch policies.

### 6.6 Action and interaction matrix

Use **one** `step: 'record' | 'result' | 'details'`, current recorder refs, `uploading`, `saving`, `capture`, `captureError`, and `createdSpeaker`. Do not build seven workflow states or a new state-management abstraction.

| Current UI condition | Primary control | Secondary control | Allowed transition |
| --- | --- | --- | --- |
| Loading providers | Disabled / loading | Đóng | Stay at record. |
| No enabled Provider/runtime | Disabled | Đóng / Providers link | Stay at record. |
| Step 1, ready | Bắt đầu ghi âm | Huỷ | Start microphone, stay at record. |
| Step 1, recording `< min_clip_ms` | Dừng ghi âm (disabled) | Huỷ | Auto-stop only at max. |
| Step 1, recording `>= min_clip_ms` | Dừng ghi âm | Huỷ | Stop + begin capture upload. |
| Uploading | Đang xử lý… (disabled) | Đóng | Accepted/rejected becomes result. |
| Step 2, accepted | Tiếp tục nhập thông tin | Ghi âm lại | Move to details / reset record. |
| Step 2, rejected | Ghi âm lại | Đóng | Reset record. |
| Step 3, valid capture | Lưu người nói | Quay lại | Commit / return to result without invalidating capture. |
| Step 3, saving | Đang lưu… (disabled) | — | Wait for known result; protect against duplicate UI submits. |
| Step 3, confirmed commit | Xem chi tiết | Đóng | Existing detail route / refresh list. |

Use local `AbortController` for the capture upload (and commit if the current request client supports `signal`). On dialog close/unmount call `recorder.dispose()`, abort in-flight requests, clear capture/form refs and discard late responses. An aborted request can still finish on the server; accepted staging is cleaned by TTL. A commit might already have succeeded even if its response is lost; never claim that closing cancels the committed operation. While Save is in flight, disable ordinary wizard controls; if the modal closes due to Escape, overlay, navigation or teardown, stop the UI safely and refresh the Speaker list on return rather than assuming failure.

### 6.7 Vietnamese UI copy and accessibility

Recommended exact UX strings (put in existing i18n keys, not hard-coded template literals):

| Element / state | Copy |
| --- | --- |
| Main list action | `Thêm người nói` |
| Modal title | `Đăng ký giọng nói` |
| Subtitle | `Thu âm và kiểm tra mẫu trước khi nhập thông tin người nói.` |
| Steps | `Ghi âm` · `Kiểm tra` · `Thông tin` |
| Microphone prompt | `Hãy nói tự nhiên trong 5–10 giây ở nơi ít tiếng ồn.` (derive displayed times from config) |
| Mic denied/unavailable | `Không thể truy cập micro. Hãy cấp quyền trong trình duyệt rồi thử lại.` |
| Upload in progress | `Đang gửi và xử lý mẫu giọng nói…` |
| Accepted | `Mẫu giọng nói hợp lệ. Bạn có thể tiếp tục.` |
| No identity claim | `Mẫu đã được xử lý, nhưng danh tính chưa được xác minh.` |
| Rejected | `Mẫu chưa đạt yêu cầu. Hãy ghi âm lại.` |
| Next | `Tiếp tục nhập thông tin` |
| Details primary | `Lưu người nói` |
| Expired | `Mẫu đã hết hạn. Vui lòng ghi âm lại.` |
| Save complete | `Đã đăng ký người nói thành công.` |
| Voiceprint status | `Provisional — chưa xác minh đầy đủ` |
| Full Enrollment link | `Hoàn tất xác minh` |
| Detail action | `Xem chi tiết` |

Accessibility and browser behavior:

- Keep `BaseModal`'s existing dialog semantics and focus restoration. On each step transition focus the new step heading, first meaningful control or error summary. Use `aria-current="step"` on the visual progress indicator; never let a decorative stepper bypass capture validation.
- Assign labels to Provider, name and description. Use `role="status"`/`aria-live="polite"` for upload and success; use `role="alert"` for relevant errors, but **do not announce the recording timer every 100 ms**. Ensure keyboard-only operation for Start/Stop/Retry/Save and visible focus rings.
- The level indicator is decorative and must not be the only recording indicator; provide visible `Đang ghi âm` text and elapsed time. Icons do not replace labels. Avoid fixed colors as the only success/failure signal.
- Microphone access requires HTTPS or localhost. If access is denied, no audio is sent. Stop MediaStream tracks and AudioContext on closing, recording failure and unmount; revoke all Blob object URLs. Do not save the audio or capture token locally.

### 6.8 Recorder integration check — fix the raw-sample-rate cap

**Important implementation finding on `main`:** `useMicrophoneRecorder.ts::append` uses `limits.maxClipMs * TARGET_SAMPLE_RATE` (16 kHz) to cap **raw input frame count** before resampling, even though `AudioContext.sampleRate` may be 44.1/48 kHz. At a 48 kHz browser input, 10 seconds of wall-clock recording can be truncated to around 3.3 seconds of source audio before WAV encoding. This is directly relevant to Quick Enrollment and must be addressed before shipping the UI.

- Bound pre-resample raw input frame count with **the current `AudioContext.sampleRate`**, not the 16 kHz WAV output rate. Keep output WAV at PCM16/mono/16 kHz using the existing `resampleLinear`/`encodeWavPcm16` implementation.
- Alternatively resample each input chunk to 16 kHz before counting, but do **not** add that extra path unless necessary. The minimal change is the raw input rate bound.
- Test 44.1 kHz and 48 kHz input simulations, 5–10 s durations, early Stop, auto-stop, and the expected output WAV sample count; test that `stop()` produces a valid clip once even when a manual Stop races the auto-stop callback.
- Ensure neither Vue timer nor the browser's displayed waveform claims a valid 8-second sample when the encoded WAV contains substantially less audio. The server remains the definitive WAV/quality validator.

### 6.9 Frontend tests and completion checklist

Add focused tests to existing Vue/Vitest conventions (extend `api/speakers.test.ts`, `lib/wav.test.ts` and a view-local Quick modal test; do not create a broad screenshot-testing infrastructure):

1. One enabled Provider auto-selected; several Providers require selection; zero Providers disables Start; cold Provider still selectable.
2. Mic granted, real recorder timer/level shown, early Stop blocked, max-duration auto-stop triggers one upload with correct WAV + `If-Match`.
3. 201 accepted: Step 2 shows quality, no identity claim; metadata form is inaccessible before acceptance. A failed/422 capture offers Retry, not Save.
4. Permission denial and audio quality/runtime failures display readable Vietnamese text; no attempted commit and no false success.
5. Details validation preserves name/description on 400 or quota 409; expiry drops `capture_id` and returns to Step 1.
6. Double click Save, response loss, and idempotent 200 result never produce two Speaker UI entries or two commit intents. Success uses server-returned key.
7. Closing during recording/upload stops tracks, aborts/ignores late response, frees temporary Blob resources; navigating away has the same cleanup. No local persistence.
8. Mobile layout keeps provider, microphone, hints, errors and primary/secondary buttons visible; keyboard and screen-reader announcements work.
9. Speaker Detail preserves Full Enrollment, displays `pending` as `Provisional`, and can promote to `passed` without creating another Speaker profile.

**Web acceptance:** A user can open Speakers, select a Provider, record once, see the server's result, enter a name only after acceptance, save, and see an actual created Speaker — with no manual key, draft management, repeated sample slots, holdout or separate Prepare step in the Quick UI. Full Enrollment remains reachable through the existing Speaker Detail path.

## 7. Implementation file map (minimum change set)

| File | Required action |
| --- | --- |
| `crates/voice-agent-server/src/app/admin/mod.rs` | Register two Admin routes; inherit middleware and request caps. |
| `crates/voice-agent-server/src/app/admin/speaker_quick.rs` **new** | Own both operations, staging/commit logic, bounded inference coordination. |
| `crates/voice-agent-server/src/app/admin/speakers.rs` | Expose minimal `pub(super)` reusable helpers; account for accepted captures in shared quota; preserve Full finalize. |
| `crates/voice-agent-server/src/audio/enrollment.rs` | Reuse as-is if possible; factor out only genuinely duplicated logic. |
| `crates/voice-agent-server/src/database/speaker_candidate_set.rs` | Include `browser_validation_status` in projection and implement one Required eligibility predicate. |
| `crates/voice-agent-server/src/app/admin/speaker_calibration.rs` | Require eligibility + current exact-set evidence for `required`. |
| `crates/voice-agent-server/src/session/speaker_observe.rs` | Fail closed for pending/mixed required candidates; observe remains non-authoritative. |
| `crates/voice-agent-server/src/app/admin/speaker_policy.rs` | Surface existing qualification failure as clear blocker; avoid duplicate eligibility logic. |
| Next `crates/voice-agent-server/migrations/*_speaker_quick_captures.sql` | **Only** staging table/index; no new voiceprint assurance column. |
| Existing startup/retention sweep | Expire Quick accepted vectors and old committed tombstones with existing scheduler. |
| `apps/admin-web/src/views/SpeakersView.vue` | Primary Quick entry; secondary manual profile creation. |
| `apps/admin-web/src/components/speakers/QuickSpeakerEnrollment.vue` **new** | Three-step wizard. |
| `apps/admin-web/src/api/speakers.ts` + `api/types/speakers.ts` | Typed capture/commit methods; map existing `pending` status. |
| `apps/admin-web/src/pages/speakers/SpeakerDetailPage.vue` | Show provisional + upgrade to existing Full Enrollment. |
| `apps/admin-web/src/i18n/messages.ts` | New labels/errors/success copy. |
| `docs/api/00-all-apis.postman_collection.json` | Two routes, real sample WAV, headers, responses/error cases. |
| `docs/speaker-provider.md` and relevant ADR/runbook | Quick vs Full and unchanged Required calibration authority. |

Check all uses of `speaker_voiceprints` including direct SQL reads and tests; do not edit unrelated MCP authorization code or provider model loaders.

## 8. Rollout and verification

### P0 — Server, schema and Required gating (must ship before Web)

1. Implement **staging-only** forward migration, shared quota and reuse of `pending/passed` status.
2. Add two authenticated operations and atomic single-use commit with idempotent retry and TTL cleanup.
3. Fix exact candidate-set projection, qualification and session Required admission together; never expose Quick creation before this gate is tested.
4. Extend Rust HTTP + SQLite integration tests, including existing Full Enrollment promotion and live-session invalidation.

### P1 — Vue default Quick wizard

1. Add typed API calls; implement the single three-step module with existing mic recorder.
2. Make Quick primary in Speakers view; retain manual Create and Speaker Detail's Full Enrollment.
3. Add i18n, accessibility, status/upgrade guidance and Vue tests.

### P2 — Docs, Postman and regression

1. Update the Postman collection and `docs/speaker-provider.md`; reconcile `docs/adr/0077-speaker-v1-authority-and-calibration.md`, `docs/adr/0081-exact-candidate-set-qualification.md` and speaker runbooks only where behavior changed.
2. Run regression tests and an actual browser microphone flow; record unresolved limitations instead of declaring qualification from a single clip.

### Acceptance matrix (one source of truth)

| Scenario | Must hold |
| --- | --- |
| Valid WAV / enabled provider | Capture accepted; **zero** Speaker rows until Save; quality shown. |
| Malformed, oversized, silent/clipped WAV | No accepted capture; no Speaker; actionable Web error. |
| Provider disabled/stale/busy or runtime unavailable | Fail safely, no partial vector, no details step on error. |
| Expired/deleted-provider/cross-runtime capture | No Speaker; record again; 410/409 as appropriate. |
| Valid capture + metadata | Exactly one Speaker/one `pending` voiceprint, audit + catalog bump, no auto-grant. |
| Name invalid/quota full | No Speaker; accepted capture survives until TTL for correction/retry. |
| Concurrent/repeated commit, lost response | Never duplicate Speaker; same successful result on retry, or retryable busy. |
| Capture TTL / startup / cleanup | Vector purged; committed tombstones cleared after retention. |
| `observe` with explicit grant | May use Quick only for **non-authoritative** diagnostics when embedding space matches. |
| `required` with pending or mixed candidates | **Fails closed** at qualification/admission, including Template switch; no eligibility via old digest. |
| Full holdout/finalize on Quick Speaker | Same key; passed voiceprint; catalog revision and scoped security invalidation. |
| Existing Full/passed Speakers | No new assurance column/backfill; existing functionality stays valid, subject to exact-set qualification. |
| Web permission denial, abort, navigation and retry | Mic/worklet and preview cleaned; no false success; no duplicate commit. |
| Unauthenticated API | 401 and no audio/extraction or capture exposure. |

Suggested checks (adjust to actual target branch):

```bash
cargo fmt --all --check
cargo test -p voice-agent-server --test speaker_enrollment_api
cargo test -p voice-agent-server --test speaker_calibration_api
cargo test -p voice-agent-server --test agent_speaker_policy_api
cargo test -p voice-agent-server
cargo clippy --workspace --all-targets -- -D warnings
cd apps/admin-web
npm run typecheck
npm run test
npm run build
```

**Done** when P0–P2 tests pass, the browser flow works end-to-end, `required` remains strictly qualified, all docs/Postman are updated, and no replacement path is exposed before its server-side authority gate.

## 9. Explicitly rejected complexity (Ponytail + Codebase Design resolution)

| Earlier suggestion | Resolution and reason |
| --- | --- |
| New `enrollment_assurance` database column | **Cut.** Use existing `browser_validation_status`; one persisted truth. |
| Separate quick services + database repositories + handlers | **Cut.** One local module, two external operations; existing SQLite/runtime paths. |
| Rich duplicate `voiceprint`/`activation` response schema | **Cut.** Reuse existing `speaker_resource_value` serialization. |
| Stage provider key + quality fields | **Cut.** Provider FK/revision and extraction provenance suffice; return quality in API response. |
| New quota setting | **Cut.** Reuse existing `max_open_enrollments` across both Full/Quick active flows; preserve current runtime limiter. |
| Derived `mixed` summary state | **Cut.** Show per-voiceprint status in detail; simple summary in list. |
| Seven-step frontend state machine | **Cut.** Three visible steps plus local request/recorder flags. |
| Prepare as required wizard action | **Cut.** Acquisition remains behind the existing diagnostic/runtime interface. |
| Duplicate DoD and verification lists | **Cut.** One acceptance matrix above. |
| Extra broker, SSE, background extraction worker or new Web route | **Cut.** One bounded synchronous capture and atomic commit. |

**Do not cut:** Server-side `required` eligibility, canonical digest changes, short-lived biometric staging, atomic commit, idempotent retries, bounded inference, audit, and Full Enrollment upgrade. These enforce correctness/security rather than speculative flexibility.

**Implementation note:** Before editing code, recheck HEAD, highest SQLx migration, all raw `speaker_voiceprints` queries, the existing retention scheduler, and current tests. This guide is a design and update plan, not evidence that repository code has already changed.
