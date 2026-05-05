use crate::context::LineContext;
use crate::ime::InputMode;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharKind {
    Chinese,
    English,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionReason {
    CurrentCharChinese,
    CurrentCharEnglish,
    NeighborChineseBias,
    NeighborEnglishBias,
    DefaultEnglish,
}

impl fmt::Display for DecisionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentCharChinese => write!(f, "current_char_chinese"),
            Self::CurrentCharEnglish => write!(f, "current_char_english"),
            Self::NeighborChineseBias => write!(f, "neighbor_chinese_bias"),
            Self::NeighborEnglishBias => write!(f, "neighbor_english_bias"),
            Self::DefaultEnglish => write!(f, "default_english"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub mode: InputMode,
    pub reason: DecisionReason,
}

pub fn classify(context: &LineContext) -> Decision {
    match classify_char(context.current_char()) {
        CharKind::Chinese => {
            return Decision {
                mode: InputMode::Chinese,
                reason: DecisionReason::CurrentCharChinese,
            };
        }
        CharKind::English => {
            return Decision {
                mode: InputMode::English,
                reason: DecisionReason::CurrentCharEnglish,
            };
        }
        CharKind::Other => {}
    }

    let previous = nearest_signal(context.chars_before_cursor());
    let next = nearest_signal(context.chars_after_cursor());

    match (previous, next) {
        (CharKind::Chinese, CharKind::Chinese)
        | (CharKind::Chinese, CharKind::Other)
        | (CharKind::Other, CharKind::Chinese) => Decision {
            mode: InputMode::Chinese,
            reason: DecisionReason::NeighborChineseBias,
        },
        (CharKind::English, CharKind::English)
        | (CharKind::English, CharKind::Other)
        | (CharKind::Other, CharKind::English) => Decision {
            mode: InputMode::English,
            reason: DecisionReason::NeighborEnglishBias,
        },
        _ => Decision {
            mode: InputMode::English,
            reason: DecisionReason::DefaultEnglish,
        },
    }
}

fn classify_char(ch: Option<char>) -> CharKind {
    match ch {
        Some(value) if is_cjk(value) => CharKind::Chinese,
        Some(value) if value.is_ascii_alphabetic() => CharKind::English,
        Some(_) | None => CharKind::Other,
    }
}

fn nearest_signal(chars: impl Iterator<Item = char>) -> CharKind {
    chars
        .map(|ch| classify_char(Some(ch)))
        .find(|kind| *kind != CharKind::Other)
        .unwrap_or(CharKind::Other)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x2F800..=0x2FA1F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(line: &str, cursor: usize) -> LineContext {
        LineContext::new(line.to_string(), cursor).unwrap()
    }

    #[test]
    fn picks_chinese_when_cursor_is_after_chinese_char() {
        let decision = classify(&context("abc中文", 4)); //  cursor is after the Chinese character '中'
        assert_eq!(decision.mode, InputMode::Chinese);
    }

    #[test]
    fn picks_english_when_cursor_is_after_english_char() {
        let decision = classify(&context("中文abc", 3)); //  cursor is after the English character 'a'
        assert_eq!(decision.mode, InputMode::English);
    }

    #[test]
    fn uses_neighbor_bias_for_punctuation_gap() {
        let decision = classify(&context("中文 () 中文", 3)); //  cursor is after the punctuation character '('
        assert_eq!(decision.mode, InputMode::Chinese);
    }

    #[test]
    fn falls_back_to_english_when_no_signal_exists() {
        let decision = classify(&context("12345", 2)); //  cursor is after the number character '2'
        assert_eq!(decision.mode, InputMode::English);
        assert_eq!(decision.reason, DecisionReason::DefaultEnglish);
    }
}
