//! Things inside a sentence that must come out exactly as they went in: web addresses, e-mail
//! addresses and (for Markdown) code and link targets. They are swapped for placeholder tokens
//! before translation and put back afterwards, the same way the glossary works.

/// `(token, original)` pairs of one text.
pub type Tokens = Vec<(String, String)>;

fn token(index: usize) -> String {
    format!("Uq{index}x")
}

fn is_trailing_punctuation(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\'' | '\u{201d}' | '\u{2019}' | '\u{bb}')
}

/// The byte range of the web or e-mail address that starts at `start`, if one does.
fn address_at(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    let lower: String = rest.chars().take(8).collect::<String>().to_lowercase();
    let is_url = lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ftp://") || lower.starts_with("www.") || lower.starts_with("mailto:");
    if is_url {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        return Some(start + trim_end_punctuation(&rest[..end]).len());
    }
    // name@host.tld
    let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let word = trim_end_punctuation(&rest[..word_end]);
    let at = word.find('@')?;
    let (local, host) = (&word[..at], &word[at + 1..]);
    let local_ok = !local.is_empty() && local.chars().all(|c| c.is_alphanumeric() || "._%+-".contains(c));
    let host_ok = host.contains('.') && host.chars().all(|c| c.is_alphanumeric() || ".-".contains(c)) && !host.starts_with('.') && !host.ends_with('.');
    (local_ok && host_ok).then_some(start + word.len())
}

fn trim_end_punctuation(word: &str) -> &str {
    word.trim_end_matches(is_trailing_punctuation)
}

/// Replace web and e-mail addresses by tokens.
pub fn protect_addresses(text: &str, counter: &mut usize) -> (String, Tokens) {
    let mut out = String::with_capacity(text.len());
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut at_word_start = true;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap_or(' ');
        if at_word_start {
            if let Some(end) = address_at(text, i) {
                if end > i {
                    *counter += 1;
                    let t = token(*counter);
                    out.push_str(&t);
                    tokens.push((t, text[i..end].to_string()));
                    i = end;
                    at_word_start = false;
                    continue;
                }
            }
        }
        out.push(c);
        at_word_start = c.is_whitespace() || matches!(c, '(' | '[' | '<' | '"' | '\u{201c}' | '\u{ab}');
        i += c.len_utf8();
    }
    (out, tokens)
}

/// Put the originals back. A token the model dropped is simply gone (its text is not invented).
pub fn restore(text: &str, tokens: &Tokens) -> String {
    let mut out = text.to_string();
    for (token, original) in tokens {
        out = out.replacen(token.as_str(), original, 1);
    }
    out
}

/// Add a token for an arbitrary span (Markdown code, link targets), numbering from `counter`.
pub fn push_token(counter: &mut usize, tokens: &mut Tokens, original: &str) -> String {
    *counter += 1;
    let t = token(*counter);
    tokens.push((t.clone(), original.to_string()));
    t
}

/// Does `text` contain something to translate: at least one letter, and not just an address, a
/// formula or an identifier?
pub fn is_translatable(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.chars().any(char::is_alphabetic) {
        return false;
    }
    if trimmed.starts_with('=') {
        return false;
    }
    let single_token = !trimmed.contains(char::is_whitespace);
    if single_token {
        if address_at(trimmed, 0) == Some(trimmed.len()) {
            return false;
        }
        // identifiers and paths: snake_case, a/b/c, A1B2, camelCase with digits, file names
        let looks_like_code = trimmed.contains(['_', '/', '\\', '{', '}', '<', '>', '(', ')']) || (trimmed.chars().any(|c| c.is_ascii_digit()) && trimmed.chars().any(char::is_alphabetic));
        if looks_like_code {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_addresses_are_protected_and_restored() {
        let mut n = 0;
        let (text, tokens) = protect_addresses("Visit https://example.org/a?b=1, or mail me@example.com. See www.example.org.", &mut n);
        assert_eq!(text, "Visit Uq1x, or mail Uq2x. See Uq3x.");
        assert_eq!(tokens.len(), 3);
        assert_eq!(restore("Besuchen Sie Uq1x oder Uq2x. Siehe Uq3x.", &tokens), "Besuchen Sie https://example.org/a?b=1 oder me@example.com. Siehe www.example.org.");
    }

    #[test]
    fn plain_text_and_at_signs_in_words_are_left_alone() {
        let mut n = 0;
        let (text, tokens) = protect_addresses("Price: 5 @ home, nothing@ here, @user.", &mut n);
        assert_eq!(text, "Price: 5 @ home, nothing@ here, @user.");
        assert!(tokens.is_empty());
    }

    #[test]
    fn addresses_in_brackets_and_quotes_are_found() {
        let mut n = 0;
        let (text, tokens) = protect_addresses("(see https://example.org) and \"mailto:a@b.de\"", &mut n);
        assert_eq!(text, "(see Uq1x) and \"Uq2x\"");
        assert_eq!(tokens[0].1, "https://example.org");
        assert_eq!(tokens[1].1, "mailto:a@b.de");
    }

    #[test]
    fn numbers_formulas_addresses_and_identifiers_are_not_translatable() {
        for text in ["12,345.67", "2024-05-01", "=SUM(A1:A3)", "https://example.org", "me@example.com", "snake_case_name", "C:\\temp\\file", "A1B2", "   ", "42%", "(1)"] {
            assert!(!is_translatable(text), "{text:?}");
        }
        for text in ["Hello", "Hello world", "Total: 5", "Q1 results", "file.txt is here", "Straße"] {
            assert!(is_translatable(text), "{text:?}");
        }
    }
}
