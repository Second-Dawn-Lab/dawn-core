//! Word-level changes between a text and its rewrite (the Write tab), grouped so that each change can be accepted on its own.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Piece {
    /// Text that did not change.
    Same { text: String },
    /// A change: what was there (may be empty) and what the rewrite has (may be empty). `id` counts the changes.
    Change { id: usize, old: String, new: String },
}

/// Words and the whitespace between them, each run its own token, so joining the tokens gives the text back.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last_space: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        if let Some(prev) = last_space {
            if prev != space {
                out.push(&text[start..i]);
                start = i;
            }
        }
        last_space = Some(space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Beyond this many tokens on either side the diff would be slow; the whole text becomes one change.
const MAX_TOKENS: usize = 3000;

pub fn diff_words(old: &str, new: &str) -> Vec<Piece> {
    if old == new {
        return if old.is_empty() { vec![] } else { vec![Piece::Same { text: old.to_string() }] };
    }
    let a = tokens(old);
    let b = tokens(new);
    if a.len() > MAX_TOKENS || b.len() > MAX_TOKENS {
        return vec![Piece::Change { id: 0, old: old.to_string(), new: new.to_string() }];
    }
    // Longest common subsequence table.
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    // Walk both token lists; equal tokens are "same", the rest collects into changes.
    let mut pieces: Vec<Piece> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut del, mut ins, mut same) = (String::new(), String::new(), String::new());
    let mut id = 0;
    let push_change = |pieces: &mut Vec<Piece>, del: &mut String, ins: &mut String, id: &mut usize| {
        if !del.is_empty() || !ins.is_empty() {
            pieces.push(Piece::Change { id: *id, old: std::mem::take(del), new: std::mem::take(ins) });
            *id += 1;
        }
    };
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            if !del.is_empty() || !ins.is_empty() {
                push_change(&mut pieces, &mut del, &mut ins, &mut id);
            }
            same.push_str(a[i]);
            i += 1;
            j += 1;
            continue;
        }
        if !same.is_empty() {
            pieces.push(Piece::Same { text: std::mem::take(&mut same) });
        }
        if j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
            del.push_str(a[i]);
            i += 1;
        } else {
            ins.push_str(b[j]);
            j += 1;
        }
    }
    push_change(&mut pieces, &mut del, &mut ins, &mut id);
    if !same.is_empty() {
        pieces.push(Piece::Same { text: same });
    }
    merge_neighbours(pieces)
}

/// Two changes with only a space between them are one change ("b c" -> "x y" reads as one edit), neighbouring `Same`
/// pieces are joined, and the changes are numbered.
fn merge_neighbours(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        let n = out.len();
        match p {
            Piece::Change { old, new, .. } => {
                let glued = n >= 2
                    && matches!(&out[n - 1], Piece::Same { text } if text.trim().is_empty())
                    && matches!(&out[n - 2], Piece::Change { .. });
                if glued {
                    let Some(Piece::Same { text: space }) = out.pop() else { unreachable!() };
                    if let Some(Piece::Change { old: o, new: nw, .. }) = out.last_mut() {
                        o.push_str(&space);
                        o.push_str(&old);
                        nw.push_str(&space);
                        nw.push_str(&new);
                    }
                } else {
                    out.push(Piece::Change { id: 0, old, new });
                }
            }
            Piece::Same { text } => match out.last_mut() {
                Some(Piece::Same { text: last }) => last.push_str(&text),
                _ => out.push(Piece::Same { text }),
            },
        }
    }
    let mut id = 0;
    for p in &mut out {
        if let Piece::Change { id: slot, .. } = p {
            *slot = id;
            id += 1;
        }
    }
    out
}

/// The text with the accepted changes applied (`accepted[id]`); missing entries count as not accepted.
pub fn apply(pieces: &[Piece], accepted: &[bool]) -> String {
    pieces
        .iter()
        .map(|p| match p {
            Piece::Same { text } => text.as_str(),
            Piece::Change { id, old, new } => {
                if accepted.get(*id).copied().unwrap_or(false) {
                    new.as_str()
                } else {
                    old.as_str()
                }
            }
        })
        .collect()
}

pub fn change_count(pieces: &[Piece]) -> usize {
    pieces.iter().filter(|p| matches!(p, Piece::Change { .. })).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(pieces: &[Piece], v: bool) -> Vec<bool> {
        vec![v; change_count(pieces)]
    }

    #[test]
    fn identical_text_has_no_changes() {
        let d = diff_words("Hello world", "Hello world");
        assert_eq!(change_count(&d), 0);
        assert_eq!(apply(&d, &[]), "Hello world");
    }

    #[test]
    fn accepting_everything_gives_the_new_text_and_nothing_the_old_one() {
        let (old, new) = ("Yesterday me and him goes to the shop and buyed three apple.", "Yesterday he and I went to the shop and bought three apples.");
        let d = diff_words(old, new);
        assert!(change_count(&d) >= 3);
        assert_eq!(apply(&d, &all(&d, true)), new);
        assert_eq!(apply(&d, &all(&d, false)), old);
    }

    #[test]
    fn single_changes_can_be_accepted_on_their_own() {
        let d = diff_words("I has a dog and a cat.", "I have a dog and a cat!");
        assert_eq!(change_count(&d), 2);
        assert_eq!(apply(&d, &[true, false]), "I have a dog and a cat.");
        assert_eq!(apply(&d, &[false, true]), "I has a dog and a cat!");
    }

    #[test]
    fn insertions_and_deletions_are_changes() {
        let d = diff_words("one two three", "one three");
        assert_eq!(change_count(&d), 1);
        assert_eq!(apply(&d, &[true]), "one three");
        let d = diff_words("one three", "one two three");
        assert_eq!(apply(&d, &[true]), "one two three");
        assert_eq!(apply(&d, &[false]), "one three");
    }

    #[test]
    fn whole_rewrites_and_empty_sides_round_trip() {
        for (a, b) in [("", "new"), ("old", ""), ("a b c", "x y z"), ("Zeile eins\nZeile zwei", "Zeile eins\nZeile 2"), ("Ein  Test", "Ein Test")] {
            let d = diff_words(a, b);
            assert_eq!(apply(&d, &all(&d, true)), b, "{a:?} -> {b:?}");
            assert_eq!(apply(&d, &all(&d, false)), a, "{a:?} -> {b:?}");
        }
    }

    #[test]
    fn neighbouring_word_changes_are_one_change() {
        let d = diff_words("start me and him end", "start he and I end");
        // "me"->"he", "and" unchanged, "him"->"I": two changes with "and" between them.
        assert_eq!(change_count(&d), 2);
        let d = diff_words("a b c d", "a x y d");
        assert_eq!(change_count(&d), 1);
    }

    #[test]
    fn huge_texts_become_one_change() {
        let big = "wort ".repeat(MAX_TOKENS);
        let d = diff_words(&big, "kurz");
        assert_eq!(change_count(&d), 1);
    }
}
