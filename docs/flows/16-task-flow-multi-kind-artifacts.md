# Flow 16 — Unified Task Flow (`orchestrator_flow_rs`) & Multi-Kind Task Artifacts

Tài liệu này mô tả luồng thực thi Task nền hợp nhất trên `voice-agent-server` sử dụng `orchestrator_flow_rs` (`TaskManager`) và `orchestrator_agent_rs` (`Orchestrator`), cùng cơ chế xuất bản, liệt kê và tải xuống **Multi-Kind Task Artifacts** (Báo cáo Markdown, Hình ảnh PNG/SVG, Âm thanh TTS WAV, và Dữ liệu JSON/CSV) qua các Subagent.

---

## 1. Kiến trúc Dual-Flow & Hệ thống Subagent trên `voice-agent-server`

`crates/voice-agent-server/src/services/task_flow/mod.rs` khởi tạo `TaskManager` và `Orchestrator` với 2 Flow chính:

### Flow 1 (`daily_root`) — Thu thập Thông tin & Trợ lý Đa chức năng
- **Root Agent (`daily_root`, `user_routable: true`)**: Chỉ có tool `subagent`, là điểm vào chuẩn duy nhất (`DEFAULT_USER_ROOT_AGENT_ID`) nhận toàn bộ Job từ người dùng / Voice Agent (`task_create`, `reminder_create`) và điều phối tới 5 Subagent chuyên biệt có ranh giới công cụ ghi (write seams) tách biệt:
  1. `source_collector`: Thu thập nguồn ngoài (`COLLECTOR_TOOL_NAMES`: YouTube Playlist qua `yt_list_playlist` / `yt_fetch_transcript`, tài liệu PDF qua `anydoc_inspect_pdf` / `anydoc_save_to_sources`, hoặc trang Web qua `fetch_html` / `html_extract_*`), phân loại chủ đề `<domain>/<topic>` và lưu file Markdown thô vào `sources/<domain>/<topic>/` (tự động kích hoạt Flow 2).
  2. `wiki_researcher`: Tra cứu chỉ-đọc kho tri thức Wiki (`wiki_query`, `wiki_get`, `wiki_tree`, `wiki_graph_neighbors`) và trích xuất trang Web (`fetch_html`, `html_extract_*`).
  3. `report_stager`: Tổng hợp nội dung thảo luận vào `sources/<domain>/<topic>/` qua `add_to_sources` hoặc xuất báo cáo độc lập qua `markdown_create` / `markdown_read`.
  4. `media_creator`: Chuyên gia tạo hình ảnh minh họa/sơ đồ (`image_create` cho `image/png` và `image/svg+xml`), tổng hợp giọng nói TTS (`tts_synthesize` cho `audio/wav`), và xuất dữ liệu cấu trúc (`artifact_create`).
  5. `reminder_planner`: Quản lý lịch hẹn và lời nhắc (`reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`).

### Flow 2 (`wiki_root`) — Đường ống Biên soạn Tri thức Karpathy LLM Wiki Tự động
- **Root Agent (`wiki_root`, `user_routable: false`)**: Tự động kích hoạt qua `WikiAutoFlowBinding` khi phát hiện có file mới thêm vào `sources/`, điều phối tuần tự 2 Subagent (`wiki_ingestor`, `wiki_curator`) để biên soạn trang Wiki, tổng hợp Synthesis và kiểm toán `wiki_lint` (hoàn toàn không chứa tool thu thập ngoại vi).
- **Bảo vệ Điều hướng (`VoiceTaskSubmit`)**:
  - `VoiceTaskSubmit::tool_definitions()` sử dụng `is_user_routable_root_agent` để chỉ hiển thị các Root Agent có `user_routable == true` (`daily_root`) trong schema `target_agent`.
  - `VoiceTaskSubmit::execute()` sử dụng `normalize_user_target_agent` để tự động chuyển hướng mọi lệnh gọi legacy (`task_create` hoặc `reminder_create`) có `target_agent = "wiki_root"` (hoặc `"<flow>/wiki_root"`) sang `"daily_root"`.
- **Đồng bộ Prompts & Cấu hình Lưu trữ**:
  - Toàn bộ hằng số System Prompt & Skill chuẩn (`DAILY_ROOT_PROMPT`, `WIKI_ROOT_PROMPT`, `MEDIA_CREATOR_PROMPT`, `DAILY_SYNTHESIS_STAGING_SKILL`, ...) được dùng chung từ `orchestrator_flow_rs::prompts`.
  - Khi nạp `agent_settings.json`, `load_persisted_settings` tự động lọc bỏ các `allowed_tools` và `allowed_skills` không còn nằm trong `spec.available_tools` / `spec.available_skills`.

---

## 2. Luồng Xuất bản & Truy vấn Multi-Kind Task Artifacts

1. **Xuất bản từ Subagent (`HostTool`)**:
   - Khi `media_creator` hoặc `report_stager` gọi `markdown_create`, `image_create`, `tts_synthesize`, hoặc `artifact_create`, công cụ thực thi `ctx.artifacts().put(ArtifactInput::from_bytes(...))`.
   - `orchestrator_agent_rs::ArtifactStore` ghi dữ liệu vào `staging/`, tính SHA-256, `fsync`, đổi tên nguyên tử sang `committed/<storage_key>`, và lưu bản ghi vào bảng `artifacts` gắn với `root_session_id = task.session_id`.
2. **Tổng hợp trong `TaskDetailView` & `AdminJob` (`projections.rs`)**:
   - Khi Admin UI gọi `GET /api/admin/jobs/{id}`, `tm.get_task_detail_any_scope(&id)` truy vấn đồng thời `FamilySnapshot` và `Orchestrator::list_family_artifacts` cho `task.session_id`.
   - `project_task_to_admin_job` ánh xạ `detail.artifacts` sang mảng JSON `"artifacts"`:
     ```json
     {
       "id": "art-...",
       "artifact_id": "art-...",
       "task_id": "<job_id>",
       "name": "b200-voiceover.wav",
       "mime_type": "audio/wav",
       "kind": "audio",
       "role": "primary",
       "size_bytes": 16044,
       "sha256": "<sha256_hex>",
       "producer_session_id": "<child_session_id>",
       "producer_agent_id": "media_creator",
       "producer_tool_name": "tts_synthesize",
       "download_url": "/api/admin/jobs/<job_id>/artifacts/art-..."
     }
     ```
   - Đồng thời `"budget_used"` bổ sung các chỉ số: `total_artifacts`, `primary_artifacts`, `report_artifacts`, `image_artifacts`, `audio_artifacts`, `artifact_ids`.
3. **Tải xuống có xác minh SHA-256 (`GET /api/admin/jobs/{id}/artifacts/{artifact_id}`)**:
   - Handler `artifact` trong `crates/voice-agent-server/src/app/admin/background.rs` gọi `tm.read_task_artifact_bytes_any_scope(&id, &artifact_id)`.
   - Hệ thống kiểm tra `artifact_id` phải thuộc đúng `root_session_id` của Task `{id}`, xác minh kích thước file và chữ ký SHA-256 trên dòng dữ liệu đọc ra, rồi trả về HTTP `200 OK` kèm `Content-Type`, `Content-Disposition: inline; filename="..."` và `ETag: "<sha256>"`.
