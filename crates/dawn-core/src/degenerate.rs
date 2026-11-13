//! Degenerate model output: a translation (or a rewrite) that fell into a loop ("Hallo, wie geht's dir? n n n n n n ...")
//! or ran far past the text it came from. Models do this on misspelled or garbled input. Nothing downstream may show it:
//! the caller retries once with stricter settings and, if that fails too, shows the part before the loop (or the source).

/// Words that make up a legitimate one-letter word in some language (not counted as junk).
fn legit_single(w: &str) -> bool {
    matches!(w, "a" | "i" | "o" | "y" | "e" | "u" | "à" | "é" | "ö" | "я" | "в" | "к" | "с" | "и" | "о" | "у" | "а" | "–" | "-" | "&")
}

fn word_core(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn spaced(text: &str) -> bool {
    text.split_whitespace().count() >= 3
}

/// Cut `words` before word `at` and tidy the end.
fn join_before(words: &[&str], at: usize) -> String {
    words[..at].join(" ").trim().trim_end_matches([',', ';', ':', '-', ' ']).to_string()
}

/// Is `output` (the model's answer for `source`) degenerate? `Some(safe)` is the part of it before the loop starts (possibly
/// empty); `None` means the output is fine.
pub fn find_degeneration(source: &str, output: &str) -> Option<String> {
    let words: Vec<&str> = output.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let cores: Vec<String> = words.iter().map(|w| word_core(w)).collect();

    // 1. One word, again and again: "n n n n", "very very very very".
    let mut i = 0;
    while i < cores.len() {
        let mut j = i + 1;
        while j < cores.len() && cores[j] == cores[i] && !cores[i].is_empty() {
            j += 1;
        }
        if j - i >= 4 {
            // A short repeated word ("n") is junk and goes; a real word stays once.
            let keep = if cores[i].chars().count() <= 2 { i } else { i + 1 };
            return Some(join_before(&words, keep));
        }
        i = j.max(i + 1);
    }

    // 2. A group of two to four words repeated three times in a row.
    for n in 2..=4usize {
        if cores.len() < n * 3 {
            continue;
        }
        for start in 0..=cores.len() - n * 3 {
            let group = &cores[start..start + n];
            if group.iter().all(|c| c.is_empty()) {
                continue;
            }
            if cores[start + n..start + 2 * n] == *group && cores[start + 2 * n..start + 3 * n] == *group {
                return Some(join_before(&words, start + n));
            }
        }
    }

    // 3. A run of one-letter junk ("... dir? n n n"), or mostly one-letter words.
    let junk = |c: &str| c.chars().count() == 1 && c.chars().all(|ch| ch.is_alphabetic()) && !legit_single(c);
    if let Some(start) = (0..cores.len().saturating_sub(2)).find(|&k| junk(&cores[k]) && junk(&cores[k + 1]) && junk(&cores[k + 2])) {
        return Some(join_before(&words, start));
    }
    if cores.len() >= 8 && cores.iter().filter(|c| junk(c)).count() * 10 >= cores.len() * 4 {
        return Some(join_before(&words, cores.iter().position(|c| junk(c)).unwrap_or(0)));
    }

    // 4. A short piece of letters repeated many times in text without spaces (CJK) or inside a word ("ababababab").
    let chars: Vec<char> = output.chars().collect();
    for period in 1..=6usize {
        let mut run = 1usize;
        for k in period..chars.len() {
            if chars[k] == chars[k - period] && chars[k].is_alphabetic() {
                run += 1;
                if run >= period * 5 + period && run / period >= 6 {
                    let start = k + 1 - run + period;
                    return Some(chars[..start].iter().collect::<String>().trim().to_string());
                }
            } else {
                run = 1;
            }
        }
    }

    // 5. Far longer than the text it answers (only where both are written with spaces, so scripts differ little).
    if spaced(source) && spaced(output) {
        let (s, o) = (source.chars().count(), output.chars().count());
        if o > s * 3 + 30 {
            let keep = words.len().min((source.split_whitespace().count() * 2).max(3));
            return Some(join_before(&words, keep));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reported_loop_is_cut_where_it_starts() {
        let bad = format!("Hallo, wie geht's dir? {}", "n ".repeat(40));
        assert_eq!(find_degeneration("hi how are you ? i m softwar devllper", &bad).as_deref(), Some("Hallo, wie geht's dir?"));
    }

    #[test]
    fn a_repeated_word_a_repeated_group_and_one_letter_junk_are_found() {
        assert_eq!(find_degeneration("x y z", "Das ist sehr sehr sehr sehr sehr gut.").as_deref(), Some("Das ist sehr"));
        assert_eq!(find_degeneration("x y z", "Wir gehen nach Hause und dann und dann und dann und dann los").as_deref(), Some("Wir gehen nach Hause und dann"));
        assert_eq!(find_degeneration("x y z", "Ich bin hier t t t sehr").as_deref(), Some("Ich bin hier"));
        assert_eq!(find_degeneration("one two three four", "a b c d e f g h i j k l m").is_some(), true);
    }

    #[test]
    fn a_loop_inside_text_without_spaces_and_inside_a_word_is_found() {
        let cjk = format!("你好{}", "哈".repeat(30));
        assert_eq!(find_degeneration("hello", &cjk).as_deref(), Some("你好哈"));
        assert!(find_degeneration("hello", "Hallo abababababababababab").is_some());
    }

    #[test]
    fn output_far_longer_than_the_input_is_cut() {
        let long = "Dies ist ein sehr langer Satz der nichts mit dem Eingang zu tun hat und einfach immer weiter geht ohne jede Wiederholung aber doch viel zu lang für diese kurze Frage";
        let safe = find_degeneration("hi how are you", long).unwrap();
        assert!(safe.split_whitespace().count() <= 8, "{safe}");
    }

    #[test]
    fn good_translations_are_left_alone() {
        for (src, out) in [
            ("Hello there.", "Hallo!"),
            ("The owls sleep during the day.", "Die Eulen schlafen tagsüber."),
            ("I am a developer, a good one.", "Ich bin ein Entwickler, ein guter."),
            ("Yes, yes, yes.", "Ja, ja, ja."),
            ("It is very very good.", "Es ist sehr, sehr gut."),
            ("Wait...", "Warte ..."),
            ("Dear all, thanks.", "Liebe Alle, danke. -----"),
            ("日本語です", "It is Japanese."),
            ("我喜欢你", "I like you very much indeed, yes."),
            ("", "Ok"),
        ] {
            assert_eq!(find_degeneration(src, out), None, "{out}");
        }
        assert_eq!(find_degeneration("x", ""), None);
    }

    #[test]
    fn nothing_but_a_loop_leaves_nothing() {
        assert_eq!(find_degeneration("a b", "n n n n n n").as_deref(), Some(""));
    }
}
