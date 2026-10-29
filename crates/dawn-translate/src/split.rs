//! Splitting text into sentences for batching, and putting translations back together.
//!
//! Line breaks, blank lines and leading indentation are preserved; only the text between
//! them goes through the model.
//!
//! Splitting is language-aware. A full stop after an abbreviation ("z. B.", "Dr.", "Mr.") or a
//! German ordinal ("3. Oktober") does not end a sentence: cutting a sentence in half makes
//! models produce garbage, while merging two sentences is harmless. To teach the splitter a
//! new language, add a [`Rules`] entry to [`rules_for`].

const TERMINATORS: &[char] = &['.', '!', '?', '…', '؟', '۔', '。', '！', '？'];
/// Terminators that end a sentence even without a following space (CJK).
const WIDE_TERMINATORS: &[char] = &['。', '！', '？'];
const CLOSERS: &[char] = &['"', '\'', ')', ']', '”', '’', '»', '」'];
const MAX_SENTENCE_CHARS: usize = 400;

/// Per-language knowledge about full stops that do not end a sentence.
pub struct Rules {
    /// Lowercase abbreviations written with a trailing full stop (without the stop itself).
    pub abbreviations: &'static [&'static str],
    /// A number followed by a full stop is an ordinal ("3. Oktober"), not a sentence end.
    pub numeric_ordinals: bool,
}

const GENERIC: Rules = Rules { abbreviations: &[], numeric_ordinals: false };

const GERMAN: Rules = Rules {
    abbreviations: &[
        "bzw", "ca", "usw", "etc", "vgl", "dr", "prof", "nr", "tel", "str", "mio", "mrd", "tsd", "evtl", "ggf", "inkl", "exkl",
        "sog", "bspw", "zzgl", "abb", "hr", "fr", "st", "geb", "gest", "ggü", "jh", "bes", "bez", "max", "min", "std", "kfz",
        "jan", "feb", "mär", "apr", "jun", "jul", "aug", "sept", "okt", "nov", "dez", "dipl", "ing", "mag", "mwst", "ust", "zt",
    ],
    numeric_ordinals: true,
};

const ENGLISH: Rules = Rules {
    abbreviations: &[
        "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "approx", "inc", "ltd", "co", "corp", "no", "fig", "al",
        "cf", "mt", "ave", "dept", "est", "govt", "gen", "sen", "rep", "rev", "col", "lt", "capt", "sgt", "pp", "vol", "ch", "jan",
        "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec", "min", "max", "tel", "mio",
    ],
    numeric_ordinals: false,
};

/// Rules for a language code such as "de". Unknown languages get the generic rules (initials,
/// "U.S." style abbreviations, and "next word starts in lowercase").
pub fn rules_for(lang: &str) -> &'static Rules {
    match lang {
        "de" => &GERMAN,
        "en" => &ENGLISH,
        _ => &GENERIC,
    }
}

#[derive(Debug, PartialEq)]
pub struct Line {
    /// Leading whitespace of the original line.
    pub indent: String,
    pub sentences: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct Document {
    pub lines: Vec<Line>,
    /// The original used `\r\n`.
    pub crlf: bool,
}

impl Document {
    pub fn sentence_count(&self) -> usize {
        self.lines.iter().map(|l| l.sentences.len()).sum()
    }

    pub fn sentences(&self) -> Vec<String> {
        self.lines.iter().flat_map(|l| l.sentences.iter().cloned()).collect()
    }
}

/// Split `text` for translation out of `lang` (the source language).
pub fn split_document(text: &str, lang: &str) -> Document {
    let crlf = text.contains("\r\n");
    let lines = text
        .split('\n')
        .map(|raw| {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let body = raw.trim_start();
            Line { indent: raw[..raw.len() - body.len()].to_string(), sentences: split_sentences(body.trim_end(), lang) }
        })
        .collect();
    Document { lines, crlf }
}

/// Split one line into sentences.
pub fn split_sentences(line: &str, lang: &str) -> Vec<String> {
    let rules = rules_for(lang);
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if !TERMINATORS.contains(&chars[i]) {
            i += 1;
            continue;
        }
        // The terminator run: "?!", "...", and closing quotes or brackets right after it.
        let first = chars[i];
        let mut end = i;
        while end + 1 < chars.len() && (TERMINATORS.contains(&chars[end + 1]) || CLOSERS.contains(&chars[end + 1])) {
            end += 1;
        }
        let at_end = end + 1 >= chars.len();
        let spaced = !at_end && chars[end + 1].is_whitespace();
        let wide = WIDE_TERMINATORS.contains(&first);
        let single_stop = first == '.' && end == i;

        let breaks = if !(at_end || spaced || wide) {
            false // "1.000", "3.5", "U.S.A"
        } else if single_stop && !at_end && !wide {
            !continues_after_stop(&chars, start, i, rules)
        } else {
            true
        };
        if breaks {
            push_sentence(&mut out, &chars[start..=end]);
            start = end + 1;
        }
        i = end + 1;
    }
    push_sentence(&mut out, &chars[start..]);
    out
}

/// Does the sentence go on after the full stop at `stop`?
fn continues_after_stop(chars: &[char], start: usize, stop: usize, rules: &Rules) -> bool {
    let word: String = chars[start..stop].iter().rev().take_while(|c| c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
    let lower = word.to_lowercase();
    let next = chars[stop + 1..].iter().find(|c| !c.is_whitespace()).copied();

    let initial = word.chars().count() == 1 && word.chars().all(char::is_alphabetic); // "z.", "B.", "U."
    let abbreviation = rules.abbreviations.contains(&lower.as_str());
    let ordinal = rules.numeric_ordinals && !word.is_empty() && word.chars().all(|c| c.is_ascii_digit());
    let lowercase_next = next.is_some_and(char::is_lowercase);
    initial || abbreviation || ordinal || lowercase_next
}

fn push_sentence(out: &mut Vec<String>, chars: &[char]) {
    let sentence: String = chars.iter().collect();
    let sentence = sentence.trim();
    if !sentence.is_empty() {
        out.extend(chunk_long(sentence));
    }
}

/// Break a very long sentence at spaces so it stays within the model's comfortable length.
fn chunk_long(sentence: &str) -> Vec<String> {
    if sentence.chars().count() <= MAX_SENTENCE_CHARS {
        return vec![sentence.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in sentence.split_inclusive(char::is_whitespace) {
        if current.chars().count() + word.chars().count() > MAX_SENTENCE_CHARS && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current.clear();
        }
        current.push_str(word);
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

/// Languages written without spaces between sentences.
fn joins_without_space(lang: &str) -> bool {
    matches!(lang, "zh" | "ja")
}

/// Rebuild the text from translated sentences, in the order `split_document` produced them.
///
/// If fewer translations than sentences are given, only the part translated so far is
/// returned (used to show a result while it is still being produced).
pub fn join_document(doc: &Document, translated: &[String], target_lang: &str) -> String {
    let sep = if joins_without_space(target_lang) { "" } else { " " };
    let newline = if doc.crlf { "\r\n" } else { "\n" };
    let partial = translated.len() < doc.sentence_count();
    let mut next = translated.iter();
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for line in &doc.lines {
        // Everything translated so far is already placed: do not add trailing blank lines.
        if partial && used >= translated.len() {
            break;
        }
        if line.sentences.is_empty() {
            lines.push(String::new());
            continue;
        }
        let parts: Vec<&str> = line.sentences.iter().filter_map(|_| next.next().map(String::as_str)).collect();
        used += parts.len();
        let complete = parts.len() == line.sentences.len();
        if !parts.is_empty() {
            lines.push(format!("{}{}", line.indent, parts.join(sep)));
        }
        if partial && !complete {
            break;
        }
    }
    lines.join(newline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de(line: &str) -> Vec<String> {
        split_sentences(line, "de")
    }

    fn en(line: &str) -> Vec<String> {
        split_sentences(line, "en")
    }

    #[test]
    fn splits_plain_sentences() {
        assert_eq!(de("Hallo Welt. Wie geht es dir? Gut!"), vec!["Hallo Welt.", "Wie geht es dir?", "Gut!"]);
        assert_eq!(en("Hello there. How are you? Fine!"), vec!["Hello there.", "How are you?", "Fine!"]);
        assert_eq!(en("no terminator"), vec!["no terminator"]);
        assert!(en("   ").is_empty());
    }

    #[test]
    fn german_abbreviations_do_not_end_sentences() {
        assert_eq!(de("Das ist z. B. ein Test. Danach geht es weiter."), vec!["Das ist z. B. ein Test.", "Danach geht es weiter."]);
        assert_eq!(de("Dr. Müller kam. Prof. Schmidt ging."), vec!["Dr. Müller kam.", "Prof. Schmidt ging."]);
        assert_eq!(de("Es dauert ca. 20 Minuten."), vec!["Es dauert ca. 20 Minuten."]);
        assert_eq!(de("Im Sommer bzw. im Herbst."), vec!["Im Sommer bzw. im Herbst."]);
        assert_eq!(de("Restaurants usw. kosten mehr."), vec!["Restaurants usw. kosten mehr."]);
        assert_eq!(de("Wir kaufen Obst, Gemüse etc. Danach gehen wir."), vec!["Wir kaufen Obst, Gemüse etc. Danach gehen wir."]);
        assert_eq!(de("Rufen Sie an (Tel. 0761 / 12345) oder schreiben Sie."), vec!["Rufen Sie an (Tel. 0761 / 12345) oder schreiben Sie."]);
    }

    #[test]
    fn german_numbers_and_ordinals() {
        assert_eq!(de("Mehr als 1.000.000 Besucher kamen."), vec!["Mehr als 1.000.000 Besucher kamen."]);
        assert_eq!(de("Am 3. Oktober feiern wir. Danach nicht."), vec!["Am 3. Oktober feiern wir.", "Danach nicht."]);
        assert_eq!(de("Die Stadt investiert 2,5 Mio. Euro in den Ausbau."), vec!["Die Stadt investiert 2,5 Mio. Euro in den Ausbau."]);
        assert_eq!(de("Das kostet 3,50 Euro. Teuer!"), vec!["Das kostet 3,50 Euro.", "Teuer!"]);
    }

    #[test]
    fn english_abbreviations_and_numbers() {
        assert_eq!(en("Mr. Smith arrived. He sat down."), vec!["Mr. Smith arrived.", "He sat down."]);
        assert_eq!(en("She lives in the U.S. now. It is nice."), vec!["She lives in the U.S. now.", "It is nice."]);
        assert_eq!(en("Pi is about 3.5 or so. Next."), vec!["Pi is about 3.5 or so.", "Next."]);
        assert_eq!(en("See e.g. the manual. Done."), vec!["See e.g. the manual.", "Done."]);
        assert_eq!(en("It was in 1990. The war ended."), vec!["It was in 1990.", "The war ended."]);
        assert_eq!(en("Dr. Jones and Mrs. Lee met."), vec!["Dr. Jones and Mrs. Lee met."]);
    }

    #[test]
    fn german_ordinal_rule_is_language_specific() {
        // The same text splits differently depending on the language.
        assert_eq!(de("Er wurde 1990. Danach zog er um."), vec!["Er wurde 1990. Danach zog er um."]);
        assert_eq!(en("He was born in 1990. Then he moved."), vec!["He was born in 1990.", "Then he moved."]);
    }

    #[test]
    fn unknown_languages_use_the_generic_rules() {
        assert_eq!(split_sentences("Dr. Müller kam. Er ging.", "xx"), vec!["Dr.", "Müller kam.", "Er ging."]);
        assert_eq!(split_sentences("Das ist z. B. ein Test. Fertig.", "xx"), vec!["Das ist z. B. ein Test.", "Fertig."]);
    }

    #[test]
    fn terminator_runs_and_cjk() {
        assert_eq!(en("Really?! Yes."), vec!["Really?!", "Yes."]);
        assert_eq!(en(r#"He said "no." Then he left."#), vec![r#"He said "no.""#, "Then he left."]);
        assert_eq!(split_sentences("今日は晴れ。明日は雨。", "ja"), vec!["今日は晴れ。", "明日は雨。"]);
        assert_eq!(split_sentences("آپ کیسے ہیں؟ میں ٹھیک ہوں۔", "ur"), vec!["آپ کیسے ہیں؟", "میں ٹھیک ہوں۔"]);
    }

    #[test]
    fn a_realistic_german_paragraph_gives_seven_sentences() {
        let text = "Die Stadt Freiburg gehört zu den sonnigsten Orten und zieht mehr als 1.000.000 Besucher an. Dr. Müller erklärt, dass sich die Altstadt z. B. wegen der Bächle unterscheidet. Viele Gäste kommen im Sommer, bzw. im Herbst, die Weinberge liegen ca. 20 Minuten entfernt. Prof. Schmidt sagt, die Preise für Hotels usw. steigen um 15 %. Am 3. Oktober sind die Geschäfte geschlossen. Die Stadt investiert 2,5 Mio. Euro. Informationen gibt es im Zentrum (Tel. 0761 / 12345).";
        assert_eq!(de(text).len(), 7);
    }

    #[test]
    fn long_sentences_are_chunked() {
        let long = "word ".repeat(300);
        let parts = en(&long);
        assert!(parts.len() >= 3);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_SENTENCE_CHARS));
    }

    #[test]
    fn preserves_line_breaks_blank_lines_and_indent() {
        let text = "First line. Second.\n\n  Indented.\nLast";
        let doc = split_document(text, "en");
        assert_eq!(doc.lines.len(), 4);
        let translated: Vec<String> = doc.sentences().iter().map(|s| s.to_uppercase()).collect();
        assert_eq!(join_document(&doc, &translated, "de"), "FIRST LINE. SECOND.\n\n  INDENTED.\nLAST");
    }

    #[test]
    fn partial_results_stop_at_what_is_translated() {
        let doc = split_document("One. Two.\n\nThree. Four.", "en");
        let all = doc.sentences();
        assert_eq!(join_document(&doc, &all[..1], "en"), "One.");
        assert_eq!(join_document(&doc, &all[..2], "en"), "One. Two.");
        assert_eq!(join_document(&doc, &all[..3], "en"), "One. Two.\n\nThree.");
        assert_eq!(join_document(&doc, &all, "en"), "One. Two.\n\nThree. Four.");
        assert_eq!(join_document(&doc, &[], "en"), "");
    }

    #[test]
    fn keeps_crlf() {
        let doc = split_document("a\r\nb", "en");
        assert!(doc.crlf);
        assert_eq!(join_document(&doc, &["A".into(), "B".into()], "de"), "A\r\nB");
    }

    #[test]
    fn cjk_targets_join_without_spaces() {
        let doc = split_document("One. Two.", "en");
        assert_eq!(join_document(&doc, &["一。".into(), "二。".into()], "zh"), "一。二。");
    }
}
