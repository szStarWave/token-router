//! Statistical rare-word detection via wordfreq + tokenizers WordLevel segmentation.

use whatlang::{Lang, detect};

use super::keywords::{contains_easy_intent, contains_special_lexical};
use super::wordfreq_store::WordFreqStore;

const RARE_FREQ_THRESHOLD: f32 = 1e-7;
const RARE_RATIO_THRESHOLD: f32 = 0.25;
const MIN_TOKENS_FOR_RATIO: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LexicalSignals {
    pub rare_lexical: bool,
    pub special_lexical: bool,
    pub rare_token_ratio: f32,
}

impl LexicalSignals {
    pub fn none() -> Self {
        Self {
            rare_lexical: false,
            special_lexical: false,
            rare_token_ratio: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LexicalLang {
    En,
    Zh,
    Ja,
    Ko,
}

pub fn analyze_lexical(text: &str, store: &WordFreqStore) -> LexicalSignals {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return LexicalSignals::none();
    }

    let special_lexical = contains_special_lexical(trimmed);
    if contains_easy_intent(trimmed) && !special_lexical {
        store.observe_casual(trimmed);
        return LexicalSignals {
            rare_lexical: false,
            special_lexical,
            rare_token_ratio: 0.0,
        };
    }

    let lang = detect_lexical_lang(trimmed);
    let tokens = store.tokenize(trimmed, lang);
    if tokens.is_empty() {
        return LexicalSignals {
            rare_lexical: false,
            special_lexical,
            rare_token_ratio: 0.0,
        };
    }

    let mut rare_count = 0usize;
    let mut ultra_rare = false;

    for token in &tokens {
        if !token_counts_for_rarity(token) {
            continue;
        }
        let freq = store.word_frequency(lang, token);
        if freq < RARE_FREQ_THRESHOLD {
            rare_count += 1;
            if freq == 0.0 || freq < 1e-9 {
                ultra_rare = true;
            }
        }
    }

    let scored = tokens.iter().filter(|t| token_counts_for_rarity(t)).count();
    let rare_token_ratio = if scored == 0 {
        0.0
    } else {
        rare_count as f32 / scored as f32
    };

    let rare_lexical = ultra_rare
        || rare_count >= 2
        || (scored >= MIN_TOKENS_FOR_RATIO && rare_token_ratio >= RARE_RATIO_THRESHOLD);

    LexicalSignals {
        rare_lexical,
        special_lexical,
        rare_token_ratio,
    }
}

pub(crate) fn detect_lexical_lang(text: &str) -> LexicalLang {
    if let Some(info) = detect(text) {
        match info.lang() {
            Lang::Eng => return LexicalLang::En,
            Lang::Jpn => return LexicalLang::Ja,
            Lang::Kor => return LexicalLang::Ko,
            Lang::Cmn => return LexicalLang::Zh,
            _ => {}
        }
    }
    script_lexical_lang(text)
}

fn script_lexical_lang(text: &str) -> LexicalLang {
    let mut han = 0u32;
    let mut kana = 0u32;
    let mut hangul = 0u32;
    let mut latin = 0u32;

    for ch in text.chars() {
        if is_cjk(ch) {
            han += 1;
        } else if is_hiragana_katakana(ch) {
            kana += 1;
        } else if ('\u{AC00}'..='\u{D7A3}').contains(&ch) {
            hangul += 1;
        } else if ch.is_ascii_alphabetic() {
            latin += 1;
        }
    }

    if kana > han && kana > hangul {
        LexicalLang::Ja
    } else if hangul > han && hangul > latin {
        LexicalLang::Ko
    } else if han > latin {
        LexicalLang::Zh
    } else {
        LexicalLang::En
    }
}

pub(crate) fn token_counts_for_rarity(token: &str) -> bool {
    let alnum_len = token.chars().filter(|c| c.is_alphanumeric()).count();
    if token.is_ascii() {
        return alnum_len >= 3;
    }
    alnum_len >= 1
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F | 0x2B820..=0x2CEAF | 0xF900..=0xFAFF | 0x2F800..=0x2FA1F
    )
}

fn is_hiragana_katakana(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x309F | 0x30A0..=0x30FF | 0xFF66..=0xFF9D)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    static TEST_STORE: LazyLock<WordFreqStore> =
        LazyLock::new(|| WordFreqStore::open_in_memory().expect("test wordfreq store"));

    fn store() -> &'static WordFreqStore {
        &TEST_STORE
    }

    #[test]
    fn english_rare_vs_common() {
        let rare = analyze_lexical("The epistemological paradigm shift", store());
        assert!(rare.rare_lexical, "expected rare for academic English");
        let common = analyze_lexical("Hello, how is the weather today", store());
        assert!(!common.rare_lexical, "expected common English not rare");
    }

    #[test]
    fn chinese_rare_vs_common() {
        let rare = analyze_lexical("认识论范式下的本体论谱系", store());
        assert!(rare.rare_lexical, "expected rare Chinese");
        let common = analyze_lexical("你好，今天天气怎么样", store());
        assert!(!common.rare_lexical, "expected common Chinese not rare");
    }

    #[test]
    fn easy_intent_skips_rare_without_special() {
        let s = analyze_lexical("你好，今天天气怎么样", store());
        assert!(!s.rare_lexical);
        assert!(!s.special_lexical);
    }

    #[test]
    fn special_lexical_from_keywords() {
        let s = analyze_lexical("Please audit GDPR data handling", store());
        assert!(s.special_lexical);
    }

    #[test]
    fn japanese_rare_sample() {
        let rare = analyze_lexical("認識論的パラダイム転換の本体論", store());
        assert!(rare.rare_lexical);
    }

    #[test]
    fn korean_rare_sample() {
        let rare = analyze_lexical("인식론적 패러다임 전환의 존재론", store());
        assert!(rare.rare_lexical);
    }

    #[test]
    fn yue_common_not_rare() {
        let common = analyze_lexical("你好，今日天气点呀", store());
        assert!(!common.rare_lexical);
    }
}
