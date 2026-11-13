//! Tone may only change how someone who is already addressed as "you" is addressed (Sie / du, vous / tu ...). It must never
//! change who is speaking, the meaning or the kind of sentence. Two checks keep the model to that:
//!
//! * [`tone_applies`]: no second person in the text, no tone (the normal polish is used).
//! * [`check_tone_change`]: the tone answer is compared with the fast translation; a changed speaker (first-person words
//!   gone or new), a question mark that appears or disappears, or content words added or removed beyond the address forms
//!   reject it.

/// Words (lowercase) by which a sentence speaks about the speaker. Only languages with a list are checked for person.
fn first_person(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &["i", "me", "my", "mine", "myself", "we", "us", "our", "ours", "ourselves"],
        "de" => &["ich", "mich", "mir", "mein", "meine", "meinen", "meinem", "meiner", "meines", "wir", "uns", "unser", "unsere", "unseren", "unserem", "unserer", "unseres"],
        "fr" => &["je", "j", "me", "m", "moi", "mon", "ma", "mes", "nous", "notre", "nos"],
        "es" => &["yo", "me", "mí", "mi", "mis", "mío", "mía", "conmigo", "nosotros", "nosotras", "nuestro", "nuestra", "nuestros", "nuestras", "nos"],
        "it" => &["io", "mi", "me", "mio", "mia", "miei", "mie", "noi", "nostro", "nostra", "nostri", "nostre", "ci"],
        "pt" => &["eu", "me", "mim", "meu", "minha", "meus", "minhas", "nós", "nosso", "nossa", "nossos", "nossas", "comigo"],
        "nl" => &["ik", "mij", "me", "mijn", "wij", "we", "ons", "onze"],
        "ru" => &["я", "мне", "меня", "мной", "мой", "моя", "моё", "мои", "моего", "моей", "мы", "нам", "нас", "наш", "наша", "наше", "наши"],
        "pl" => &["ja", "mnie", "mi", "mój", "moja", "moje", "moi", "my", "nas", "nam", "nasz", "nasza", "nasze"],
        "tr" => &["ben", "beni", "bana", "benim", "biz", "bizi", "bize", "bizim"],
        _ => &[],
    }
}

/// Words by which a sentence addresses someone as "you" (also the polite forms), in lowercase.
fn address(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &["you", "your", "yours", "yourself", "yourselves", "y"],
        "de" => &["du", "dich", "dir", "dein", "deine", "deinen", "deinem", "deiner", "deines", "sie", "ihnen", "ihr", "ihre", "ihren", "ihrem", "ihrer", "ihres", "euch", "euer", "eure", "euren"],
        "fr" => &["tu", "te", "t", "toi", "ton", "ta", "tes", "vous", "votre", "vos"],
        "es" => &["tú", "tu", "tus", "te", "ti", "contigo", "usted", "ustedes", "su", "sus", "le", "lo", "la", "os", "vosotros", "vos", "vuestro", "vuestra"],
        "it" => &["tu", "te", "ti", "tuo", "tua", "tuoi", "tue", "lei", "la", "le", "suo", "sua", "suoi", "sue", "voi", "vi", "vostro", "vostra"],
        "pt" => &["você", "vocês", "tu", "te", "ti", "teu", "tua", "teus", "tuas", "seu", "sua", "seus", "suas", "o senhor", "senhor", "senhora"],
        "nl" => &["je", "jij", "jou", "jouw", "u", "uw", "jullie"],
        "ru" => &["ты", "тебя", "тебе", "тобой", "твой", "твоя", "твоё", "твои", "вы", "вас", "вам", "вами", "ваш", "ваша", "ваше", "ваши"],
        "pl" => &["ty", "ciebie", "cię", "ci", "ci", "twój", "twoja", "twoje", "pan", "pani", "państwo", "pana", "pani", "panu", "wy", "was", "wam"],
        "tr" => &["sen", "seni", "sana", "senin", "siz", "sizi", "size", "sizin"],
        _ => &[],
    }
}

/// The words of a text in lowercase, letters only (apostrophes split words: "j'ai" is "j", "ai"). Marks, digits and
/// punctuation are dropped.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphabetic() {
            cur.extend(c.to_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn cased_words(text: &str) -> Vec<(String, bool)> {
    // (word, starts a sentence)
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut at_start = true;
    let mut word_start = true;
    for c in text.chars() {
        if c.is_alphabetic() {
            if cur.is_empty() {
                word_start = at_start;
            }
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push((std::mem::take(&mut cur), word_start));
                at_start = false;
            }
            if matches!(c, '.' | '!' | '?' | '\n' | '\u{3002}' | '\u{ff1f}' | '\u{ff01}') {
                at_start = true;
            }
        }
    }
    if !cur.is_empty() {
        out.push((cur, word_start));
    }
    out
}

/// Does `text` (in `lang`) speak to someone as "you"? A capitalized "Sie" inside a German sentence counts (the polite
/// form); the same word at the start of a sentence could be "she" or "they" and does not. Languages without a list: false.
pub fn addresses_someone(lang: &str, text: &str) -> bool {
    let list = address(lang);
    if list.is_empty() {
        return false;
    }
    cased_words(text).into_iter().any(|(w, starts)| {
        let lower = w.to_lowercase();
        match (lang, lower.as_str()) {
            ("de", "sie") => w == "Sie" && !starts,
            ("de", "ihr" | "ihre" | "ihren" | "ihrem" | "ihrer" | "ihres") => w.starts_with('I') && !starts,
            ("de", "euch" | "euer") => true,
            ("es", "le" | "lo" | "la" | "su" | "sus" | "os") | ("it", "la" | "le" | "suo" | "sua" | "suoi" | "sue" | "vi") => false,
            ("it", "lei") => w == "Lei" && !starts,
            ("pt", "seu" | "sua" | "seus" | "suas") => false,
            ("en", "y") => false,
            ("pl", "pan" | "pani" | "pana" | "panu") => w.starts_with('P') && !starts,
            _ => list.contains(&lower.as_str()),
        }
    })
}

/// Should the tone be applied to this sentence? Only when the source speaks to someone as "you", or the fast translation
/// already addresses someone (an imperative such as "Please send me ..." becomes "Senden Sie ..."). Otherwise tone has no
/// effect and the normal polished translation is used.
pub fn tone_applies(from: &str, to: &str, source: &str, draft: &str) -> bool {
    addresses_someone(from, source) || addresses_someone(to, draft)
}

fn is_question(text: &str) -> bool {
    text.contains(['?', '\u{ff1f}', '\u{61f}', '\u{bf}'])
}

fn first_person_count(lang: &str, text: &str) -> usize {
    let list = first_person(lang);
    words(text).iter().filter(|w| list.contains(&w.as_str())).count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToneReject {
    /// The speaker changed: first-person words disappeared or appeared.
    PersonChanged,
    /// A question mark appeared or disappeared (a statement became a question or the reverse).
    SentenceTypeChanged,
    /// Words were added or removed beyond the address forms.
    ContentChanged,
}

fn stem(w: &str) -> String {
    w.chars().take(4).collect()
}

/// The light check for a source whose fast translation cannot be trusted (garbled input): only the kind of sentence.
pub fn check_tone_light(source: &str, out: &str) -> Result<(), ToneReject> {
    if is_question(source) != is_question(out) {
        return Err(ToneReject::SentenceTypeChanged);
    }
    Ok(())
}

/// Compare the answer with the fast translation `draft`. `Ok` when only the address forms changed.
pub fn check_tone_change(lang: &str, draft: &str, out: &str) -> Result<(), ToneReject> {
    if is_question(draft) != is_question(out) {
        return Err(ToneReject::SentenceTypeChanged);
    }
    let (d, o) = (first_person_count(lang, draft), first_person_count(lang, out));
    if (d > 0) != (o > 0) || d.abs_diff(o) > 1 {
        return Err(ToneReject::PersonChanged);
    }
    // Languages written without spaces are not compared word by word.
    if matches!(lang, "ja" | "zh" | "th") {
        return Ok(());
    }
    let skip = address(lang);
    let content = |text: &str| -> Vec<String> { words(text).into_iter().filter(|w| w.chars().count() >= 3 && !skip.contains(&w.as_str())).collect() };
    let (dw, ow) = (content(draft), content(out));
    let novel = ow.iter().filter(|w| !dw.iter().any(|x| x == *w || stem(x) == stem(w))).count();
    let missing = dw.iter().filter(|w| !ow.iter().any(|x| x == *w || stem(x) == stem(w))).count();
    if novel > (ow.len() / 4).max(2) || missing > (dw.len() / 4).max(2) {
        return Err(ToneReject::ContentChanged);
    }
    Ok(())
}

/// Determiners that do not fix the number of the noun after them: "the owls" and "the owl" are both right, so a rewrite that
/// only fixes grammar must not turn one into the other ("three apple" is different: the numeral forces the plural).
const NUMBER_NEUTRAL: [&str; 9] = ["the", "my", "your", "his", "her", "its", "our", "their", "die"];

fn plural_pair(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    long.len() > short.len() && long.starts_with(short) && matches!(&long[short.len()..], "s" | "es" | "n" | "en" | "e" | "er")
}

/// Did the rewrite change the number of a noun after a number-neutral determiner ("The owls is sleeping" became "The owl is
/// sleeping")? The right fix changes the verb.
pub fn noun_number_changed(original: &str, out: &str) -> bool {
    let (o, n) = (words(original), words(out));
    o.iter().enumerate().any(|(i, w)| {
        i > 0
            && NUMBER_NEUTRAL.contains(&o[i - 1].as_str())
            && !n.contains(w)
            && n.iter().enumerate().any(|(j, v)| j > 0 && n[j - 1] == o[i - 1] && v != w && plural_pair(w, v))
    })
}

fn stems_of(words: &[String]) -> Vec<String> {
    words.iter().filter(|w| w.chars().count() >= 3).map(|w| stem(w)).collect()
}

/// A grammar fix is the smallest change that makes the text right: it keeps every noun's number, and adds or removes hardly
/// any content word (words of three letters or more, compared by their first four letters).
pub fn is_minimal_fix(original: &str, out: &str) -> bool {
    if noun_number_changed(original, out) {
        return false;
    }
    let (o, n) = (words(original), words(out));
    let (os, ns) = (stems_of(&o), stems_of(&n));
    let added = ns.iter().filter(|s| !os.contains(s)).count();
    let removed = os.iter().filter(|s| !ns.contains(s)).count();
    let allowed = 2 + o.len() / 4;
    added <= allowed && removed <= allowed
}

/// Does `candidate` say everything `reference` says? Used for sentence alternatives, which may be worded differently but
/// must not lose a part of the meaning ("Ich bin Ali und arbeite als Softwareentwickler" is not "Ich arbeite als
/// Softwareentwickler"). Names that stand in the source (and in `reference`) must still be there.
pub fn content_preserved(lang: &str, source: &str, reference: &str, candidate: &str) -> bool {
    if matches!(lang, "ja" | "zh" | "th") {
        return true;
    }
    let skip = address(lang);
    let content = |text: &str| -> Vec<String> { words(text).into_iter().filter(|w| w.chars().count() >= 3 && !skip.contains(&w.as_str())).collect() };
    let (rw, cw) = (content(reference), content(candidate));
    let matches = |w: &String, list: &[String]| list.iter().any(|x| x == w || stem(x) == stem(w));
    let missing = rw.iter().filter(|w| !matches(w, &cw)).count();
    let novel = cw.iter().filter(|w| !matches(w, &rw)).count();
    if missing > (rw.len() / 5).max(1) || novel > (cw.len() / 3).max(2) {
        return false;
    }
    // Names: capitalized in the middle of the reference and present in the source (ignoring case).
    let source_words = words(source);
    let candidate_words = words(candidate);
    cased_words(reference).into_iter().filter(|(w, starts)| !starts && w.chars().next().is_some_and(char::is_uppercase)).all(|(w, _)| {
        let lower = w.to_lowercase();
        !source_words.contains(&lower) || candidate_words.contains(&lower)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_without_a_you_gets_no_tone() {
        assert!(!tone_applies("en", "de", "my name is ali and i work as software developer", "Mein Name ist Ali und ich arbeite als Softwareentwickler."));
        assert!(!tone_applies("en", "de", "The owls sleep during the day.", "Die Eulen schlafen tagsüber."));
    }

    #[test]
    fn a_text_that_speaks_to_someone_gets_the_tone() {
        assert!(tone_applies("en", "de", "Could you tell me where you live?", "Könnten Sie mir sagen, wo Sie wohnen?"));
        assert!(tone_applies("en", "fr", "Thank you for your help.", "Merci pour votre aide."));
        // An imperative has no "you" in English, but the translation addresses someone.
        assert!(tone_applies("en", "de", "Please send me the invoice.", "Bitte senden Sie mir die Rechnung."));
        // A German "sie" at the start of a sentence is "she" or "they".
        assert!(!addresses_someone("de", "Sie schläft tagsüber."));
        assert!(addresses_someone("de", "Wie geht es Ihnen?"));
        assert!(addresses_someone("de", "Können Sie mir helfen?"));
        assert!(!addresses_someone("xx", "you"));
    }

    #[test]
    fn the_reported_bug_the_speaker_becoming_you_is_rejected() {
        let draft = "Mein Name ist Ali und ich arbeite als Softwareentwickler";
        assert_eq!(check_tone_change("de", draft, "Ihr Name ist Ali, und Sie arbeiten als Softwareentwickler"), Err(ToneReject::PersonChanged));
        assert_eq!(check_tone_change("de", draft, "Du heißt Ali und bist Softwareentwickler, oder?"), Err(ToneReject::SentenceTypeChanged));
        assert_eq!(check_tone_change("de", draft, "Du heißt Ali und bist Softwareentwickler."), Err(ToneReject::PersonChanged));
    }

    #[test]
    fn a_real_tone_change_passes() {
        assert_eq!(check_tone_change("de", "Könnten Sie mir bitte sagen, wo Sie wohnen?", "Kannst du mir bitte sagen, wo du wohnst?"), Ok(()));
        assert_eq!(check_tone_change("fr", "Pourriez-vous me dire où vous habitez ?", "Peux-tu me dire où tu habites ?"), Ok(()));
        assert_eq!(check_tone_change("es", "¿Podría decirme dónde vive?", "¿Podrías decirme dónde vives?"), Ok(()));
        assert_eq!(check_tone_change("ru", "Можете ли вы сказать мне, где вы живёте?", "Можешь сказать мне, где ты живёшь?"), Ok(()));
        assert_eq!(check_tone_change("de", "Bitte senden Sie mir die Rechnung.", "Bitte sende mir die Rechnung."), Ok(()));
    }

    #[test]
    fn added_or_removed_content_is_rejected() {
        let draft = "Könnten Sie mir bitte sagen, wo Sie wohnen?";
        assert_eq!(check_tone_change("de", draft, "Kannst du mir bitte erklären, warum deine Familie in diesem großen Haus wohnt?"), Err(ToneReject::ContentChanged));
        assert_eq!(check_tone_change("de", "Die Firma lädt Sie herzlich zu einer Feier am Samstag in der Stadt ein und freut sich auf Ihren Besuch.", "Die Firma lädt dich ein."), Err(ToneReject::ContentChanged));
    }

    #[test]
    fn a_question_mark_that_appears_or_goes_is_rejected_in_any_script() {
        assert_eq!(check_tone_change("ja", "お元気ですか？", "お元気です。"), Err(ToneReject::SentenceTypeChanged));
        assert_eq!(check_tone_change("ur", "کیا آپ ٹھیک ہیں؟", "آپ ٹھیک ہیں۔"), Err(ToneReject::SentenceTypeChanged));
        assert_eq!(check_tone_change("ja", "お元気ですか？", "元気？"), Ok(()));
    }

    #[test]
    fn a_grammar_fix_may_not_turn_a_plural_noun_into_a_singular() {
        // The reported case: the verb was wrong, not the noun.
        assert!(noun_number_changed("The owls is sleeping in the barn.", "The owl is sleeping in the barn."));
        assert!(!noun_number_changed("The owls is sleeping in the barn.", "The owls are sleeping in the barn."));
        // A numeral forces the number: fixing "three apple" is a real fix.
        assert!(!noun_number_changed("buyed three apple", "bought three apples"));
        assert!(!is_minimal_fix("The owls is sleeping in the barn.", "The owl is sleeping in the barn."));
        assert!(is_minimal_fix("The owls is sleeping in the barn.", "The owls are sleeping in the barn."));
    }

    #[test]
    fn a_grammar_fix_does_not_add_or_drop_content() {
        let src = "Yesterday me and him goes to the shop and buyed three apple.";
        assert!(is_minimal_fix(src, "Yesterday he and I went to the shop and bought three apples."));
        assert!(!is_minimal_fix(src, "Yesterday he and I went to the new supermarket downtown, bought fresh fruit and vegetables there."));
        assert!(!is_minimal_fix("I has a dog and a cat and a bird and a fish.", "I have a dog."));
        assert!(is_minimal_fix("i and me go zoo", "He and I go to the zoo."));
    }

    #[test]
    fn an_alternative_that_drops_part_of_the_sentence_is_not_an_alternative() {
        let (source, polished) = ("im ali and i work as softwar devloper", "Ich bin Ali und arbeite als Softwareentwickler");
        assert!(!content_preserved("de", source, polished, "Ich arbeite als Softwareentwickler"), "\"Ich bin Ali\" is missing");
        assert!(content_preserved("de", source, polished, "Ich heiße Ali und arbeite als Softwareentwickler"));
        assert!(content_preserved("de", source, polished, "Mein Name ist Ali und ich arbeite als Softwareentwickler"));
        assert!(!content_preserved("de", source, polished, "Ich bin Peter und arbeite als Softwareentwickler"), "the name is kept");
        assert!(!content_preserved("de", source, polished, "Ich bin Ali und arbeite als Softwareentwickler in einer großen Firma in Berlin seit vielen Jahren"), "nothing added");
        assert!(content_preserved("ja", "x", "a", "b"), "not compared word by word");
    }

    #[test]
    fn languages_without_pronoun_lists_are_only_checked_for_the_sentence_type() {
        assert_eq!(check_tone_change("hi", "मैं डेवलपर हूँ।", "आप डेवलपर हैं।"), Ok(()));
    }
}
