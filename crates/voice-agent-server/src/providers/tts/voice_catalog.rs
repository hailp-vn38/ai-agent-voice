//! Nguồn chân lý duy nhất (`Locality`) cho danh mục giọng đọc Nam/Nữ của toàn bộ các TTS Adapter
//! (`vieneu` v3 Turbo & v3 Nano, `zerotts_onnx`, `edge_tts`, `kokoro_vi_onnx`, `chillaudio_ws`,
//! `vifasttts_onnx`, `openai`) và bộ phân giải giọng đọc theo ngữ nghĩa giới tính (`resolve_tts_voice`).

use super::vieneu::config::VieneuModel;

/// Giới tính giọng đọc phục vụ phân giải ngữ nghĩa (`"female"`, `"giọng nữ"`, `"male"`, `"giọng nam"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceGender {
    Female,
    Male,
}

/// Danh mục giọng đọc quảng bá và cấu hình mặc định của một TTS adapter/model cụ thể.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterVoiceCatalog {
    pub default_voice: String,
    pub female_voices: Vec<String>,
    pub male_voices: Vec<String>,
    pub guidance: String,
}

pub const VIENEU_TURBO_FEMALE_VOICES: &[&str] = &[
    "ngoc_huyen",
    "truc_ly",
    "mai_anh",
    "thuy_dung",
    "ngoc_tran",
    "ngoc_linh",
    "doan_trang",
    "thuc_doan",
    "my_duyen",
    "quynh_anh",
    "kim_thanh",
];

pub const VIENEU_TURBO_MALE_VOICES: &[&str] = &[
    "hai_dang",
    "thien_minh",
    "thien_tam_duc",
    "quang_son",
    "minh_duc",
    "pham_tuyen",
    "thai_son",
    "xuan_vinh",
    "thanh_binh",
    "minh_triet",
    "duc_tri",
    "adam",
    "adam_bua",
    "quoc_tuan",
];

pub const VIENEU_NANO_FEMALE_VOICES: &[&str] = &["my_duyen", "mai_anh", "truc_ly", "ai_han"];

pub const VIENEU_NANO_MALE_VOICES: &[&str] = &[
    "minh_quan",
    "adam",
    "duc_tri",
    "huu_quan",
    "xuan_tien",
    "anh_khoi",
    "manh_dung",
];

const VIENEU_NANO_EXCLUSIVE_VOICES: &[&str] = &[
    "minh_quan",
    "ai_han",
    "huu_quan",
    "xuan_tien",
    "anh_khoi",
    "manh_dung",
];

pub const ZEROTTS_FEMALE_VOICES: &[&str] = &["maichi", "baotrang", "hamy", "kimoanh"];
pub const ZEROTTS_MALE_VOICES: &[&str] = &["giahuy", "huuduc", "quangminh", "tiendat"];

pub const EDGE_ADVERTISED_FEMALE_VOICES: &[&str] = &[
    "vi-VN-HoaiMyNeural",
    "en-US-AriaNeural",
    "en-US-JennyNeural",
];
pub const EDGE_ADVERTISED_MALE_VOICES: &[&str] = &[
    "vi-VN-NamMinhNeural",
    "en-US-GuyNeural",
    "en-US-ChristopherNeural",
];

pub const KOKORO_FEMALE_VOICES: &[&str] = &[
    "ngoc_huyen",
    "diem_trinh",
    "mai_linh",
    "mai_loan",
    "my_yen",
    "thuc_trinh",
];
pub const KOKORO_MALE_VOICES: &[&str] = &[
    "manh_dung",
    "duc_an",
    "duc_duy",
    "hung_thinh",
    "phat_tai",
    "thanh_dat",
    "tuan_ngoc",
    "storyvert",
];

pub const CHILLAUDIO_FEMALE_VOICES: &[&str] = &[
    "BV421_vivn_streaming",
    "vi_female_huong",
    "BV074_streaming",
];
pub const CHILLAUDIO_MALE_VOICES: &[&str] = &["BV075_streaming"];

pub const VIFASTTTS_FEMALE_VOICES: &[&str] = &["ngoc_huyen"];
pub const VIFASTTTS_MALE_VOICES: &[&str] = &[];

pub const OPENAI_FEMALE_VOICES: &[&str] = &["nova", "shimmer", "coral", "sage", "alloy"];
pub const OPENAI_MALE_VOICES: &[&str] = &["onyx", "echo", "ash", "ballad", "fable"];

/// Nhận diện xem chuỗi `voice` mà Agent truyền vào có phải là yêu cầu giới tính ngữ nghĩa không
/// (ví dụ: `"female"`, `"vi-female"`, `"nữ"`, `"giong_nu"`, `"male"`, `"vi-male"`, `"nam"`, ...).
pub fn parse_gender_intent(raw: &str) -> Option<VoiceGender> {
    let normalized = raw.trim().to_lowercase();
    match normalized.as_str() {
        "female" | "vi-female" | "vi_female" | "nu" | "nữ" | "giong_nu" | "giong-nu"
        | "giọng nữ" | "woman" | "girl" => Some(VoiceGender::Female),
        "male" | "vi-male" | "vi_male" | "nam" | "giong_nam" | "giong-nam" | "giọng nam"
        | "man" | "boy" => Some(VoiceGender::Male),
        _ => None,
    }
}

fn normalize_voice_token(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('-', "_")
}

fn match_canonical<'a>(candidate: &str, list: &[&'a str]) -> Option<&'a str> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(&exact) = list.iter().find(|&&v| v.eq_ignore_ascii_case(trimmed)) {
        return Some(exact);
    }
    let norm = normalize_voice_token(trimmed);
    list.iter()
        .copied()
        .find(|&v| normalize_voice_token(v) == norm)
}

fn split_model_and_voice_spec(spec: Option<&str>) -> (Option<&str>, Option<&str>) {
    let Some(raw) = spec.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    if let Some((left, right)) = raw.split_once(':') {
        let model = left.trim();
        let voice = right.trim();
        (
            (!model.is_empty()).then_some(model),
            (!voice.is_empty()).then_some(voice),
        )
    } else {
        (Some(raw), Some(raw))
    }
}

/// Xác định biến thể `VieneuModel` (`V3TurboFp32` vs `V3NanoFp32`) từ `model_or_voice_spec`
/// và `runtime_default_voice`.
pub fn resolve_vieneu_model(
    model_or_voice_spec: Option<&str>,
    runtime_default_voice: Option<&str>,
) -> VieneuModel {
    let (model_part, voice_part) = split_model_and_voice_spec(model_or_voice_spec);
    if let Some(m) = model_part {
        if let Some(parsed) = VieneuModel::parse(m) {
            return parsed;
        }
        let lower = m.to_ascii_lowercase();
        if lower.contains("nano") {
            return VieneuModel::V3NanoFp32;
        }
        if lower.contains("turbo") {
            return VieneuModel::V3TurboFp32;
        }
    }
    for candidate in [voice_part, runtime_default_voice].into_iter().flatten() {
        if match_canonical(candidate, VIENEU_NANO_EXCLUSIVE_VOICES).is_some() {
            return VieneuModel::V3NanoFp32;
        }
    }
    VieneuModel::V3TurboFp32
}

fn vieneu_family_gender(voice: &str) -> Option<VoiceGender> {
    if match_canonical(voice, VIENEU_TURBO_FEMALE_VOICES).is_some()
        || match_canonical(voice, VIENEU_NANO_FEMALE_VOICES).is_some()
    {
        Some(VoiceGender::Female)
    } else if match_canonical(voice, VIENEU_TURBO_MALE_VOICES).is_some()
        || match_canonical(voice, VIENEU_NANO_MALE_VOICES).is_some()
    {
        Some(VoiceGender::Male)
    } else {
        None
    }
}

fn match_edge_voice(candidate: &str) -> Option<(&'static str, VoiceGender)> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    let found = super::edge_tts::voices::VOICES
        .iter()
        .find(|v| v.id.eq_ignore_ascii_case(trimmed))?;
    let gender = if found.name.contains("(Female") {
        VoiceGender::Female
    } else {
        VoiceGender::Male
    };
    Some((found.id, gender))
}

fn match_adapter_voice(
    adapter: &str,
    vieneu_model: VieneuModel,
    candidate: &str,
) -> Option<(String, VoiceGender)> {
    match adapter {
        "vieneu" => {
            let (females, males) = match vieneu_model {
                VieneuModel::V3TurboFp32 => (VIENEU_TURBO_FEMALE_VOICES, VIENEU_TURBO_MALE_VOICES),
                VieneuModel::V3NanoFp32 => (VIENEU_NANO_FEMALE_VOICES, VIENEU_NANO_MALE_VOICES),
            };
            if let Some(v) = match_canonical(candidate, females) {
                return Some((v.to_owned(), VoiceGender::Female));
            }
            if let Some(v) = match_canonical(candidate, males) {
                return Some((v.to_owned(), VoiceGender::Male));
            }
            None
        }
        "zerotts_onnx" => {
            if let Some(v) = match_canonical(candidate, ZEROTTS_FEMALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Female));
            }
            if let Some(v) = match_canonical(candidate, ZEROTTS_MALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Male));
            }
            None
        }
        "edge_tts" => match_edge_voice(candidate).map(|(id, g)| (id.to_owned(), g)),
        "kokoro_vi_onnx" => {
            if let Some(v) = match_canonical(candidate, KOKORO_FEMALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Female));
            }
            if let Some(v) = match_canonical(candidate, KOKORO_MALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Male));
            }
            None
        }
        "chillaudio_ws" => {
            if let Some(v) = match_canonical(candidate, CHILLAUDIO_FEMALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Female));
            }
            if let Some(v) = match_canonical(candidate, CHILLAUDIO_MALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Male));
            }
            None
        }
        "vifasttts_onnx" => match_canonical(candidate, VIFASTTTS_FEMALE_VOICES)
            .map(|v| (v.to_owned(), VoiceGender::Female)),
        _ => {
            if let Some(v) = match_canonical(candidate, OPENAI_FEMALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Female));
            }
            if let Some(v) = match_canonical(candidate, OPENAI_MALE_VOICES) {
                return Some((v.to_owned(), VoiceGender::Male));
            }
            None
        }
    }
}

/// Trả về danh mục giọng đọc quảng bá (`female_voices`, `male_voices`, `default_voice`, `guidance`)
/// cho một TTS adapter cụ thể.
pub fn catalog_for_adapter(
    adapter: &str,
    model_or_voice_spec: Option<&str>,
    runtime_default_voice: Option<&str>,
) -> AdapterVoiceCatalog {
    let vieneu_model = resolve_vieneu_model(model_or_voice_spec, runtime_default_voice);
    let (_, spec_voice) = split_model_and_voice_spec(model_or_voice_spec);

    let (fallback_default, female_slice, male_slice, guidance): (
        &str,
        &[&str],
        &[&str],
        &str,
    ) = match adapter {
        "vieneu" => match vieneu_model {
            VieneuModel::V3TurboFp32 => (
                "hai_dang",
                VIENEU_TURBO_FEMALE_VOICES,
                VIENEU_TURBO_MALE_VOICES,
                "For a female voice on VieNeu v3 Turbo, choose one of the advertised female_voices (default female: 'ngoc_huyen'). For a male voice, choose one of male_voices (default male: 'hai_dang').",
            ),
            VieneuModel::V3NanoFp32 => (
                "minh_quan",
                VIENEU_NANO_FEMALE_VOICES,
                VIENEU_NANO_MALE_VOICES,
                "For a female voice on VieNeu v3 Nano, choose one of ['my_duyen', 'mai_anh', 'truc_ly', 'ai_han'] (default female: 'my_duyen'). For a male voice, choose one of male_voices (default male: 'minh_quan').",
            ),
        },
        "zerotts_onnx" => (
            "giahuy",
            ZEROTTS_FEMALE_VOICES,
            ZEROTTS_MALE_VOICES,
            "For a female voice on ZeroTTS, choose 'maichi', 'baotrang', 'hamy', or 'kimoanh'. For a male voice, choose 'giahuy', 'huuduc', 'quangminh', or 'tiendat'.",
        ),
        "edge_tts" => (
            "vi-VN-HoaiMyNeural",
            EDGE_ADVERTISED_FEMALE_VOICES,
            EDGE_ADVERTISED_MALE_VOICES,
            "For Vietnamese female narration choose 'vi-VN-HoaiMyNeural'; for Vietnamese male narration choose 'vi-VN-NamMinhNeural'.",
        ),
        "kokoro_vi_onnx" => (
            "diem_trinh",
            KOKORO_FEMALE_VOICES,
            KOKORO_MALE_VOICES,
            "For a female voice on Kokoro Vietnamese, choose 'ngoc_huyen' or 'diem_trinh'. For a male voice, choose 'manh_dung' or 'duc_an'.",
        ),
        "chillaudio_ws" => (
            "BV421_vivn_streaming",
            CHILLAUDIO_FEMALE_VOICES,
            CHILLAUDIO_MALE_VOICES,
            "For a female voice on ChillAudio, choose 'BV421_vivn_streaming', 'vi_female_huong', or 'BV074_streaming'. For a male voice, choose 'BV075_streaming'.",
        ),
        "vifasttts_onnx" => (
            "ngoc_huyen",
            VIFASTTTS_FEMALE_VOICES,
            VIFASTTTS_MALE_VOICES,
            "ViFastTTS supports a single Vietnamese female voice: 'ngoc_huyen'.",
        ),
        _ => (
            "alloy",
            OPENAI_FEMALE_VOICES,
            OPENAI_MALE_VOICES,
            "For a female voice on OpenAI Speech, choose 'nova', 'shimmer', 'coral', or 'sage'. For a male voice, choose 'onyx', 'echo', 'ash', 'ballad', or 'fable'.",
        ),
    };

    let resolved_default = runtime_default_voice
        .and_then(|v| match_adapter_voice(adapter, vieneu_model, v).map(|(id, _)| id))
        .or_else(|| {
            spec_voice.and_then(|v| match_adapter_voice(adapter, vieneu_model, v).map(|(id, _)| id))
        })
        .unwrap_or_else(|| fallback_default.to_owned());

    AdapterVoiceCatalog {
        default_voice: resolved_default,
        female_voices: female_slice.iter().map(|s| (*s).to_owned()).collect(),
        male_voices: male_slice.iter().map(|s| (*s).to_owned()).collect(),
        guidance: guidance.to_owned(),
    }
}

fn is_local_tts_adapter(adapter: &str) -> bool {
    matches!(
        adapter,
        "vieneu"
            | "zerotts_onnx"
            | "edge_tts"
            | "kokoro_vi_onnx"
            | "chillaudio_ws"
            | "vifasttts_onnx"
            | "qualification_tts"
    )
}

/// Phân giải mã giọng thực tế (`effective_voice`) dựa trên `adapter`, `model_or_voice_spec`,
/// `runtime_default_voice` và `requested_voice` do Agent/Caller truyền vào.
pub fn resolve_tts_voice(
    adapter: &str,
    model_or_voice_spec: Option<&str>,
    runtime_default_voice: Option<&str>,
    requested_voice: Option<&str>,
) -> String {
    let vieneu_model = resolve_vieneu_model(model_or_voice_spec, runtime_default_voice);
    let catalog = catalog_for_adapter(adapter, model_or_voice_spec, runtime_default_voice);

    let Some(raw) = requested_voice.map(str::trim).filter(|s| !s.is_empty()) else {
        return catalog.default_voice;
    };

    // Bước 1: Giá trị trung tính / mặc định
    if raw.eq_ignore_ascii_case("default")
        || raw.eq_ignore_ascii_case("vi-default")
        || raw.eq_ignore_ascii_case("vi_default")
        || (raw.eq_ignore_ascii_case("alloy") && is_local_tts_adapter(adapter))
    {
        return catalog.default_voice;
    }

    // Bước 2: Khớp trực tiếp Voice ID của Adapter/Model hiện hành
    if let Some((canonical_id, _)) = match_adapter_voice(adapter, vieneu_model, raw) {
        return canonical_id;
    }

    // Bước 3: Phân giải theo Ý định Giới tính (`parse_gender_intent` hoặc giọng chéo Turbo <-> Nano trong `vieneu`)
    let target_gender = parse_gender_intent(raw).or_else(|| {
        if adapter == "vieneu" {
            vieneu_family_gender(raw)
        } else {
            None
        }
    });

    if let Some(gender) = target_gender {
        let default_gender = match_adapter_voice(adapter, vieneu_model, &catalog.default_voice)
            .map(|(_, g)| g);
        if default_gender == Some(gender) {
            return catalog.default_voice;
        }
        return match gender {
            VoiceGender::Female => catalog
                .female_voices
                .first()
                .cloned()
                .unwrap_or(catalog.default_voice),
            VoiceGender::Male => catalog
                .male_voices
                .first()
                .cloned()
                .unwrap_or(catalog.default_voice),
        };
    }

    // Bước 4: Fallback an toàn về giọng mặc định đã cấu hình của provider
    catalog.default_voice
}
