//! Shortcut descriptions: modifiers + a key, pressed once or twice (`Ctrl+C C`).

use std::fmt;
use std::str::FromStr;

/// The keys a shortcut can use: letters, digits, F1-F12 and Space (Windows virtual-key codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key(pub u32);

impl Key {
    pub fn name(self) -> String {
        match self.0 {
            0x41..=0x5A | 0x30..=0x39 => char::from_u32(self.0).map(String::from).unwrap_or_default(),
            0x70..=0x7B => format!("F{}", self.0 - 0x70 + 1),
            0x20 => "Space".into(),
            other => format!("0x{other:02X}"),
        }
    }
}

impl FromStr for Key {
    type Err = ShortcutError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let upper = s.trim().to_ascii_uppercase();
        let mut chars = upper.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphanumeric() => Ok(Key(c as u32)),
            _ if upper == "SPACE" => Ok(Key(0x20)),
            _ => match upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
                Some(n) if (1..=12).contains(&n) => Ok(Key(0x70 + n - 1)),
                _ => Err(ShortcutError::UnknownKey(s.trim().to_string())),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutError {
    Empty,
    UnknownKey(String),
    NeedsModifier,
    BadRepeat(String),
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the shortcut is empty"),
            Self::UnknownKey(k) => write!(f, "unknown key \"{k}\""),
            Self::NeedsModifier => write!(f, "a shortcut needs at least one of Ctrl, Alt, Shift or Win"),
            Self::BadRepeat(s) => write!(f, "expected one key or the same key twice, got \"{s}\""),
        }
    }
}

impl std::error::Error for ShortcutError {}

/// Modifiers + key, pressed once, or twice in a row (`double`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
    pub double: bool,
}

impl Shortcut {
    /// The default quick-access shortcut: Ctrl+C pressed twice.
    pub fn ctrl_c_c() -> Self {
        Self { ctrl: true, alt: false, shift: false, win: false, key: Key(b'C' as u32), double: true }
    }

    /// The same modifiers and key, ignoring single or double.
    pub fn same_combo(&self, other: &Shortcut) -> bool {
        (self.ctrl, self.alt, self.shift, self.win, self.key) == (other.ctrl, other.alt, other.shift, other.win, other.key)
    }
}

impl FromStr for Shortcut {
    type Err = ShortcutError;

    /// `"Ctrl+C C"` (double), `"Ctrl+Alt+T"` (single). Modifiers: Ctrl, Alt, Shift, Win.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ShortcutError::Empty);
        }
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let last = parts.pop().unwrap_or_default();
        let keys: Vec<&str> = last.split_whitespace().collect();
        let (key, double) = match keys.as_slice() {
            [one] => (one.parse::<Key>()?, false),
            [a, b] if a.eq_ignore_ascii_case(b) => (a.parse::<Key>()?, true),
            _ => return Err(ShortcutError::BadRepeat(last.to_string())),
        };
        let mut shortcut = Shortcut { ctrl: false, alt: false, shift: false, win: false, key, double };
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => shortcut.ctrl = true,
                "alt" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                "win" | "windows" | "meta" | "super" => shortcut.win = true,
                other => return Err(ShortcutError::UnknownKey(other.to_string())),
            }
        }
        if !(shortcut.ctrl || shortcut.alt || shortcut.shift || shortcut.win) {
            return Err(ShortcutError::NeedsModifier);
        }
        Ok(shortcut)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                parts.push(name.to_string());
            }
        }
        let key = self.key.name();
        parts.push(if self.double { format!("{key} {key}") } else { key });
        write!(f, "{}", parts.join("+"))
    }
}

/// Two shortcuts that would react to the same keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub first: String,
    pub second: String,
}

/// Find shortcuts that share their modifiers and key. A single `Ctrl+X` also fires on the first
/// press of a double `Ctrl+X X`, so those conflict too. `list` is `(name, shortcut)`.
pub fn find_conflicts(list: &[(&str, Shortcut)]) -> Vec<Conflict> {
    let mut found = Vec::new();
    for (i, (a_name, a)) in list.iter().enumerate() {
        for (b_name, b) in &list[i + 1..] {
            if a.same_combo(b) {
                found.push(Conflict { first: a_name.to_string(), second: b_name.to_string() });
            }
        }
    }
    found
}

/// A warning if a single-press shortcut would fire on an everyday editing shortcut.
pub fn everyday_shortcut_warning(s: &Shortcut) -> Option<&'static str> {
    let only_ctrl = s.ctrl && !s.alt && !s.shift && !s.win;
    let letter = (s.key.0 as u8) as char;
    (!s.double && only_ctrl && "CXVAZYSF".contains(letter)).then_some("this fires every time you use a standard editing shortcut")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Shortcut {
        s.parse().unwrap()
    }

    #[test]
    fn parses_single_and_double_shortcuts() {
        let d = parse("Ctrl+C C");
        assert_eq!((d.ctrl, d.alt, d.shift, d.win, d.key, d.double), (true, false, false, false, Key(0x43), true));
        let s = parse("ctrl + alt + t");
        assert_eq!((s.ctrl, s.alt, s.key, s.double), (true, true, Key(0x54), false));
        assert_eq!(parse("Win+Shift+F8").key, Key(0x77));
        assert_eq!(parse("Ctrl+Space").key, Key(0x20));
    }

    #[test]
    fn displays_in_a_fixed_order_and_round_trips() {
        for text in ["Ctrl+C C", "Ctrl+Alt+T", "Ctrl+Shift+Win+F8", "Alt+Space", "Shift+Z Z"] {
            assert_eq!(parse(text).to_string(), text);
        }
        assert_eq!(parse("shift+ctrl+c c").to_string(), "Ctrl+Shift+C C");
        assert_eq!(Shortcut::ctrl_c_c().to_string(), "Ctrl+C C");
    }

    #[test]
    fn rejects_bad_shortcuts() {
        assert_eq!("".parse::<Shortcut>(), Err(ShortcutError::Empty));
        assert_eq!("C".parse::<Shortcut>(), Err(ShortcutError::NeedsModifier));
        assert!(matches!("Ctrl+C D".parse::<Shortcut>(), Err(ShortcutError::BadRepeat(_))));
        assert!(matches!("Ctrl+Banana".parse::<Shortcut>(), Err(ShortcutError::BadRepeat(_) | ShortcutError::UnknownKey(_))));
        assert!(matches!("Ctrl+F13".parse::<Shortcut>(), Err(ShortcutError::UnknownKey(_))));
        assert!(matches!("Hyper+C".parse::<Shortcut>(), Err(ShortcutError::UnknownKey(_))));
    }

    #[test]
    fn finds_conflicts_between_shortcuts() {
        let translate = parse("Ctrl+C C");
        let capture = parse("Ctrl+F8");
        assert!(find_conflicts(&[("translate", translate), ("capture", capture)]).is_empty());

        // A single Ctrl+C is the first half of Ctrl+C C.
        let clash = find_conflicts(&[("translate", translate), ("capture", parse("Ctrl+C"))]);
        assert_eq!(clash, vec![Conflict { first: "translate".into(), second: "capture".into() }]);
        // Same combo twice, one single and one double press.
        assert_eq!(find_conflicts(&[("a", parse("Alt+T T")), ("b", parse("Alt+T"))]).len(), 1);
        // Different modifiers do not conflict.
        assert!(find_conflicts(&[("a", parse("Ctrl+T T")), ("b", parse("Alt+T T"))]).is_empty());
    }

    #[test]
    fn warns_about_single_everyday_shortcuts() {
        assert!(everyday_shortcut_warning(&parse("Ctrl+C")).is_some());
        assert!(everyday_shortcut_warning(&parse("Ctrl+C C")).is_none());
        assert!(everyday_shortcut_warning(&parse("Ctrl+Alt+C")).is_none());
        assert!(everyday_shortcut_warning(&parse("Ctrl+F8")).is_none());
    }
}
