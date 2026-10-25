# Flow 16 — Unified Task Flow (`orchestrator_flow_rs`) & Multi-Kind Task Artifacts

Tài liệu này mô tả luồng thực thi Task nền hợp nhất trên `voice-agent-server` sử dụng `orchestrator_flow_rs` (`TaskManager`) và `orchestrator_agent_rs` (`Orchestrator`), cùng cơ chế xuất bản, liệt kê và tải xuống **Multi-Kind Task Artifacts** (Báo cáo Markdown, Hình ảnh PNG/SVG, Âm thanh TTS WAV, và Dữ liệu JSON/CSV) qua các Subagent.

---

## 1. Kiến trúc Dual-Flow & Hệ thống Subagent trên `voice-agent-server`

`crates/voice-agent-server/src/services/task_flow/mod.rs` khởi tạo `TaskManager` và `Orchestrator` với 2 Flow chính:

### Flow 2 (`daily_root`) — Trợ lý Hằng ngày & Xuất bản Đa Phương Tiện
- **Root Agent (`daily_root`)**: Chỉ có tool `subagent`, điều phối công việc tới 4 Subagent chuyên biệt:
  1. `wiki_researcher`: Tra cứu chỉ-đọc kho tri thức Wiki (`wiki_query`, `wiki_get`, `wiki_tree`, `wiki_graph_neighbors`) và trích xuất trang Web (`fetch_html`, `html_extract_*`).
  2. `report_stager`: Tổng hợp nội dung thảo luận vào `sources/<domain>/<topic>/` qua `add_to_sources`, xuất báo cáo độc lập qua `markdown_create`, hoặc đính kèm hình ảnh/âm thanh (`image_create`, `tts_synthesize`, `artifact_create`).
  3. `media_creator`: Chuyên gia tạo hình ảnh minh họa/sơ đồ (`image_create` cho `image/png` và `image/svg+xml`), tổng hợp giọng nói TTS (`tts_synthesize` cho `audio/wav`), và xuất dữ liệu cấu trúc (`artifact_create`, `markdown_create`).
  4. `reminder_planner`: Quản lý lịch hẹn và lời nhắc (`reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`).

### Flow 1 (`wiki_root`) — Đường ống Biên soạn Tri thức Karpathy LLM Wiki
- **Root Agent (`wiki_root`)**: Điều phối tuần tự 3 Subagent (`source_collector`, `wiki_ingestor`, `wiki_curator`) để thu thập nguồn ngoài (PDF, YouTube Playlist, Web), biên soạn trang Wiki và kiểm toán `wiki_lint`.

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
