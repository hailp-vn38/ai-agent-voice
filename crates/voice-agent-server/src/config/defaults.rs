pub(super) fn default_hello_timeout_ms() -> u64 {
    5_000
}
pub(super) fn default_database_url() -> String {
    "sqlite://data/voice-agent.db".into()
}
pub(super) fn default_true() -> bool {
    true
}
pub(super) fn default_database_max_connections() -> u32 {
    5
}
pub(super) fn default_database_busy_timeout_ms() -> u64 {
    5_000
}
pub(super) fn default_shutdown_grace_ms() -> u64 {
    15_000
}
pub(super) fn default_provider_test_concurrency() -> usize {
    2
}
pub(super) fn default_provider_test_timeout_ms() -> u64 {
    30_000
}
pub(super) fn default_history_retention_days() -> u32 {
    30
}
pub(super) fn default_history_queue_capacity() -> usize {
    256
}
pub(super) fn default_enrollment_code_ttl_seconds() -> u64 {
    600
}
pub(super) fn default_enrollment_retention_seconds() -> u64 {
    86_400
}
pub(super) fn default_enrollment_cleanup_interval_seconds() -> u64 {
    60
}
pub(super) fn default_enrollment_max_pending() -> u32 {
    1_000
}
pub(super) fn default_speaker_max_speakers() -> usize {
    256
}
pub(super) fn default_speaker_max_candidates_per_agent() -> usize {
    32
}
pub(super) fn default_speaker_max_voiceprint_spaces() -> usize {
    4
}
pub(super) fn default_speaker_min_samples() -> usize {
    3
}
pub(super) fn default_speaker_max_samples() -> usize {
    5
}
pub(super) fn default_speaker_min_clip_ms() -> u64 {
    5_000
}
pub(super) fn default_speaker_max_clip_ms() -> u64 {
    10_000
}
pub(super) fn default_speaker_min_speech_ms() -> u64 {
    3_000
}
pub(super) fn default_speaker_enrollment_ttl_ms() -> u64 {
    1_800_000
}
pub(super) fn default_speaker_max_open_enrollments() -> usize {
    16
}
pub(super) fn default_speaker_max_audio_body_bytes() -> usize {
    524_288
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
pub(crate) fn default_llm_timeout_ms() -> u64 {
    60_000
}
pub(super) fn default_tts_concurrency() -> usize {
    2
}
pub(super) fn default_vad_model() -> String {
    "silero_vad_v5".into()
}
pub(crate) fn default_asr_model() -> String {
    "zipformer_vi_streaming".into()
}
pub(crate) fn default_openai_base_url() -> Url {
    Url::parse("https://api.openai.com/v1").expect("valid default OpenAI URL")
}
pub(crate) fn default_openai_model() -> String {
    "model-name".into()
}
pub(super) fn default_vision_timeout_ms() -> u64 {
    30_000
}
pub(super) fn default_vision_max_tokens() -> u32 {
    500
}
pub(super) fn default_vision_temperature() -> f32 {
    0.7
}
pub(super) fn default_vision_top_p() -> f32 {
    1.0
}
pub(super) fn default_vision_max_image_bytes() -> usize {
    5 * 1024 * 1024
}
pub(super) fn default_vision_max_question_bytes() -> usize {
    4 * 1024
}
pub(super) fn default_vision_concurrency() -> usize {
    2
}
pub(crate) fn default_tts_model() -> String {
    "zerotts_default".into()
}
pub(crate) fn default_tts_voice() -> String {
    "maichi".into()
}
pub(crate) fn default_kokoro_vi_model() -> String {
    "kokoro_vi_contextbox".into()
}
pub(crate) fn default_kokoro_vi_voice() -> String {
    "diem_trinh".into()
}
pub(crate) fn default_kokoro_vi_speed_percent() -> u16 {
    100
}
pub(super) fn default_kokoro_vi_g2p_executable() -> PathBuf {
    PathBuf::from("runtime/kokoro-vi/kokoro_vi_g2p")
}
pub(crate) fn default_vietnamese_language() -> String {
    "vi-VN".into()
}
pub(crate) fn default_chillaudio_ws_url() -> Url {
    Url::parse("wss://sami-normal-sg.capcutapi.com/internal/api/v1/ws?device_id=7486429558272460289&iid=7486431924195657473&app_id=359289&region=VN&update_version_code=5.7.1.2101&version_code=5.7.1&appKey=ddjeqjLGMn&device_type=macos&device_platform=macos").expect("valid ChillAudio URL")
}
pub(crate) fn default_chillaudio_app_key() -> SecretString {
    SecretString("ddjeqjLGMn".into())
}
pub(crate) fn default_chillaudio_voice() -> String {
    "BV421_vivn_streaming".into()
}
pub(crate) fn default_chillaudio_timeout_ms() -> u64 {
    12_000
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
pub(crate) fn default_asr_threads() -> i32 {
    2
}
pub(crate) fn default_decoding_method() -> super::TransducerDecodingMethod {
    super::TransducerDecodingMethod::GreedySearch
}
pub(crate) fn default_gipformer_threads() -> i32 {
    4
}
pub(crate) fn default_gipformer_decoding_method() -> super::TransducerDecodingMethod {
    super::TransducerDecodingMethod::ModifiedBeamSearch
}
pub(crate) fn default_gipformer_max_active_paths() -> i32 {
    4
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
pub(super) fn default_prompt_budget_tokens() -> usize {
    12_000
}
pub(super) fn default_max_tool_result_chars() -> usize {
    4_096
}
pub(super) fn default_max_calls_per_round() -> usize {
    8
}
pub(super) fn default_max_rounds_per_turn() -> usize {
    4
}
pub(super) fn default_tool_execution_budget_ms() -> u64 {
    30_000
}
pub(super) fn default_mcp_enabled() -> bool {
    true
}
pub(super) fn default_mcp_call_timeout_ms() -> u64 {
    30_000
}
pub(super) fn default_mcp_discovery_timeout_ms() -> u64 {
    10_000
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
pub(super) fn default_external_per_server_resolution_timeout_ms() -> u64 {
    3_000
}
pub(super) fn default_external_overall_resolution_budget_ms() -> u64 {
    5_000
}
pub(super) fn default_external_max_concurrent_calls_per_server() -> u32 {
    16
}
pub(super) fn default_external_max_tools_per_server() -> usize {
    128
}
pub(super) fn default_external_max_tools_per_session() -> usize {
    512
}
pub(super) fn default_external_max_tool_schema_bytes() -> usize {
    16_384
}
pub(super) fn default_external_max_tool_description_bytes() -> usize {
    4_096
}
pub(super) fn default_external_max_tool_result_bytes() -> usize {
    16_384
}
pub(super) fn default_external_max_pages_per_server() -> usize {
    32
}
use super::SecretString;
use std::path::PathBuf;
use url::Url;
pub(super) fn default_enrollment_ws_max_connections() -> usize {
    32
}
pub(super) fn default_enrollment_ws_timeout_seconds() -> u64 {
    120
}
pub(super) fn default_enrollment_ws_poll_interval_ms() -> u64 {
    2_000
}
pub(super) fn default_enrollment_ws_prompt_repeat_seconds() -> u64 {
    60
}
pub(super) fn default_enrollment_prompt_assets_dir() -> std::path::PathBuf {
    "assets/enrollment/vi-VN".into()
}

pub(crate) fn default_gipformer_model() -> String {
    crate::providers::local_model_identity("gipformer_sherpa_offline")
        .expect("compiled local adapter")
        .into()
}
