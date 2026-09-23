pub(super) fn default_hello_timeout_ms() -> u64 {
    5_000
}
pub(super) fn default_input_rate() -> u32 {
    16_000
}
pub(super) fn default_output_rate() -> u32 {
    24_000
}
pub(super) fn default_channels() -> u8 {
    1
}
pub(super) fn default_frame_ms() -> u16 {
    60
}
pub(super) fn default_max_frame_bytes() -> usize {
    65_536
}
pub(super) fn default_max_utterance_ms() -> u64 {
    30_000
}
pub(super) fn default_queue_capacity() -> usize {
    32
}
pub(super) fn default_max_active_turns() -> usize {
    8
}
pub(super) fn default_llm_concurrency() -> usize {
    2
}
pub(super) fn default_llm_timeout_ms() -> u64 {
    60_000
}
pub(super) fn default_tts_concurrency() -> usize {
    2
}
pub(super) fn default_vad_adapter() -> String {
    "silero_onnx".into()
}
pub(super) fn default_vad_model() -> String {
    "silero_vad_v5".into()
}
pub(super) fn default_asr_adapter() -> String {
    "zipformer_sherpa".into()
}
pub(super) fn default_asr_model() -> String {
    "zipformer_vi_streaming".into()
}
pub(super) fn default_llm_adapter() -> String {
    "openai".into()
}
pub(super) fn default_openai_base_url() -> Url {
    Url::parse("https://api.openai.com/v1").expect("valid default OpenAI URL")
}
pub(super) fn default_openai_model() -> String {
    "model-name".into()
}
pub(super) fn default_tts_adapter() -> String {
    "zerotts_onnx".into()
}
pub(super) fn default_tts_model() -> String {
    "zerotts_default".into()
}
pub(super) fn default_tts_voice() -> String {
    "maichi".into()
}
pub(super) fn default_provider_threads() -> i32 {
    1
}
pub(super) fn default_min_speech_ms() -> u64 {
    180
}
pub(super) fn default_end_silence_ms() -> u64 {
    600
}
pub(super) fn default_pre_roll_ms() -> u64 {
    300
}
pub(super) fn default_speech_threshold() -> f32 {
    0.50
}
pub(super) fn default_exit_threshold() -> f32 {
    0.35
}
pub(super) fn default_vad_worker_count() -> usize {
    4
}
pub(super) fn default_asr_worker_count() -> usize {
    2
}
pub(super) fn default_asr_threads() -> i32 {
    2
}
pub(super) fn default_decoding_method() -> String {
    "greedy_search".into()
}
pub(super) fn default_manifest_path() -> std::path::PathBuf {
    "models/manifest.toml".into()
}
pub(super) fn default_models_root() -> std::path::PathBuf {
    "models".into()
}
pub(super) fn default_onnx_runtime_library() -> std::path::PathBuf {
    "runtime/onnxruntime/libonnxruntime.dylib".into()
}
pub(super) fn default_worker_queue_capacity() -> usize {
    32
}
pub(super) fn default_asr_final_timeout_ms() -> u64 {
    15_000
}
pub(super) fn default_vad_reset_timeout_ms() -> u64 {
    15_000
}
pub(super) fn default_cleanup_grace_ms() -> u64 {
    5_000
}
pub(super) fn default_max_history_messages() -> usize {
    20
}
pub(super) fn default_tts_timeout_ms() -> u64 {
    15_000
}
pub(super) fn default_speech_min_chars() -> usize {
    24
}
pub(super) fn default_speech_soft_break_min_chars() -> usize {
    48
}
pub(super) fn default_speech_max_chars() -> usize {
    160
}
pub(super) fn default_pending_segments() -> usize {
    8
}
use url::Url;
