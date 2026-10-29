//! Which OCR language to use: the user's capture languages against what the engine offers.

/// Capture languages when the user did not choose any: English plus the active languages.
/// At most three, because every language costs a recognition pass.
pub fn automatic_languages(active: &[String]) -> Vec<String> {
    let mut out = vec!["en".to_string()];
    for code in active {
        if !out.contains(code) && out.len() < 3 {
            out.push(code.clone());
        }
    }
    out
}

/// The capture languages to use: the user's list (up to three), or the automatic one.
pub fn effective_languages(configured: &[String], active: &[String]) -> Vec<String> {
    if configured.is_empty() {
        automatic_languages(active)
    } else {
        configured.iter().take(3).cloned().collect()
    }
}

/// The engine's tag for a language code ("de" -> "de-DE"), if the engine has it.
pub fn tag_for(code: &str, available_tags: &[String]) -> Option<String> {
    let code = code.to_lowercase();
    available_tags.iter().find(|t| t.to_lowercase() == code).or_else(|| available_tags.iter().find(|t| t.to_lowercase().split('-').next() == Some(code.as_str()))).cloned()
}

/// What to do for a set of capture languages.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// `(language code, engine tag)` to try, in the user's order.
    pub run: Vec<(String, String)>,
    /// Languages the engine cannot do on this machine (so the app can say what to install).
    pub missing: Vec<String>,
}

pub fn plan(languages: &[String], available_tags: &[String]) -> Plan {
    let mut run = Vec::new();
    let mut missing = Vec::new();
    for code in languages {
        match tag_for(code, available_tags) {
            Some(tag) if !run.iter().any(|(_, t)| *t == tag) => run.push((code.clone(), tag)),
            Some(_) => {}
            None => missing.push(code.clone()),
        }
    }
    Plan { run, missing }
}

/// Pick the best of several recognition results (one per language): the one with the most letters
/// wins, because text read with the wrong language is mostly punctuation and gaps.
pub fn best_result(results: &[(String, String)]) -> Option<&(String, String)> {
    results.iter().max_by_key(|(_, text)| text.chars().filter(|c| c.is_alphanumeric()).count()).filter(|(_, text)| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn automatic_is_english_plus_active_languages_up_to_three() {
        assert_eq!(automatic_languages(&v(&["en", "de"])), v(&["en", "de"]));
        assert_eq!(automatic_languages(&v(&["de", "fr", "es", "it"])), v(&["en", "de", "fr"]));
        assert_eq!(automatic_languages(&[]), v(&["en"]));
    }

    #[test]
    fn an_explicit_list_wins_and_is_capped() {
        assert_eq!(effective_languages(&v(&["de"]), &v(&["fr"])), v(&["de"]));
        assert_eq!(effective_languages(&v(&["de", "fr", "es", "it"]), &[]), v(&["de", "fr", "es"]));
        assert_eq!(effective_languages(&[], &v(&["de"])), v(&["en", "de"]));
    }

    #[test]
    fn language_codes_match_regional_tags() {
        let tags = v(&["de-DE", "en-GB"]);
        assert_eq!(tag_for("de", &tags).as_deref(), Some("de-DE"));
        assert_eq!(tag_for("EN", &tags).as_deref(), Some("en-GB"));
        assert_eq!(tag_for("fr", &tags), None);
        assert_eq!(tag_for("de-DE", &tags).as_deref(), Some("de-DE"));
    }

    #[test]
    fn the_plan_separates_what_can_run_from_what_is_missing() {
        let p = plan(&v(&["en", "de", "ur", "fr"]), &v(&["de-DE", "en-GB"]));
        assert_eq!(p.run, vec![("en".to_string(), "en-GB".to_string()), ("de".to_string(), "de-DE".to_string())]);
        assert_eq!(p.missing, v(&["ur", "fr"]));
        let none = plan(&v(&["ur"]), &v(&["de-DE"]));
        assert!(none.run.is_empty());
        assert_eq!(none.missing, v(&["ur"]));
    }

    #[test]
    fn the_result_with_the_most_letters_wins() {
        let results = vec![
            ("en".to_string(), "Dq. S0nn3 sch31nt ///".to_string()),
            ("de".to_string(), "Die Sonne scheint heute".to_string()),
        ];
        assert_eq!(best_result(&results).map(|r| r.0.as_str()), Some("de"));
        assert_eq!(best_result(&[("en".to_string(), "  ".to_string())]), None);
        assert_eq!(best_result(&[]), None);
    }
}
