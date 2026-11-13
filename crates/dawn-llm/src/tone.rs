//! Tone (formal / informal address) per target language.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    /// The model decides from the text.
    #[default]
    Automatic,
    Formal,
    Informal,
}

/// How a language marks the two tones, as the model is told about it. `None` for languages where tone does not change
/// the words in a way the picker can promise (English, Chinese, ...).
fn forms(lang: &str) -> Option<(&'static str, &'static str)> {
    Some(match lang {
        "de" => ("formal address (Sie, Ihnen, Ihr)", "informal address (du, dir, dein)"),
        "fr" => ("formal address (vous)", "informal address (tu, toi)"),
        "es" => ("formal address (usted)", "informal address (tú)"),
        "it" => ("formal address (Lei)", "informal address (tu)"),
        "pt" => ("formal address (o senhor / a senhora)", "informal address (você or tu)"),
        "nl" => ("formal address (u)", "informal address (je, jij)"),
        "pl" => ("formal address (Pan / Pani)", "informal address (ty)"),
        "ru" => ("formal address (Вы)", "informal address (ты)"),
        "uk" => ("formal address (Ви)", "informal address (ти)"),
        "cs" => ("formal address (vy)", "informal address (ty)"),
        "tr" => ("formal address (siz)", "informal address (sen)"),
        "ja" => ("polite, formal Japanese (keigo, です/ます)", "casual, plain Japanese (だ/である-less everyday speech)"),
        "ko" => ("formal polite speech (합쇼체/해요체)", "informal speech (반말)"),
        "hi" => ("formal address (आप)", "informal address (तुम)"),
        "ur" => ("formal address (آپ)", "informal address (تم)"),
        _ => return None,
    })
}

/// What a tone must never do. The sentence is part of every tone prompt, with an example of what stays.
pub const TONE_LIMITS: &str = "The tone only changes how someone who is addressed as \"you\" is addressed. It never changes who is speaking (I stays I, we stays we), never turns a statement into a question or a question into a statement, and never adds or removes information. Example: \"My name is Ali and I work as a developer\" has no \"you\", so it stays a statement in the first person in every tone.";

/// Does the Formal / Informal picker do something for this target language?
pub fn tone_available(lang: &str) -> bool {
    forms(lang).is_some()
}

/// The sentence added to a prompt for `tone`; `None` for Automatic or a language without tones.
pub fn tone_instruction(lang: &str, tone: Tone) -> Option<String> {
    let (formal, informal) = forms(lang)?;
    match tone {
        Tone::Automatic => None,
        Tone::Formal => Some(format!("Tone: address the reader with {formal} and use a formal register. {TONE_LIMITS}")),
        Tone::Informal => Some(format!("Tone: address the reader with {informal} and use a casual, friendly register. {TONE_LIMITS}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_is_offered_for_languages_that_have_it() {
        for l in ["de", "fr", "es", "it", "ru", "ja", "tr", "hi", "ur"] {
            assert!(tone_available(l), "{l}");
        }
        for l in ["en", "zh", "ar", "xx"] {
            assert!(!tone_available(l), "{l}");
        }
    }

    #[test]
    fn german_sie_and_du_are_named_in_the_instruction() {
        assert!(tone_instruction("de", Tone::Formal).unwrap().contains("Sie"));
        assert!(tone_instruction("de", Tone::Informal).unwrap().contains("du"));
        assert_eq!(tone_instruction("de", Tone::Automatic), None);
        assert_eq!(tone_instruction("en", Tone::Formal), None);
    }

    #[test]
    fn tone_serializes_in_lowercase() {
        assert_eq!(serde_json::to_string(&Tone::Informal).unwrap(), "\"informal\"");
        assert_eq!(serde_json::from_str::<Tone>("\"automatic\"").unwrap(), Tone::Automatic);
    }
}
