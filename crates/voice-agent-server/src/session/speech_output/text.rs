use crate::config::SpeechOutputConfig;
use unicode_normalization::UnicodeNormalization;

#[derive(Default)]
pub(super) struct JsonFilter {
    pub(super) candidate: String,
    depth: usize,
    in_string: bool,
    escaped: bool,
    last_emitted: Option<char>,
    skip_next_space: bool,
}
impl JsonFilter {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(super) fn push(&mut self, delta: &str) -> String {
        let mut output = String::new();
        for ch in delta.chars() {
            if self.depth == 0 {
                if matches!(ch, '{' | '[') {
                    self.depth = 1;
                    self.candidate.push(ch);
                } else {
                    if self.skip_next_space && ch == ' ' {
                        self.skip_next_space = false;
                        continue;
                    }
                    self.skip_next_space = false;
                    output.push(ch);
                    self.last_emitted = Some(ch);
                }
                continue;
            }
            self.candidate.push(ch);
            if self.escaped {
                self.escaped = false;
            } else if ch == '\\' && self.in_string {
                self.escaped = true;
            } else if ch == '"' {
                self.in_string = !self.in_string;
            } else if !self.in_string {
                match ch {
                    '{' | '[' => self.depth += 1,
                    '}' | ']' => self.depth -= 1,
                    _ => {}
                }
            }
            if self.depth == 0 {
                if serde_json::from_str::<serde_json::Value>(&self.candidate).is_ok() {
                    if self.last_emitted.is_some_and(char::is_alphanumeric) {
                        output.push(' ');
                        self.last_emitted = Some(' ');
                    }
                    self.skip_next_space = self.last_emitted == Some(' ');
                } else {
                    output.push_str(&self.candidate);
                    self.last_emitted = Some(ch);
                    self.skip_next_space = false;
                }
                self.candidate.clear();
            }
        }
        output
    }
    pub(super) fn finish(&mut self) -> String {
        self.depth = 0;
        self.in_string = false;
        self.escaped = false;
        std::mem::take(&mut self.candidate)
    }
}

pub(super) struct SentenceSegmenter {
    pub(super) config: SpeechOutputConfig,
    pub(super) buffer: String,
}
impl SentenceSegmenter {
    pub(super) fn new(config: SpeechOutputConfig) -> Self {
        Self {
            config,
            buffer: String::new(),
        }
    }
    pub(super) fn reset(&mut self) {
        self.buffer.clear();
    }
    pub(super) fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut segments = Vec::new();
        loop {
            let boundary = self.buffer.char_indices().find_map(|(index, ch)| {
                let next = self.buffer[index + ch.len_utf8()..].chars().next();
                let previous = self.buffer[..index].chars().next_back();
                let sentence_dot = ch == '.'
                    && match next {
                        Some(next) => {
                            next.is_whitespace()
                                || matches!(next, '"' | '\'' | ')' | ']' | '”' | '’')
                        }
                        None => previous.is_some_and(|previous| !previous.is_ascii_digit()),
                    };
                (sentence_dot || matches!(ch, '!' | '?' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}'))
                    .then_some(index + ch.len_utf8())
            });
            let Some(split) = boundary else { break };
            let segment = self.buffer[..split].trim().to_owned();
            self.buffer.drain(..split);
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
        segments
    }
    pub(super) fn finish(&mut self) -> Option<String> {
        let text = self.buffer.trim().to_owned();
        self.buffer.clear();
        (!text.is_empty()).then_some(text)
    }
}
pub(super) fn sanitize_tts_text(input: &str) -> String {
    let normalized = input.nfc().collect::<String>();
    let mut output = String::with_capacity(normalized.len());
    let mut pending_space = false;
    for (index, ch) in normalized.char_indices() {
        let between_digits = normalized[..index]
            .chars()
            .next_back()
            .is_some_and(|previous| previous.is_ascii_digit())
            && normalized[index + ch.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_digit());
        if ch.is_alphanumeric() {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            output.push(ch);
        } else if matches!(ch, '/' | '-') && between_digits {
            pending_space = false;
            output.push(ch);
        } else if ch.is_whitespace() || matches!(ch, '_' | '-' | '–' | '—') {
            pending_space = true;
        } else if matches!(ch, '.' | ',' | '!' | '?' | ';' | ':') {
            pending_space = false;
            output.push(ch);
        }
    }
    output.trim().to_owned()
}
pub(super) fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}
