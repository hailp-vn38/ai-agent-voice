# Chọn Giọng Động (Nam/Nữ) theo Provider cho Công cụ `tts_synthesize` — Hướng dẫn Triển khai

- **Repositories**:
  - `orchestrator_task_rs` (`src/tools/media_tts.rs` *(tách mới)*, `src/tools/media_artifacts.rs`, `src/tools/mod.rs`, `src/lib.rs`, `src/prompts.rs`, `src/tools/media_artifacts_tests.rs`)
  - `AI_Agent_Voice_rust` (`crates/voice-agent-server/src/providers/tts/voice_catalog.rs` *(tạo mới)*, `crates/voice-agent-server/src/providers/tts/`, `crates/voice-agent-server/src/workers/tts/diagnostic.rs`, `crates/voice-agent-server/src/services/task_flow/agent_settings/`)
- **Kỷ luật Thiết kế**: [`/codebase-design`](file:///Users/lamphuchai/Desktop/project_rust/.agents/skills/codebase-design/SKILL.md) (**Module**, **Interface**, **Implementation**, **Depth**, **Seam**, **Adapter**, **Leverage**, **Locality**), [`DEEPENING.md`](file:///Users/lamphuchai/Desktop/project_rust/.agents/skills/codebase-design/DEEPENING.md) (*Replace, Don't Layer*) & Từ vựng Miền ([`orchestrator_task_rs/GLOSSARY.md`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/GLOSSARY.md), [`AI_Agent_Voice_rust/GLOSSARY.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/GLOSSARY.md))
- **Mục tiêu cốt lõi**:
  1. **Dynamic `TtsProviderConstraints` qua Seam `TtsSynthesizerBackend`**: Khi `tts_synthesize` được gắn với bất kỳ TTS Provider nào (`vieneu`, `zerotts_onnx`, `edge_tts`, `kokoro_vi_onnx`, `chillaudio_ws`, `vifasttts_onnx`, hoặc `openai`), tự động phản ánh danh sách giọng Nữ (`female_voices`), giọng Nam (`male_voices`), giọng mặc định (`default_voice`) và hướng dẫn chọn giọng ngay trong `ToolDescriptor` của `tts_synthesize` (cũng như mô tả subagent `media_processor` cho `daily_root`), mà không để rò rỉ khái niệm vendor vào `orchestrator_task_rs`.
  2. **Làm sâu (Deepen) Phân giải Giọng Đọc & Giới tính tại một Nguồn Chân lý Duy nhất (`providers::tts::voice_catalog`)**: Xử lý trọn vẹn cả 2 trường hợp: (a) Subagent truyền đúng mã `voice` của provider hiện hành (ví dụ `"ngoc_huyen"`, `"vi-VN-HoaiMyNeural"`, `"maichi"`, `"nova"`), và (b) Subagent truyền nhãn giới tính ngữ nghĩa (`"female"`, `"vi-female"`, `"nữ"`, `"giọng nữ"`, `"male"`, `"vi-male"`, `"nam"`, `"giọng nam"`). Nếu `default_voice` đang cấu hình trên provider đã khớp giới tính yêu cầu thì giữ nguyên; nếu lệch giới tính thì tự động chuyển sang giọng chuẩn đầu tiên khớp giới tính của provider đó.
  3. **Mở khóa Đa giọng Động (Dynamic Per-Request Voice) cho `kokoro_vi_onnx` và `chillaudio_ws`**: Nâng cấp `ConfiguredKokoroVi` nạp bảng `BTreeMap<String, Voicepack>` cho toàn bộ 14 giọng có sẵn trên đĩa và cập nhật `ChillAudioWsProvider` (cả `validate_diagnostic`, `synthesize_diagnostic` lẫn `ChillAudioWsWorker::synthesize`) để cả hai adapter đều hỗ trợ đổi giọng nam/nữ theo từng request mà không cần khởi tạo lại worker hay session ONNX.

---

## 0. Quyết định Thiết kế Cốt lõi (`/codebase-design` & `DEEPENING.md`)

| # | Bẫy Thiết kế Nông (Shallow Trap) | Quyết định Làm sâu Module (Deepened Decision) |
| :--- | :--- | :--- |
| **1** | **Hardcode danh sách tên giọng của từng hãng trong prompt tĩnh (`prompts.rs`)**: Nếu ghi cứng *"Khi dùng VieNeu hãy chọn `ngoc_huyen`, khi dùng Edge TTS hãy chọn `vi-VN-HoaiMyNeural`"* trong `prompts.rs` của `orchestrator_task_rs`, thiết kế sẽ vi phạm tính cô lập của core crate (rò rỉ chi tiết **Adapter** của `AI_Agent_Voice_rust`) và khiến LLM chọn sai tên giọng của hãng khác so với provider đang kích hoạt. | **Dynamic `TtsProviderConstraints` qua Seam `TtsSynthesizerBackend`**: Mở rộng `trait TtsSynthesizerBackend` với `fn tts_constraints(&self) -> TtsProviderConstraints` (đồng nhất kiến trúc với `VideoGeneratorBackend::video_constraints()`), đồng thời tách công cụ TTS sang `orchestrator_task_rs/src/tools/media_tts.rs` (đồng bộ với `media_image.rs` và `media_video.rs`). `TtsSynthesizeTool::descriptor()` tự động nhúng danh sách giọng Nam/Nữ của provider đang active vào `description` và `input_schema.properties.voice`. |
| **2** | **Thêm các tham số mới (`gender`, `voice_preset`, `provider_voice_id`) vào JSON Schema của `tts_synthesize`**: Việc tách thêm trường `gender: "female" \| "male"` cạnh trường `voice` làm phình **Interface** của `TtsSynthesizeTool` và tạo trạng thái mâu thuẫn khi LLM truyền `voice="hai_dang"` nhưng `gender="female"`. | **Giữ Small Interface (`voice: string`), tăng Depth bên trong Implementation**: Giữ nguyên tập thuộc tính hiện hữu của `tts_synthesize` (`text`, `filename`, `voice`, `language`, `speed`, `role`). Trường `voice` vừa nhận mã giọng cụ thể của provider (được quảng bá rõ trong `descriptor()`), vừa nhận alias giới tính (`"female"`, `"vi-female"`, `"nữ"`, `"male"`, `"vi-male"`, `"nam"`). Một **Interface** nhỏ mang lại **Leverage** tối đa cho cả Agent lẫn test. |
| **3** | **Đặt bảng danh mục giọng ở `services::task_flow` khiến `workers::tts` không thể tái sử dụng (Vi phạm `Locality`)**: Nếu đặt logic phân loại giọng Nam/Nữ trong `ResolvedLlmProvider` (`services::task_flow`), tầng thấp hơn là `TtsWorkerRuntime` (`workers::tts`) không thể gọi ngược lên `services::task_flow`, dẫn đến trùng lặp logic hoặc âm thầm nuốt lỗi `UnsupportedVoice` rồi rơi về `self.binding.voice` tại [`diagnostic.rs:109-124`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/diagnostic.rs#L109-L124). | **Đặt `voice_catalog` tại tầng `providers::tts` (`Locality` tuyệt đối)**: Tạo module `crates/voice-agent-server/src/providers/tts/voice_catalog.rs` làm nguồn chân lý duy nhất cho danh mục giọng Nam/Nữ và hàm `resolve_tts_voice(adapter, model_or_voice_spec, runtime_default_voice, requested_voice)`. Cả `TtsWorkerRuntime` (`workers::tts`) lẫn `ResolvedLlmProvider` / `CatalogMediaBackend` (`services::task_flow`) đều ủy quyền về `providers::tts::voice_catalog`. |
| **4** | **Chỉ nạp 1 file `Voicepack` duy nhất vào `ConfiguredKokoroVi` hoặc giữ trường cũ chồng chéo**: [`factory_registry.rs:589-600`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/factory_registry.rs#L589-L600) hiện chỉ nạp đúng 1 `Voicepack` theo `options.voice`, còn `ConfiguredKokoroVi` và `ChillAudioWsProvider` chưa cài đặt `validate_diagnostic` / `synthesize_diagnostic` (luôn trả về `Err(DiagnosticOverrideUnsupported)` khi có `request.voice`). | **Áp dụng *"Replace, Don't Layer"* cho `KokoroViArtifacts` & mở khóa `validate_diagnostic` + `synthesize_diagnostic`**: Thay thế `voicepack: &'a Path` trong `KokoroViArtifacts<'a>` bằng `voicepacks: &'a BTreeMap<String, PathBuf>`, cập nhật dứt khoát cả 2 call sites ([`factory_registry.rs:596`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/factory_registry.rs#L596) và [`kokoro_vi/mod.rs:165`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs#L165)), nạp `Arc<BTreeMap<String, Voicepack>>` một lần trong `ConfiguredKokoroVi::load`, và cài đặt đầy đủ `validate_diagnostic` + `synthesize_diagnostic` cho cả `ConfiguredKokoroVi` và `ChillAudioWsProvider`. |

---

## 1. Kiến trúc Tổng thể, Các Đường nối (Seams) & Danh mục Giọng Đọc

```mermaid
flowchart TD
    Root["Root Orchestrator: daily_root<br/>Nhận yêu cầu người dùng (VD: 'Tạo file âm thanh giọng nữ...')<br/>Chuyển Audio Brief kèm yêu cầu giới tính/giọng đọc"]

    subgraph Seam1["SEAM 1: Sub-Agent & Tool Interface (`orchestrator_task_rs/src/tools/media_tts.rs`)"]
        MP["Sub-Agent: `media_processor`<br/>Nhìn thấy danh sách `female_voices` & `male_voices`<br/>ngay trong `TtsSynthesizeTool::descriptor()`"]
        TTS["TtsSynthesizeTool (`tts_synthesize`)<br/>• Dynamic `descriptor()` từ `backend.tts_constraints()`<br/>• Giữ nguyên Small Interface: `voice: string`"]
        TSB["trait TtsSynthesizerBackend<br/>+ tts_constraints(&self) -> TtsProviderConstraints<br/>+ synthesize_tts(&self, req) -> Result<SynthesizedAudioPayload, String>"]
    end

    Root -->|"subagent(agent='media_processor', prompt='... giọng nữ ...')"| MP
    MP -->|"tts_synthesize(text='...', voice='ngoc_huyen' hoặc 'female')"| TTS
    TTS --> TSB

    subgraph Seam2["SEAM 2: Provider Resolution & Shared Voice Catalog (`AI_Agent_Voice_rust`)"]
        CMB["CatalogMediaBackend (`services::task_flow`)<br/>• Ủy quyền `tts_constraints()`<br/>• Phân giải `effective_voice` & gọi `ServerMediaRuntimeBridge` / `LlmProvider`"]
        RLP["ResolvedLlmProvider::tts_constraints()<br/>& resolve_tts_voice()"]
        VC["`providers::tts::voice_catalog` (Single Source of Truth)<br/>• Danh mục giọng Nam/Nữ của 7 adapter<br/>• `resolve_tts_voice(adapter, model_spec, default_voice, requested)`"]
        TWR["TtsWorkerRuntime (`workers::tts::diagnostic`)<br/>• `resolve_background_voice()` ủy quyền về `voice_catalog`"]
    end

    TSB -. implemented by .-> CMB
    CMB --> RLP
    CMB --> TWR
    RLP --> VC
    TWR --> VC

    subgraph Adapters["Tầng TTS Adapters (Hỗ trợ Đổi Giọng Động theo Request)"]
        direction LR
        VN["`vieneu` (Turbo 25 giọng / Nano 11 giọng)<br/>Tra cứu `contract.voices.get(&selection.voice)`"]
        ZT["`zerotts_onnx` (8 giọng)<br/>Tra cứu `contract.voice(&selection.voice)`"]
        ED["`edge_tts` (Quảng bá 6 giọng chính / Chấp nhận 322 giọng)<br/>Ghi đè `effective.voice = selection.voice`"]
        KK["`kokoro_vi_onnx` (14 giọng)<br/>Tra cứu `voicepacks.get(&selection.voice)`"]
        CA["`chillaudio_ws` (4 giọng)<br/>Truyền `speaker = selection.voice`"]
        VF["`vifasttts_onnx` (1 giọng nữ `ngoc_huyen`)"]
        OA["`openai` / `llm_gateway`<br/>POST `/v1/audio/speech` với `voice`"]
    end

    TWR --> VN & ZT & ED & KK & CA & VF
    CMB --> OA
```

### Bảng Phân loại Giọng Nữ / Giọng Nam Quảng bá (`TtsProviderConstraints`) và Chấp nhận (`resolve_tts_voice`)

> [!IMPORTANT]
> Với `edge_tts`, bảng `edge_tts::voices::VOICES` có tới **322 giọng** đa ngôn ngữ. Để tránh làm phình `ToolDescriptor` và context prompt của Agent hàng chục KB mỗi lượt chạy, `TtsProviderConstraints` chỉ **quảng bá** 6 giọng `vi-VN` và `en-US` tiêu biểu, trong khi `voice_catalog::resolve_tts_voice` và `EdgeTtsProvider::validate_diagnostic` vẫn **chấp nhận toàn bộ 322 giọng** hợp lệ khi Subagent truyền đích danh mã giọng.

| Adapter (`adapter`) | Model (`model`) | Giọng Nữ Mặc định (Khi yêu cầu Nữ) | Danh sách Giọng Nữ Quảng bá (`female_voices`) | Giọng Nam Mặc định (Khi yêu cầu Nam) | Danh sách Giọng Nam Quảng bá (`male_voices`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`vieneu`** *(Turbo)* | `vieneu_v3_turbo_fp32` *(mặc định)* | `ngoc_huyen` | `ngoc_huyen`, `truc_ly`, `mai_anh`, `thuy_dung`, `ngoc_tran`, `ngoc_linh`, `doan_trang`, `thuc_doan`, `my_duyen`, `quynh_anh`, `kim_thanh` *(11 giọng nữ)* | `hai_dang` | `hai_dang`, `thien_minh`, `thien_tam_duc`, `quang_son`, `minh_duc`, `pham_tuyen`, `thai_son`, `xuan_vinh`, `thanh_binh`, `minh_triet`, `duc_tri`, `adam`, `adam_bua`, `quoc_tuan` *(14 giọng nam)* |
| **`vieneu`** *(Nano)* | `vieneu_v3_nano_fp32` | `my_duyen` | `my_duyen`, `mai_anh`, `truc_ly`, `ai_han` *(4 giọng nữ)* | `minh_quan` | `minh_quan`, `adam`, `duc_tri`, `huu_quan`, `xuan_tien`, `anh_khoi`, `manh_dung` *(7 giọng nam)* |
| **`zerotts_onnx`** | `zerotts_onnx` | `maichi` | `maichi`, `baotrang`, `hamy`, `kimoanh` | `giahuy` | `giahuy`, `huuduc`, `quangminh`, `tiendat` |
| **`edge_tts`** | `edge_tts` | `vi-VN-HoaiMyNeural` | `vi-VN-HoaiMyNeural`, `en-US-AriaNeural`, `en-US-JennyNeural` *(quảng bá 3 giọng chính; `resolve_tts_voice` chấp nhận toàn bộ voice `Female` trong `edge_tts::voices::VOICES`)* | `vi-VN-NamMinhNeural` | `vi-VN-NamMinhNeural`, `en-US-GuyNeural`, `en-US-ChristopherNeural` *(quảng bá 3 giọng chính; `resolve_tts_voice` chấp nhận toàn bộ voice `Male` trong `edge_tts::voices::VOICES`)* |
| **`kokoro_vi_onnx`** | `kokoro_vi_contextbox` | `ngoc_huyen` | `diem_trinh`, `mai_linh`, `mai_loan`, `my_yen`, `ngoc_huyen`, `thuc_trinh` | `manh_dung` | `duc_an`, `duc_duy`, `hung_thinh`, `manh_dung`, `phat_tai`, `thanh_dat`, `tuan_ngoc`, `storyvert` |
| **`chillaudio_ws`** | `chillaudio_ws` | `BV421_vivn_streaming` | `BV421_vivn_streaming` *(Nữ nhẹ nhàng)*, `vi_female_huong` *(Nữ trầm ấm)*, `BV074_streaming` *(Nữ cá tính)* | `BV075_streaming` | `BV075_streaming` *(Nam ấm áp)* |
| **`vifasttts_onnx`** | `vifasttts_ngochuyen_small` | `ngoc_huyen` | `ngoc_huyen` *(Ngọc Huyền — model đơn giọng nữ)* | `ngoc_huyen` *(fallback)* | *(Rỗng — model chỉ huấn luyện 1 giọng nữ)* |
| **`openai` / `llm_gateway`** | `tts-1`, `gpt-4o-mini-tts`, ... | `nova` | `nova`, `shimmer`, `coral`, `sage`, `alloy` | `onyx` | `onyx`, `echo`, `ash`, `ballad`, `fable` |

---

## 2. Chi tiết Triển khai tại `orchestrator_task_rs`

### 2.1. Tách Module `media_tts.rs`, Định nghĩa `TtsProviderConstraints` & Mở rộng `TtsSynthesizerBackend`

- **Files**:
  - `orchestrator_task_rs/src/tools/media_tts.rs` *(tạo mới — tách phần TTS từ `media_artifacts.rs` để giữ `media_artifacts.rs` gọn nhẹ, đồng bộ với `media_image.rs` và `media_video.rs`)*
  - [`orchestrator_task_rs/src/tools/media_artifacts.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/media_artifacts.rs#L75-L310) *(re-export các kiểu từ `media_tts`)*
  - [`orchestrator_task_rs/src/tools/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/mod.rs#L17-L46)
  - [`orchestrator_task_rs/src/lib.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/lib.rs#L80-L90)

#### A. Thêm struct `TtsProviderConstraints` và trường `voice` thực tế trong `SynthesizedAudioPayload`

Trong `orchestrator_task_rs/src/tools/media_tts.rs`, định nghĩa `TtsProviderConstraints` (hoàn toàn độc lập với tên vendor cụ thể ở tầng core crate), bổ sung `pub voice: String` vào `SynthesizedAudioPayload`, và mở rộng `trait TtsSynthesizerBackend`:

```rust
/// Thông tin giọng đọc và giới hạn theo provider hiện hành của công cụ `tts_synthesize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsProviderConstraints {
    /// Nhãn hiển thị của provider/adapter đang kích hoạt (ví dụ: `"VieNeu Local TTS (model: vieneu_v3_turbo_fp32)"`).
    pub provider_label: String,
    /// Mã giọng mặc định đang được cấu hình trên provider (ví dụ: `"hai_dang"`, `"vi-VN-HoaiMyNeural"`).
    pub default_voice: String,
    /// Danh sách mã giọng Nữ (`female`) được quảng bá cho Subagent trên provider hiện hành.
    pub female_voices: Vec<String>,
    /// Danh sách mã giọng Nam (`male`) được quảng bá cho Subagent trên provider hiện hành.
    pub male_voices: Vec<String>,
    /// Hướng dẫn chọn giọng cụ thể cho Subagent khi gọi `tts_synthesize`.
    pub provider_guidance: String,
}

impl Default for TtsProviderConstraints {
    fn default() -> Self {
        Self {
            provider_label: "builtin_wav_synthesizer".to_string(),
            default_voice: "vi-default".to_string(),
            female_voices: vec!["vi-female".to_string()],
            male_voices: vec!["vi-male".to_string()],
            provider_guidance:
                "Pass a female voice ID for female narration or a male voice ID for male narration."
                    .to_string(),
        }
    }
}

/// Kết quả âm thanh sinh ra từ [`TtsSynthesizerBackend`].
#[derive(Debug, Clone)]
pub struct SynthesizedAudioPayload {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub sample_rate_hz: u32,
    pub duration_ms: u64,
    pub provider: String,
    /// Mã giọng thực tế đã sử dụng để tổng hợp (sau khi phân giải theo provider/giới tính).
    pub voice: String,
}

#[async_trait::async_trait]
pub trait TtsSynthesizerBackend: Send + Sync {
    fn tts_constraints(&self) -> TtsProviderConstraints {
        TtsProviderConstraints::default()
    }

    async fn synthesize_tts(
        &self,
        req: &TtsSynthesisRequest,
    ) -> Result<SynthesizedAudioPayload, String>;
}
```

> [!WARNING]
> Khi thêm `pub voice: String` vào `SynthesizedAudioPayload`, cần cập nhật đầy đủ **cả 3 nơi khởi tạo struct literal** trên 2 repo:
> 1. `DefaultTtsSynthesizerBackend::synthesize_tts` tại `orchestrator_task_rs/src/tools/media_tts.rs` (chuyển từ [`media_artifacts.rs:133`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/media_artifacts.rs#L133)): gán `voice: req.voice.clone()`.
> 2. `CatalogMediaBackend::synthesize_via_server_tts` tại [`media_backend.rs:433`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs#L433): gán `voice: effective_voice`.
> 3. `CatalogMediaBackend::synthesize_tts` (nhánh `LlmOperation::AudioSpeech`) tại [`media_backend.rs:461`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs#L461): gán `voice: effective_voice`.

#### B. Cập nhật `TtsSynthesizeTool::descriptor(&self)` và `execute(&self)`

Vì `ToolExecutor::definitions()` trong `orchestrator_agent_rs` đã gọi `tool.descriptor()` trực tiếp ở mỗi lượt chạy (live lookup), `TtsSynthesizeTool::descriptor()` chỉ cần đọc `self.backend.tts_constraints()` để nhúng danh sách `female_voices`, `male_voices`, và `default_voice` vào schema:

```rust
#[async_trait::async_trait]
impl HostTool for TtsSynthesizeTool {
    fn descriptor(&self) -> ToolDescriptor {
        let c = self.backend.tts_constraints();
        let female_list = if c.female_voices.is_empty() {
            "none".to_string()
        } else {
            c.female_voices.join(", ")
        };
        let male_list = if c.male_voices.is_empty() {
            "none".to_string()
        } else {
            c.male_voices.join(", ")
        };

        ToolDescriptor {
            name: "tts_synthesize".to_string(),
            description: format!(
                "Synthesize standalone spoken audio (TTS WAV) from text when the user requests an audio-only artifact. \
                 Do NOT call `tts_synthesize` to create audio for a video (`video_create` already generates video with integrated voiceover audio and subtitles). \
                 [Active provider: {label} | default_voice='{default_voice}' | Female voices: [{female_list}] | Male voices: [{male_list}]. {guidance}]",
                label = c.provider_label,
                default_voice = c.default_voice,
                guidance = c.provider_guidance,
            ),
            input_schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text script to synthesize into spoken audio."
                    },
                    "filename": {
                        "type": "string",
                        "description": "Output audio filename (e.g. 'summary-tts.wav'). Defaults to 'tts-output.wav'."
                    },
                    "voice": {
                        "type": "string",
                        "description": format!(
                            "Voice identifier for active provider '{}' (default: '{}'). \
                             For a FEMALE voice, choose one of: [{}] (or pass 'female'). \
                             For a MALE voice, choose one of: [{}] (or pass 'male').",
                            c.provider_label,
                            c.default_voice,
                            female_list,
                            male_list,
                        )
                    },
                    "language": {
                        "type": "string",
                        "description": "BCP-47 language code (default: 'vi-VN')."
                    },
                    "speed": {
                        "type": "number",
                        "description": "Playback speed multiplier (0.5 to 2.0, default: 1.0)."
                    },
                    "role": {
                        "type": "string",
                        "enum": ["primary", "intermediate"],
                        "description": "Artifact visibility role (default: 'primary')."
                    }
                },
                "additionalProperties": false
            }),
            effect: ToolEffect::Mutating,
        }
    }
```

Và trong `TtsSynthesizeTool::execute`:
- Nếu caller không truyền `voice` (hoặc truyền chuỗi rỗng), lấy `self.backend.tts_constraints().default_voice` làm mặc định.
- Sử dụng `synthesized.voice` (nếu không rỗng, fallback về `voice`) trong cả summary string của `ArtifactInput` lẫn kết quả JSON trả về cho Subagent để hiển thị chính xác giọng đã được tổng hợp.

---

### 2.2. Cập nhật System Prompts trong `orchestrator_task_rs/src/prompts.rs`

- **File**: [`orchestrator_task_rs/src/prompts.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/prompts.rs)

1. **Trong `DAILY_ROOT_PROMPT` ([`prompts.rs:56-62`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/prompts.rs#L56-L62) & [`76`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/prompts.rs#L76))**:
   - Bổ sung quy tắc khi ủy quyền cho `media_processor` tạo file âm thanh TTS:
     > *"When delegating standalone TTS synthesis (`tts_synthesize`) to `media_processor`, ALWAYS forward the user's requested voice gender or speaker preference verbatim in the delegated prompt (e.g., 'giọng nữ' / female voice, 'giọng nam' / male voice, or a specific voice name)."*
2. **Trong `MEDIA_PROCESSOR_PROMPT` ([`prompts.rs:148-151`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/prompts.rs#L148-L151))**:
   - Cập nhật mục `## 2. Creating Standalone Spoken TTS Audio (tts_synthesize — Audio-Only Requests)`:
     ```markdown
     ## 2. Creating Standalone Spoken TTS Audio (`tts_synthesize` — Audio-Only Requests)
     - **ONLY** call `tts_synthesize` when the user explicitly requests a standalone audio/voiceover file (`.wav`) without creating a video.
     - **Voice & Gender Selection (`voice`)**: Inspect the active provider's `Female voices: [...]`, `Male voices: [...]`, and `default_voice` advertised in the `tts_synthesize` tool description:
       - When the prompt requests a **female voice** (*giọng nữ*), select a voice ID from the advertised `Female voices` list (or pass `"female"`).
       - When the prompt requests a **male voice** (*giọng nam*), select a voice ID from the advertised `Male voices` list (or pass `"male"`).
       - When a specific speaker ID/name is requested and matches an advertised voice, pass that exact voice ID; otherwise omit `voice` (or use `default_voice`).
     - Call `tts_synthesize` with `text`, `filename` (e.g. `"summary-tts.wav"`), the selected `voice`, `language` (`"vi-VN"`), and optional `speed`.
     ```

---

## 3. Chi tiết Triển khai tại `AI_Agent_Voice_rust`

### 3.1. Mở khóa Đa giọng Động cho `kokoro_vi_onnx` và `chillaudio_ws`

#### A. Nâng cấp `kokoro_vi_onnx` nạp toàn bộ bảng `Voicepack` ("Replace, Don't Layer") và hỗ trợ `validate_diagnostic` + `synthesize_diagnostic`

- **Files**:
  - [`crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs)
  - [`crates/voice-agent-server/src/providers/tts/kokoro_vi/runtime.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/runtime.rs)
  - [`crates/voice-agent-server/src/providers/factory_registry.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/factory_registry.rs#L569-L605)

**Thiết kế chi tiết**:
1. **Thay thế dứt khoát (`Replace, Don't Layer`) trong `KokoroViArtifacts<'a>`** ([`kokoro_vi/mod.rs:26-30`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs#L26-L30)):
   - Thay `pub(crate) voicepack: &'a Path` bằng `pub(crate) voicepacks: &'a std::collections::BTreeMap<String, std::path::PathBuf>`:
     ```rust
     pub(crate) struct KokoroViArtifacts<'a> {
         pub(crate) model: &'a Path,
         pub(crate) config: &'a Path,
         pub(crate) voicepacks: &'a std::collections::BTreeMap<String, std::path::PathBuf>,
     }
     ```
   - Cập nhật cả **2 call sites** duy nhất trong repo:
     - [`factory_registry.rs:588-601`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/factory_registry.rs#L588-L601): Kiểm tra `assets.voicepack(&options.voice)` tồn tại, sau đó truyền `voicepacks: &assets.voicepacks` vào `KokoroViArtifacts`.
     - [`kokoro_vi/mod.rs:165-169`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs#L165-L169) (trong `real_model_tests::loads_local_contextbox_model_and_synthesizes_24khz_pcm`): Tạo `BTreeMap::from([("diem_trinh".to_owned(), root.join("voicepacks/diem_trinh.bin"))])` và truyền `&voicepacks`.
2. **Trong `ConfiguredKokoroVi`** ([`kokoro_vi/mod.rs:33-131`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs#L33-L131)):
   - Lưu `default_voice: String`, `language: String`, và `voicepacks: std::sync::Arc<std::collections::BTreeMap<String, Voicepack>>` (nạp toàn bộ các entry trong `artifacts.voicepacks` và đảm bảo `voicepacks.contains_key(&options.voice)`).
   - Cài đặt cả `validate_diagnostic` và `synthesize_diagnostic` trong `impl TtsProvider for ConfiguredKokoroVi`:
     ```rust
     fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
         let voice = request.voice.as_deref().unwrap_or(&self.default_voice);
         let language = request.language.as_deref().unwrap_or(&self.language);
         if language != "vi-VN" {
             return Err(TtsError::UnsupportedLanguage(language.into()));
         }
         if !self.voicepacks.contains_key(voice) {
             return Err(TtsError::UnsupportedVoice(voice.into()));
         }
         Ok(())
     }

     fn synthesize_diagnostic(
         &self,
         request: &TtsDiagnosticRequest,
         cancelled: &std::sync::atomic::AtomicBool,
         on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
     ) -> Result<(), TtsError> {
         self.validate_diagnostic(request)?;
         let mut worker = self.open_worker()?;
         worker.synthesize(
             &TtsSynthesisRequest {
                 text: request.text.clone(),
                 selection: TtsBinding {
                     voice: request
                         .voice
                         .clone()
                         .unwrap_or_else(|| self.default_voice.clone()),
                     language: request
                         .language
                         .clone()
                         .unwrap_or_else(|| self.language.clone()),
                 },
             },
             cancelled,
             on_pcm,
         )
     }
     ```
3. **Trong `KokoroViWorker::synthesize`** ([`kokoro_vi/runtime.rs:78-105`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/runtime.rs#L78-L105)):
   - Chọn `voicepack` theo `request.selection.voice`:
     ```rust
     let voice_id = if request.selection.voice.is_empty()
         || request.selection.voice == "__readiness__"
     {
         self.default_voice.as_str()
     } else {
         request.selection.voice.as_str()
     };
     let voicepack = self
         .voicepacks
         .get(voice_id)
         .ok_or_else(|| TtsError::UnsupportedVoice(voice_id.to_owned()))?;
     ```
   - Dùng `voicepack.latent(phoneme_count)` khi chạy `self.session.run(...)`. Không phải tạo lại `ort::session::Session` hay `G2pSidecar`.

#### B. Nâng cấp `chillaudio_ws` hỗ trợ `validate_diagnostic`, `synthesize_diagnostic` và đọc `request.selection.voice`

- **File**: [`crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs)

1. Trong `impl TtsProvider for ChillAudioWsProvider` ([`chillaudio/provider.rs:26-41`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs#L26-L41)), override cả `validate_diagnostic` và `synthesize_diagnostic` (vì default implementation của `TtsProvider::synthesize_diagnostic` tại [`providers/tts/mod.rs:91-93`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/mod.rs#L91-L93) trả về `Err(DiagnosticOverrideUnsupported)` khi `request.voice.is_some()`):
   ```rust
   fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
       let voice = request.voice.as_deref().unwrap_or(&self.config.voice);
       let language = request.language.as_deref().unwrap_or(&self.config.language);
       if !matches!(language, "vi" | "vi-VN") {
           return Err(TtsError::UnsupportedLanguage(language.into()));
       }
       if !super::descriptor::VOICES.iter().any(|v| v.id == voice) {
           return Err(TtsError::UnsupportedVoice(voice.into()));
       }
       Ok(())
   }

   fn synthesize_diagnostic(
       &self,
       request: &TtsDiagnosticRequest,
       cancelled: &AtomicBool,
       on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
   ) -> Result<(), TtsError> {
       self.validate_diagnostic(request)?;
       let mut worker = self.open_worker()?;
       worker.synthesize(
           &TtsSynthesisRequest {
               text: request.text.clone(),
               selection: super::super::TtsBinding {
                   voice: request
                       .voice
                       .clone()
                       .unwrap_or_else(|| self.config.voice.clone()),
                   language: request
                       .language
                       .clone()
                       .unwrap_or_else(|| self.config.language.clone()),
               },
           },
           cancelled,
           on_pcm,
       )
   }
   ```
2. Trong `ChillAudioWsWorker::synthesize` ([`chillaudio/provider.rs:47-63`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs#L47-L63)):
   - Tạo bản sao `let mut effective = self.config.clone();`, nếu `!request.selection.voice.is_empty() && request.selection.voice != "__readiness__"` thì kiểm tra `voice` thuộc `super::descriptor::VOICES` (trả về `Err(TtsError::UnsupportedVoice(...))` nếu không hợp lệ) rồi gán `effective.voice = request.selection.voice.clone();`, và truyền `&effective` vào `fetch_mp3(&effective, &request.text, cancelled)`.

---

### 3.2. Nguồn Chân lý Duy nhất `providers::tts::voice_catalog`, Cô lập Danh mục `vieneu` Turbo vs Nano & Bảo toàn `configured_voice`

- **Files**:
  - `crates/voice-agent-server/src/providers/tts/voice_catalog.rs` *(tạo mới)*
  - [`crates/voice-agent-server/src/providers/tts/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/mod.rs#L59-L120) *(export `pub mod voice_catalog;` và thêm `fn model_id(&self) -> Option<&str> { None }` vào `trait TtsProvider`)*
  - [`crates/voice-agent-server/src/providers/tts/vieneu/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/vieneu/mod.rs#L49-L53) *(override `fn model_id(&self) -> Option<&str> { Some(&self.options.model) }` trên `ConfiguredVieneuNative`)*
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs)
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/discovery.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/discovery.rs#L40-L46)
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/resolved.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/resolved.rs)

#### A. Cô lập Triệt để Danh mục Giọng giữa `vieneu_v3_turbo_fp32` (25 giọng) và `vieneu_v3_nano_fp32` (11 giọng)

Trong [`vieneu/assets.rs:172-224`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/vieneu/assets.rs#L172-L224), adapter `vieneu` hỗ trợ 2 model độc lập với tập giọng khác nhau (và `NativeState::supports_voice` chỉ chấp nhận giọng thuộc đúng model đang nạp):

1. **`VieneuModel::V3TurboFp32` (`"vieneu_v3_turbo_fp32"` — 25 giọng)**:
   - **11 giọng Nữ (`TURBO_FEMALE_VOICES`)**: `ngoc_huyen` *(mặc định Nữ)*, `truc_ly`, `mai_anh`, `thuy_dung`, `ngoc_tran`, `ngoc_linh`, `doan_trang`, `thuc_doan`, `my_duyen`, `quynh_anh`, `kim_thanh`.
   - **14 giọng Nam (`TURBO_MALE_VOICES`)**: `hai_dang` *(mặc định Nam & mặc định model Turbo)*, `thien_minh`, `thien_tam_duc`, `quang_son`, `minh_duc`, `pham_tuyen`, `thai_son`, `xuan_vinh`, `thanh_binh`, `minh_triet`, `duc_tri`, `adam`, `adam_bua`, `quoc_tuan`.
2. **`VieneuModel::V3NanoFp32` (`"vieneu_v3_nano_fp32"` — 11 giọng)**:
   - **4 giọng Nữ (`NANO_FEMALE_VOICES`)**: `my_duyen` *(mặc định Nữ)*, `mai_anh`, `truc_ly`, `ai_han`.
   - **7 giọng Nam (`NANO_MALE_VOICES`)**: `minh_quan` *(mặc định Nam & mặc định model Nano)*, `adam`, `duc_tri`, `huu_quan`, `xuan_tien`, `anh_khoi`, `manh_dung`.
3. **Xử lý bài toán 5 giọng giao nhau (`my_duyen`, `mai_anh`, `truc_ly`, `adam`, `duc_tri`) và các giọng độc quyền của từng model**:
   - Có **5 giọng xuất hiện ở cả Turbo lẫn Nano**: `my_duyen`, `mai_anh`, `truc_ly` (Nữ) và `adam`, `duc_tri` (Nam). Nếu một `TtsWorkerRuntime` của `vieneu` đang bind với `voice = "adam"` mà chỉ nhìn vào `(adapter = "vieneu", voice = "adam")`, hệ thống sẽ **không biết** runtime đó đang chạy `vieneu_v3_nano_fp32` hay `vieneu_v3_turbo_fp32`. Nếu đoán nhầm là Turbo và chọn `"ngoc_huyen"` khi có yêu cầu giọng Nữ, `ConfiguredVieneuNative::validate_diagnostic` của Nano sẽ báo lỗi `UnsupportedVoice("ngoc_huyen")` (vì `"ngoc_huyen"` và `"hai_dang"` **không tồn tại** trong `NANO_VOICES`!).
   - **Giải pháp dứt điểm**:
     1. Bổ sung `fn model_id(&self) -> Option<&str> { None }` vào `trait TtsProvider` ([`providers/tts/mod.rs:59`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/mod.rs#L59)) và implement trên `ConfiguredVieneuNative` ([`providers/tts/vieneu/mod.rs:49`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/vieneu/mod.rs#L49)) trả về `Some(&self.options.model)` (`"vieneu_v3_turbo_fp32"` hoặc `"vieneu_v3_nano_fp32"`).
     2. Quy tắc xác định `VieneuModel` trong `voice_catalog::resolve_vieneu_model(model_or_voice_spec, runtime_default_voice)`:
        - Nếu `model_or_voice_spec` chứa `"vieneu_v3_nano_fp32"` hoặc `"nano"` $\rightarrow$ `VieneuModel::V3NanoFp32`.
        - Nếu `model_or_voice_spec` chứa `"vieneu_v3_turbo_fp32"` hoặc `"turbo"` $\rightarrow$ `VieneuModel::V3TurboFp32`.
        - Nếu `model_or_voice_spec` hoặc `runtime_default_voice` là một trong 6 giọng **độc quyền của Nano** (`"minh_quan"`, `"ai_han"`, `"huu_quan"`, `"xuan_tien"`, `"anh_khoi"`, `"manh_dung"`) $\rightarrow$ tự động suy diễn `VieneuModel::V3NanoFp32`.
        - Mặc định còn lại $\rightarrow$ `VieneuModel::V3TurboFp32`.
     3. **Chuyển đổi chéo giới tính an toàn giữa Turbo $\leftrightarrow$ Nano trong nội bộ `vieneu`**:
        - Nếu active model là `VieneuModel::V3NanoFp32` nhưng caller/Subagent truyền nhầm một giọng độc quyền của Turbo (ví dụ `"ngoc_huyen"` là Nữ của Turbo, hoặc `"hai_dang"` là Nam của Turbo), `voice_catalog` nhận diện giới tính của giọng đó trong họ `vieneu` (`"ngoc_huyen"` $\rightarrow$ `Female`, `"hai_dang"` $\rightarrow$ `Male`) và ánh xạ sang giọng tương ứng hợp lệ của **Nano** (`"my_duyen"` cho Nữ, `"minh_quan"` cho Nam — hoặc giữ nguyên `default_voice` của Nano nếu đã cùng giới tính).
        - Ngược lại, nếu active model là `VieneuModel::V3TurboFp32` nhưng caller truyền giọng độc quyền của Nano (`"ai_han"` $\rightarrow$ `Female`, `"minh_quan"` $\rightarrow$ `Male`), `voice_catalog` ánh xạ sang giọng tương ứng hợp lệ của **Turbo** (`"ngoc_huyen"` cho Nữ, `"hai_dang"` cho Nam).

#### B. Đặt `voice_catalog.rs` tại tầng `providers::tts` (Đảm bảo `Locality` cho cả `workers::tts` và `services::task_flow`)

Tạo `crates/voice-agent-server/src/providers/tts/voice_catalog.rs` chứa:
1. `pub enum VoiceGender { Female, Male }` và `pub fn parse_gender_intent(raw: &str) -> Option<VoiceGender>`:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum VoiceGender {
       Female,
       Male,
   }

   /// Nhận diện xem chuỗi `voice` mà Agent truyền vào có phải là yêu cầu giới tính ngữ nghĩa không
   /// (ví dụ: `"female"`, `"vi-female"`, `"nữ"`, `"giong_nu"`, `"male"`, `"vi-male"`, `"nam"`, ...).
   pub fn parse_gender_intent(raw: &str) -> Option<VoiceGender> {
       let normalized = raw.trim().to_ascii_lowercase();
       match normalized.as_str() {
           "female" | "vi-female" | "vi_female" | "nu" | "nữ" | "giong_nu" | "giọng nữ"
           | "woman" | "girl" => Some(VoiceGender::Female),
           "male" | "vi-male" | "vi_male" | "nam" | "giong_nam" | "giọng nam" | "man" | "boy" => {
               Some(VoiceGender::Male)
           }
           _ => None,
       }
   }
   ```
2. `pub struct AdapterVoiceCatalog`:
   - Chứa `default_voice: String`, `female_voices: Vec<String>` (danh sách quảng bá theo đúng model active: 11 giọng nữ cho `vieneu` Turbo, 4 giọng nữ cho `vieneu` Nano), `male_voices: Vec<String>` (14 giọng nam cho `vieneu` Turbo, 7 giọng nam cho `vieneu` Nano), và `guidance: String`.
   - Hàm `pub fn catalog_for_adapter(adapter: &str, model_or_voice_spec: Option<&str>, runtime_default_voice: Option<&str>) -> AdapterVoiceCatalog`:
     - Tự động tách `model_or_voice_spec` (ví dụ `"vieneu_v3_turbo_fp32:hai_dang"`, `"vieneu_v3_nano_fp32:minh_quan"`, `"vieneu_v3_nano_fp32"`, `"hai_dang"`) để xác định chính xác biến thể model (`V3NanoFp32` vs `V3TurboFp32`) và `configured_voice`.
     - Ưu tiên `runtime_default_voice` (nếu hợp lệ với model đang chọn) $\rightarrow$ `configured_voice` trong `model_or_voice_spec` (nếu hợp lệ với model đang chọn) $\rightarrow$ giọng mặc định chuẩn của model (`"hai_dang"` cho `vieneu` Turbo, `"minh_quan"` cho `vieneu` Nano).
3. Hàm phân giải tập trung `pub fn resolve_tts_voice(adapter: &str, model_or_voice_spec: Option<&str>, runtime_default_voice: Option<&str>, requested_voice: Option<&str>) -> String`:
   - **Bước 1 — Giá trị trung tính / mặc định**:
     - Nếu `requested_voice` là `None`, `""`, `"default"`, `"vi-default"` $\rightarrow$ trả về `catalog.default_voice`.
     - Nếu `requested_voice` là `"alloy"` và `adapter` là local/server TTS adapter (không phải OpenAI / `llm_gateway`) $\rightarrow$ coi `"alloy"` là giá trị mặc định trung tính (tương thích ngược với [`diagnostic.rs:111`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/diagnostic.rs#L111) và [`tests_media.rs:142`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/tests_media.rs#L142)) và trả về `catalog.default_voice`.
   - **Bước 2 — Khớp trực tiếp Voice ID của Adapter/Model hiện hành**:
     - Kiểm tra `requested_voice` trong tập giọng hỗ trợ của model hiện hành (với `vieneu` Turbo chỉ khớp 25 giọng `TURBO_VOICES`; với `vieneu` Nano chỉ khớp 11 giọng `NANO_VOICES`; với `edge_tts` khớp toàn bộ 322 giọng trong `edge_tts::voices::VOICES`; hỗ trợ chuẩn hóa chữ thường và dấu `-` vs `_` như `"ngoc-huyen"` $\rightarrow$ `"ngoc_huyen"`): nếu khớp, trả về mã `voice` chuẩn của model đó.
   - **Bước 3 — Phân giải theo Ý định Giới tính (`parse_gender_intent` hoặc giọng chéo Turbo $\leftrightarrow$ Nano trong `vieneu`)**:
     - Xác định `target_gender` từ `parse_gender_intent(requested_voice)` (hoặc khi `adapter == "vieneu"` và `requested_voice` là giọng thuộc biến thể VieNeu kia).
     - Khi có `Some(target_gender)`:
       - Kiểm tra giới tính của `catalog.default_voice`:
         - Nếu `catalog.default_voice` **đã thuộc `target_gender`** (ví dụ Admin cấu hình sẵn giọng nữ `"mai_anh"` trên Nano hoặc Turbo và yêu cầu là `Female`): **giữ nguyên `catalog.default_voice`**.
         - Nếu `catalog.default_voice` **khác giới tính** (ví dụ đang cấu hình giọng nam `"hai_dang"` trên Turbo hoặc `"minh_quan"` trên Nano nhưng yêu cầu là `Female`): trả về giọng đầu tiên khớp `target_gender` của model hiện hành (`"ngoc_huyen"` cho `vieneu` Turbo, `"my_duyen"` cho `vieneu` Nano, `"maichi"` cho `zerotts_onnx`, `"vi-VN-HoaiMyNeural"` cho `edge_tts`, `"ngoc_huyen"` cho `kokoro_vi_onnx`, `"BV421_vivn_streaming"` cho `chillaudio_ws`, `"nova"` cho `openai`).
   - **Bước 4 — Fallback an toàn**:
     - Nếu `requested_voice` không thuộc model hiện hành và không xác định được giới tính $\rightarrow$ trả về `catalog.default_voice`.

#### C. Bảo toàn cả `model` lẫn `configured_voice` trong `speech.rs` và `discovery.rs` (Cho cả `config`, `database`, và `runtime` entries)

Hiện tại tại [`speech.rs:119`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs#L119):
```rust
parsed.get("model").or_else(|| parsed.get("voice"))
```
Với `vieneu` (`"model": "vieneu_v3_nano_fp32"`, `"voice": "mai_anh"`), JSON cấu hình có cả `"model"` lẫn `"voice"`, khiến `"voice"` bị mất khi tạo `ResolvedLlmProvider`. Ngoài ra ở [`discovery.rs:44`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/discovery.rs#L44), `runtime_entries` chỉ truyền `(k.clone(), r.provider().adapter().to_owned())` nên mất cả `r.provider().model_id()` lẫn `r.binding().voice`.

**Cập nhật**:
1. Thêm getter `pub fn binding(&self) -> &TtsBinding` trên [`TtsWorkerRuntime`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/mod.rs#L151-L160).
2. Trong [`speech.rs:112-136`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs#L112-L136), khi `kind == "tts"`:
   - Nếu `parsed` có cả `"model"` và `"voice"` (khác rỗng và khác nhau), ghép thành `format!("{model}:{voice}")` (ví dụ `"vieneu_v3_turbo_fp32:hai_dang"`, `"vieneu_v3_nano_fp32:minh_quan"`). Nếu chỉ có `"voice"` hoặc `"model"` thì giữ nguyên như cũ.
3. Trong [`discovery.rs:40-46`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/discovery.rs#L40-L46) và [`speech.rs:80-110`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs#L80-L110):
   - Khi thu thập `runtime_entries` cho TTS, lấy `model_id = r.provider().model_id().unwrap_or_else(|| r.provider().adapter())` và ghép với `r.binding().voice` thành `format!("{model_id}:{}", r.binding().voice)` (ví dụ `"vieneu_v3_nano_fp32:adam"`).
   - Nhờ vậy, `ResolvedLlmProvider` luôn bảo toàn chính xác cả biến thể model (`Turbo` vs `Nano`) lẫn `configured_voice` ở mọi nguồn (`config`, `database`, `runtime`)!

---

### 3.3. Kết nối vào `TtsWorkerRuntime`, `ServerMediaRuntimeBridge`, `CatalogMediaBackend` & `build_agent_definitions`

- **Files**:
  - [`crates/voice-agent-server/src/workers/tts/diagnostic.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/diagnostic.rs#L104-L125)
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/media_runtime_bridge.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_runtime_bridge.rs#L24-L40)
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs#L406-L470)
  - [`crates/voice-agent-server/src/services/task_flow/agent_settings/resolution.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/resolution.rs#L329-L347)

1. **Trong `TtsWorkerRuntime` ([`diagnostic.rs:104-125`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/diagnostic.rs#L104-L125))**:
   - Thêm helper `pub fn resolve_background_voice(&self, voice: Option<&str>) -> String` gọi:
     ```rust
     crate::providers::tts::voice_catalog::resolve_tts_voice(
         self.provider.adapter(),
         self.provider.model_id(),
         Some(&self.binding.voice),
         voice,
     )
     ```
   - Nhờ truyền `self.provider.model_id()`, khi `TtsWorkerRuntime` đang chạy `vieneu_v3_nano_fp32` (dù bind với giọng giao nhau như `"adam"` hay giọng riêng như `"minh_quan"`), `resolve_background_voice(Some("female"))` luôn chọn đúng giọng Nữ của Nano (`"my_duyen"`), còn khi chạy `vieneu_v3_turbo_fp32` sẽ chọn đúng giọng Nữ của Turbo (`"ngoc_huyen"`).
   - Trong `synthesize_background_wav(&self, text: &str, voice: Option<&str>)`:
     - Tính `let resolved_voice = self.resolve_background_voice(voice);`
     - Nếu `resolved_voice != self.binding.voice`, tạo `effective_diagnostic_request` với `voice: Some(resolved_voice)` (fallback về `self.binding.clone()` nếu provider là mock test không hỗ trợ diagnostic override).

2. **Trong `ServerMediaRuntimeBridge` ([`media_runtime_bridge.rs:24-40`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_runtime_bridge.rs#L24-L40))**:
   - Thêm phương thức trả về cả `model_id` và `default_voice` của runtime đang nạp:
     ```rust
     pub(crate) fn inspect_runtime_model_and_voice(
         &self,
         provider_ref: &str,
     ) -> (Option<String>, Option<String>) {
         let bare_key = provider_ref
             .strip_prefix("config:tts:")
             .unwrap_or(provider_ref);
         let Some(runtime) = self
             .runtimes
             .tts
             .get(provider_ref)
             .or_else(|| self.runtimes.tts.get(bare_key))
             .filter(|r| r.provider().adapter() != "unavailable")
         else {
             return (None, None);
         };
         (
             runtime.provider().model_id().map(str::to_owned),
             Some(runtime.binding().voice.clone()),
         )
     }
     ```

3. **Trong `CatalogMediaBackend` ([`media_backend.rs:406-470`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs#L406-L470))**:
   - Đọc `(runtime_model, runtime_voice) = self.tts_bridge.inspect_runtime_model_and_voice(&resolved.provider_ref)` để truyền vào `resolved.tts_constraints_with_runtime(runtime_model.as_deref(), runtime_voice.as_deref())` và `resolved.resolve_tts_voice_with_runtime(runtime_model.as_deref(), runtime_voice.as_deref(), Some(&req.voice))`.
   - Truyền `Some(effective_voice.as_str())` vào `self.synthesize_via_server_tts(&resolved.provider_ref, req, Some(&effective_voice))` (và gán `voice: effective_voice` vào `SynthesizedAudioPayload` ở cả nhánh server TTS lẫn nhánh `LlmOperation::AudioSpeech`).

4. **Trong `build_agent_definitions` ([`resolution.rs:329-347`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/resolution.rs#L329-L347))**:
   - Tương tự như `video_create` trên `media_creator`, khi `eff.allowed_tools.iter().any(|t| t == "tts_synthesize")`, đính kèm tóm tắt `tts_constraints` vào `description` của `media_processor`:
     ```rust
     if eff.allowed_tools.iter().any(|t| t == "tts_synthesize") {
         let tc = super::media_backend::resolve_media_tool_provider(
             catalog,
             Some(settings),
             "tts_synthesize",
             "tts",
             true,
         )
         .map(|p| p.tts_constraints(None))
         .unwrap_or_default();
         description = format!(
             "{description} [Active tts_synthesize provider: {} | default_voice='{}' | female_voices=[{}] | male_voices=[{}]]",
             tc.provider_label,
             tc.default_voice,
             tc.female_voices.join(", "),
             tc.male_voices.join(", "),
         );
     }
     ```
   - Nhờ `speech.rs` đã bảo toàn `configured_voice` trong `ResolvedLlmProvider`, lời gọi `p.tts_constraints(None)` tại đây vẫn phản ánh chính xác `default_voice` do Admin cấu hình.

---

## 4. Chiến lược Kiểm thử ("Replace, Don't Layer" — `DEEPENING.md`)

Kiểm thử đặt trực tiếp tại **External Seams** (`TtsSynthesizeTool::descriptor`, `TtsSynthesizeTool::execute`, `CatalogMediaBackend::synthesize_tts`, `TtsWorkerRuntime::synthesize_background_wav`) để xác minh hành vi quan sát được mà không phụ thuộc chi tiết cài đặt nội bộ:

1. **Unit & Integration Tests trong `orchestrator_task_rs` ([`media_artifacts_tests.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/media_artifacts_tests.rs))**:
   - `tts_synthesize_descriptor_reflects_dynamic_provider_voice_constraints`: Xác nhận khi `TtsSynthesizerBackend::tts_constraints()` trả về danh sách giọng của `vieneu` hoặc `edge_tts`, `TtsSynthesizeTool::descriptor()` tự động cập nhật `description` và `input_schema.properties.voice.description` chứa đầy đủ `default_voice`, `female_voices`, `male_voices` mà vẫn giữ nguyên `"additionalProperties": false` và các trường hiện hữu.
   - `tts_synthesize_execute_reports_resolved_provider_voice_in_artifact_metadata`: Xác nhận khi Subagent truyền `voice="female"`, metadata JSON trả về từ `TtsSynthesizeTool::execute` phản ánh đúng mã giọng thực tế mà backend đã chọn (ví dụ `"ngoc_huyen"`).

2. **Unit & Integration Tests trong `AI_Agent_Voice_rust` ([`tests_media.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/tests_media.rs))**:
   - `tts_constraints_and_voice_resolution_across_all_adapters`: Kiểm tra `tts_constraints()` và `resolve_tts_voice()` cho `vieneu` Turbo (`model = "vieneu_v3_turbo_fp32:hai_dang"`, 11 giọng nữ / 14 giọng nam), `vieneu` Nano (`model = "vieneu_v3_nano_fp32:minh_quan"` và `"vieneu_v3_nano_fp32:adam"`, 4 giọng nữ / 7 giọng nam), `zerotts_onnx`, `edge_tts` (quảng bá 6 giọng chính nhưng phân giải được cả 322 giọng), `kokoro_vi_onnx`, `chillaudio_ws`, `vifasttts_onnx`, và `openai`:
     - Khi provider đang cấu hình giọng nam (`"hai_dang"` trên VieNeu Turbo, `"minh_quan"` hoặc `"adam"` trên VieNeu Nano, `"vi-VN-NamMinhNeural"`, `"giahuy"`, `"manh_dung"`, `"BV075_streaming"`) và Subagent truyền `"female"` / `"vi-female"` / `"nữ"` $\rightarrow$ tự động chuyển sang đúng giọng nữ chuẩn của model đó (`"ngoc_huyen"` cho VieNeu Turbo, `"my_duyen"` cho VieNeu Nano, `"vi-VN-HoaiMyNeural"`, `"maichi"`, `"ngoc_huyen"`, `"BV421_vivn_streaming"`).
     - Khi `vieneu` Nano đang active nhưng caller truyền nhầm giọng độc quyền của Turbo (`"ngoc_huyen"` hoặc `"hai_dang"`) $\rightarrow$ tự động ánh xạ sang giọng tương ứng của Nano (`"my_duyen"` cho Nữ, `"minh_quan"` cho Nam); và ngược lại từ Nano sang Turbo (`"ai_han"` $\rightarrow$ `"ngoc_huyen"`, `"minh_quan"` $\rightarrow$ `"hai_dang"`).
     - Khi provider đã cấu hình sẵn một giọng nữ (ví dụ `"mai_anh"` trên cả Turbo và Nano) và Subagent truyền `"female"` $\rightarrow$ giữ nguyên `"mai_anh"`.
     - Khi Subagent truyền đích danh một giọng nữ khác của cùng model (ví dụ `"truc_ly"` hoặc `"ai_han"` trên Nano) $\rightarrow$ dùng chính xác giọng đó.
     - Khi caller truyền `"alloy"` vào local TTS adapter $\rightarrow$ coi là trung tính và giữ nguyên `default_voice` của local TTS; khi truyền `"alloy"` vào OpenAI TTS $\rightarrow$ dùng `"alloy"`.
   - `server_tts_runtime_synthesizes_with_female_voice_override_when_bound_to_male_default`: Khởi tạo `TtsWorkerRuntime` cho cả `vieneu_v3_turbo_fp32` (bound `"hai_dang"`) và `vieneu_v3_nano_fp32` (bound `"adam"`), gọi `CatalogMediaBackend::synthesize_tts` với `voice: "vi-female"`, xác nhận worker Turbo nhận `"ngoc_huyen"` còn worker Nano nhận `"my_duyen"`.
   - `chillaudio_validate_and_synthesize_diagnostic_accept_supported_voices`: Kiểm tra `validate_diagnostic` của `ChillAudioWsProvider` chấp nhận toàn bộ 4 voice trong catalog và từ chối voice lạ với `TtsError::UnsupportedVoice`.

---

## 5. Danh sách File Dự kiến Thay đổi Khi Triển khai

1. `orchestrator_task_rs/src/tools/media_tts.rs` *(tạo mới)*, [`src/tools/media_artifacts.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/media_artifacts.rs), [`src/tools/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/mod.rs), [`src/lib.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/lib.rs) — Tách module `media_tts.rs`, thêm `TtsProviderConstraints`, mở rộng `TtsSynthesizerBackend::tts_constraints()`, bổ sung `voice` vào `SynthesizedAudioPayload`, và cập nhật `TtsSynthesizeTool::descriptor()` / `execute()`.
2. [`orchestrator_task_rs/src/prompts.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/prompts.rs) — Cập nhật `DAILY_ROOT_PROMPT` và `MEDIA_PROCESSOR_PROMPT` để truyền và chọn `voice` theo danh sách giọng Nam/Nữ của active provider.
3. [`orchestrator_task_rs/src/tools/media_artifacts_tests.rs`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/tools/media_artifacts_tests.rs) — Thêm unit tests kiểm tra dynamic descriptor và voice resolution của `TtsSynthesizeTool`.
4. `AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/voice_catalog.rs` *(tạo mới)*, [`providers/tts/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/mod.rs), & [`providers/tts/vieneu/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/vieneu/mod.rs) — Nguồn chân lý duy nhất cho bảng giọng Nam/Nữ của 7 adapter (tách biệt hoàn toàn `vieneu_v3_turbo_fp32` 25 giọng và `vieneu_v3_nano_fp32` 11 giọng), bổ sung `TtsProvider::model_id()`, và hàm `resolve_tts_voice()`.
5. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/mod.rs), [`runtime.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/kokoro_vi/runtime.rs), & [`factory_registry.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/factory_registry.rs) — Thay `voicepack` bằng `voicepacks` trong `KokoroViArtifacts`, nạp bảng `BTreeMap<String, Voicepack>`, cài đặt `validate_diagnostic` + `synthesize_diagnostic` và chọn `voicepack` động theo `request.selection.voice`.
6. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/providers/tts/chillaudio/provider.rs) — Cài đặt `validate_diagnostic` + `synthesize_diagnostic` và sử dụng `request.selection.voice` khi gửi WebSocket payload.
7. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/mod.rs) & [`diagnostic.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/workers/tts/diagnostic.rs) — Thêm getter `binding(&self)`, `resolve_background_voice(&self)` (truyền `self.provider.model_id()`), và hỗ trợ phân giải alias giới tính trong `synthesize_background_wav`.
8. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/speech.rs), [`discovery.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/discovery.rs), & [`resolved.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/provider_catalog/resolved.rs) — Bảo toàn cả `model` (`vieneu_v3_turbo_fp32` vs `vieneu_v3_nano_fp32`) lẫn `configured_voice` trong `ResolvedLlmProvider` và ủy quyền `tts_constraints()` / `resolve_tts_voice()` sang `providers::tts::voice_catalog`.
9. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_runtime_bridge.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_runtime_bridge.rs), [`media_backend.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/media_backend.rs), & [`resolution.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/agent_settings/resolution.rs) — Thêm `inspect_runtime_model_and_voice`, triển khai `CatalogMediaBackend::tts_constraints()`, phân giải `effective_voice` trong `synthesize_tts()`, và đính kèm thông tin TTS provider vào mô tả `media_processor`.
10. [`AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/tests_media.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/tests_media.rs) — Thêm bộ test kiểm chứng đổi giọng Nữ/Nam động qua `CatalogMediaBackend` và `TtsWorkerRuntime` cho cả VieNeu Turbo và VieNeu Nano.
