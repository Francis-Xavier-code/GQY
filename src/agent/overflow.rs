use crate::llm::{ChatContent, ChatMessage, Usage};

const CHARS_PER_TOKEN_LATIN: usize = 4;
const CHARS_PER_TOKEN_CJK: usize = 2;
const IMAGE_TOKEN_ESTIMATE: usize = 765;
const RESERVED_RATIO: f32 = 0.1;
const MIN_RESERVED_TOKENS: usize = 4096;

pub struct OverflowCheck {
    pub context_window: Option<usize>,
    pub reserved_tokens: usize,
    pub trim_at_ratio: f32,
}

impl OverflowCheck {
    pub fn new(
        context_window: Option<usize>,
        trim_at_ratio: f32,
        reserved_tokens: Option<usize>,
    ) -> Self {
        let reserved_tokens = reserved_tokens.unwrap_or_else(|| {
            context_window
                .map(|w| ((w as f32 * RESERVED_RATIO) as usize).max(MIN_RESERVED_TOKENS))
                .unwrap_or(MIN_RESERVED_TOKENS)
        });
        Self {
            context_window,
            reserved_tokens,
            trim_at_ratio,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.context_window.is_some()
    }

    #[allow(dead_code)]
    pub fn usable_tokens(&self) -> Option<usize> {
        self.context_window
            .map(|w| w.saturating_sub(self.reserved_tokens))
    }

    pub fn threshold(&self) -> Option<usize> {
        self.context_window
            .map(|w| (w as f32 * self.trim_at_ratio).max(1.0) as usize)
    }

    pub fn check_usage(&self, usage: &Usage) -> bool {
        let Some(threshold) = self.threshold() else {
            return false;
        };
        usage.total_tokens as usize >= threshold
    }

    #[allow(dead_code)]
    pub fn check_estimate(&self, messages: &[ChatMessage]) -> bool {
        let Some(threshold) = self.threshold() else {
            return false;
        };
        estimate_messages_tokens(messages) >= threshold
    }
}

#[allow(dead_code)]
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    let tokens: usize = messages.iter().map(message_tokens).sum();
    tokens.max(1)
}

pub fn estimate_tokens(text: &str) -> usize {
    text_tokens(text).max(1)
}

fn text_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut latin = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            latin += 1;
        }
    }
    cjk / CHARS_PER_TOKEN_CJK + latin / CHARS_PER_TOKEN_LATIN
}

fn is_cjk(ch: char) -> bool {
    let code = ch as u32;
    (0x4E00..=0x9FFF).contains(&code)      // CJK Unified Ideographs
        || (0x3400..=0x4DBF).contains(&code) // CJK Extension A
        || (0x20000..=0x2A6DF).contains(&code) // CJK Extension B
        || (0x3040..=0x30FF).contains(&code) // Hiragana + Katakana
        || (0xAC00..=0xD7AF).contains(&code) // Hangul Syllables
        || (0xFF00..=0xFFEF).contains(&code) // Fullwidth Forms
}

#[allow(dead_code)]
fn message_tokens(msg: &ChatMessage) -> usize {
    let role_tokens = text_tokens(&msg.role);
    let content_tokens = match &msg.content {
        Some(ChatContent::Text(s)) => text_tokens(s),
        Some(ChatContent::Parts(parts)) => parts
            .iter()
            .map(|p| match p {
                crate::llm::ChatContentPart::Text { text } => text_tokens(text),
                crate::llm::ChatContentPart::ImageUrl { .. } => IMAGE_TOKEN_ESTIMATE,
            })
            .sum(),
        None => 0,
    };
    let tool_tokens = msg
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|c| text_tokens(&c.function.name) + text_tokens(&c.function.arguments))
                .sum::<usize>()
        })
        .unwrap_or(0);
    role_tokens + content_tokens + tool_tokens
}
