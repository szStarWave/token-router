//! WordLevel tokenizers built from wordfreq vocabulary (huggingface/tokenizers).

use std::collections::HashSet;

use ahash::AHashMap;
use tokenizers::models::wordlevel::WordLevel;
use tokenizers::pre_tokenizers::whitespace::Whitespace;
use tokenizers::Tokenizer;

use super::lexical::LexicalLang;

pub const UNK_TOKEN: &str = "<unk>";

pub fn build_wordlevel(words: &[String], lang: LexicalLang) -> anyhow::Result<Tokenizer> {
    let mut vocab = AHashMap::new();
    vocab.insert(UNK_TOKEN.to_string(), 0);
    for (i, word) in words.iter().enumerate() {
        if word.is_empty() {
            continue;
        }
        vocab.insert(word.clone(), (i + 1) as u32);
    }
    let model = WordLevel::builder()
        .vocab(vocab)
        .unk_token(UNK_TOKEN.to_string())
        .build()
        .map_err(|e| anyhow::anyhow!("build WordLevel model: {e}"))?;
    let mut tokenizer = Tokenizer::new(model);
    if lang == LexicalLang::En {
        tokenizer.with_pre_tokenizer(Some(Whitespace));
    }
    Ok(tokenizer)
}

pub fn tokenize_text(
    tokenizer: &Tokenizer,
    words: &[String],
    text: &str,
    lang: LexicalLang,
) -> Vec<String> {
    match lang {
        LexicalLang::En => tokenize_english(tokenizer, text, lang),
        LexicalLang::Zh | LexicalLang::Ja | LexicalLang::Ko => {
            greedy_longest_match(text, words, lang)
        }
    }
}

fn tokenize_english(tokenizer: &Tokenizer, text: &str, lang: LexicalLang) -> Vec<String> {
    let Ok(encoding) = tokenizer.encode(text, false) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (token, (start, end)) in encoding
        .get_tokens()
        .iter()
        .zip(encoding.get_offsets())
    {
        if token == UNK_TOKEN {
            if *end > *start && *start < text.len() {
                let end = (*end).min(text.len());
                let piece = &text[*start..end];
                if keep_token(piece) {
                    out.push(normalize_token(piece, lang));
                }
            }
            continue;
        }
        if keep_token(token) {
            out.push(normalize_token(token, lang));
        }
    }
    out
}

/// Greedy longest-match segmentation for scripts without whitespace (CJK).
fn greedy_longest_match(text: &str, words: &[String], lang: LexicalLang) -> Vec<String> {
    let vocab: HashSet<&str> = words.iter().map(String::as_str).collect();
    let max_len = words.iter().map(|w| w.chars().count()).max().unwrap_or(1);
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let mut matched: Option<String> = None;
        let mut match_len = 0usize;
        let try_max = max_len.min(chars.len() - i);
        for len in (1..=try_max).rev() {
            let candidate: String = chars[i..i + len].iter().collect();
            if vocab.contains(candidate.as_str()) {
                matched = Some(candidate);
                match_len = len;
                break;
            }
        }
        if let Some(word) = matched {
            if keep_token(&word) {
                out.push(normalize_token(&word, lang));
            }
            i += match_len;
        } else {
            let ch: String = chars[i].to_string();
            if keep_token(&ch) {
                out.push(normalize_token(&ch, lang));
            }
            i += 1;
        }
    }
    out
}

fn normalize_token(token: &str, lang: LexicalLang) -> String {
    if lang == LexicalLang::En && token.is_ascii() {
        token.to_ascii_lowercase()
    } else {
        token.to_string()
    }
}

fn keep_token(token: &str) -> bool {
    token.chars().any(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_whitespace_words() {
        let words = vec![
            "hello".to_string(),
            "how".to_string(),
            "the".to_string(),
            "weather".to_string(),
            "today".to_string(),
        ];
        let tok = build_wordlevel(&words, LexicalLang::En).unwrap();
        let tokens = tokenize_text(&tok, &words, "Hello, how is the weather today", LexicalLang::En);
        assert!(tokens.iter().any(|t| t == "hello"));
        assert!(tokens.iter().any(|t| t == "weather"));
    }

    #[test]
    fn chinese_longest_match() {
        let words = vec!["你好".to_string(), "今天".to_string(), "天气".to_string()];
        let tok = build_wordlevel(&words, LexicalLang::Zh).unwrap();
        let tokens = tokenize_text(&tok, &words, "你好今天天气", LexicalLang::Zh);
        assert!(tokens.contains(&"你好".to_string()));
        assert!(tokens.contains(&"天气".to_string()));
    }
}
