//! Target-language tokens (`>>urd<<`, `>>jpn<<`, `>>ara<<`) for multi-target Opus-MT models.
//!
//! These tokens are single entries in the model's vocabulary. Handing `">>urd<< Hello"` to
//! SentencePiece as plain text splits the token into pieces and the model produces garbage, so
//! the token has to be put in front of the encoded sentence as one token (this is what the
//! Hugging Face `MarianTokenizer` does). Some older models do know the token to SentencePiece;
//! for a token that is not in the model's vocabulary the text is encoded as before.

use std::collections::HashSet;
use std::path::Path;

use ct2rs::tokenizers::sentencepiece::Tokenizer as Spm;

/// The `>>xxx<<` tokens in a converted model's vocabulary.
pub fn language_tokens(model_dir: &Path) -> HashSet<String> {
    for name in ["shared_vocabulary.json", "source_vocabulary.json"] {
        let Ok(text) = std::fs::read_to_string(model_dir.join(name)) else { continue };
        if let Ok(tokens) = serde_json::from_str::<Vec<String>>(&text) {
            return tokens.into_iter().filter(|t| t.starts_with(">>") && t.ends_with("<<")).collect();
        }
    }
    HashSet::new()
}

/// Split a leading `>>xxx<<` off `input` if `known` contains it.
///
/// Returns the token and the rest of the text; `(None, input)` when there is no leading token
/// or it is not a vocabulary token (then it stays part of the text).
pub fn split_language_token<'a>(input: &'a str, known: &HashSet<String>) -> (Option<&'a str>, &'a str) {
    if let Some(rest) = input.strip_prefix(">>") {
        if let Some(end) = rest.find("<<") {
            let token = &input[..end + 4];
            if known.contains(token) {
                return (Some(token), rest[end + 2..].trim_start());
            }
        }
    }
    (None, input)
}

/// SentencePiece plus single-token language prefixes.
pub struct LangTokenizer {
    spm: Spm,
    known: HashSet<String>,
}

impl LangTokenizer {
    /// Open `source.spm` / `target.spm` and read the language tokens from the model folder.
    pub fn open(model_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self { spm: Spm::new(model_dir)?, known: language_tokens(model_dir) })
    }
}

impl ct2rs::Tokenizer for LangTokenizer {
    fn encode(&self, input: &str) -> anyhow::Result<Vec<String>> {
        let (token, text) = split_language_token(input, &self.known);
        let mut tokens: Vec<String> = token.map(str::to_string).into_iter().collect();
        tokens.extend(ct2rs::Tokenizer::encode(&self.spm, text)?);
        Ok(tokens)
    }

    fn decode(&self, tokens: Vec<String>) -> anyhow::Result<String> {
        ct2rs::Tokenizer::decode(&self.spm, tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(tokens: &[&str]) -> HashSet<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn a_known_leading_token_is_split_off_as_one_token() {
        let k = known(&[">>urd<<", ">>jpn<<"]);
        assert_eq!(split_language_token(">>urd<< Good morning!", &k), (Some(">>urd<<"), "Good morning!"));
        assert_eq!(split_language_token(">>jpn<<Hello", &k), (Some(">>jpn<<"), "Hello"));
    }

    #[test]
    fn unknown_or_missing_tokens_stay_part_of_the_text() {
        let k = known(&[">>urd<<"]);
        // Not a vocabulary token: the plain-text path (older models) keeps working.
        assert_eq!(split_language_token(">>ara<< Hello", &k), (None, ">>ara<< Hello"));
        assert_eq!(split_language_token("Hello >>urd<< there", &k), (None, "Hello >>urd<< there"));
        assert_eq!(split_language_token("Hello", &k), (None, "Hello"));
        assert_eq!(split_language_token(">>urd Hello", &k), (None, ">>urd Hello"));
        assert_eq!(split_language_token("anything", &HashSet::new()), (None, "anything"));
    }

    #[test]
    fn reads_language_tokens_from_the_converted_vocabulary() {
        let dir = std::env::temp_dir().join(format!("dawn-langtok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shared_vocabulary.json"), r#"["<unk>", ">>urd<<", "▁Hello", ">>hin<<", ">>not a token"]"#).unwrap();
        let tokens = language_tokens(&dir);
        assert_eq!(tokens, known(&[">>urd<<", ">>hin<<"]));
        assert!(language_tokens(&dir.join("missing")).is_empty());
    }
}
