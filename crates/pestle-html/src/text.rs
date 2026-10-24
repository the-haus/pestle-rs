//! Text as a reader sees it (festimap's `textOf`): whitespace collapsed, a
//! space where a block element or a `<br>` breaks the run, nothing of
//! scripts, styles and the other elements whose content is never shown.

/// Elements that do not break a run of text (`<b>Nora</b>s` is one word).
pub fn is_inline(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr"
            | "b"
            | "bdi"
            | "bdo"
            | "cite"
            | "code"
            | "data"
            | "em"
            | "i"
            | "kbd"
            | "mark"
            | "q"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
            | "time"
            | "u"
            | "var"
            | "wbr"
    )
}

/// Elements whose content is never text a reader sees.
pub fn is_never_text(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "noscript" | "template" | "svg" | "iframe" | "object" | "canvas"
    )
}

/// What JavaScript's `\s` matches, and the zero-width space.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200b}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Whether `s` is already collapsed: no whitespace at its ends, none but single spaces inside.
fn collapsed(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() == Some(&b' ') || b.last() == Some(&b' ') {
        return false;
    }
    let mut prev_space = false;
    for &c in b {
        if c >= 0x80 {
            // Non-ASCII whitespace (U+00A0 and the rest): the slow way decides.
            return false;
        }
        let space = c == b' ';
        if (space && prev_space) || matches!(c, b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            return false;
        }
        prev_space = space;
    }
    true
}

/// [`collapse`] of an owned string, which it returns as it is when there is nothing to collapse.
pub fn collapse_owned(s: String) -> String {
    if collapsed(&s) {
        s
    } else {
        collapse(&s)
    }
}

/// Whitespace runs as one space, trimmed.
pub fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if is_space(c) {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
}

/// At most `max` characters.
pub fn cut(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((at, _)) => &s[..at],
        None => s,
    }
}

/// Text for HTML: `&`, `<` and `>` escaped.
pub fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// An attribute value for HTML: `&` and `"` escaped.
pub fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_like_festimap() {
        assert_eq!(collapse("  a \u{a0}\n b\u{200b}c  "), "a b c");
        assert_eq!(collapse("\t\n"), "");
        assert_eq!(cut("été", 2), "ét");
    }
}
