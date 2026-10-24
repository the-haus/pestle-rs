//! Character references (`&amp;`, `&#233;`, `&eacute`) decoded the way
//! htmlparser2 (the `entities` package) does: every named reference of the
//! HTML standard, the legacy ones without `;` too (in an attribute value
//! not when a letter, digit or `=` follows), numeric ones with or without
//! `;`, and the code points HTML replaces (NUL, surrogates, the C1 controls
//! windows-1252 names).

use std::borrow::Cow;

use crate::entities::{ENTITIES, LONGEST};

/// Bytes as text: UTF-8, invalid sequences replaced (valid input, the usual case, borrowed as is).
#[inline]
pub(crate) fn utf8(bytes: &[u8]) -> Cow<'_, str> {
    match simdutf8::basic::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => String::from_utf8_lossy(bytes),
    }
}

/// Bytes known to be valid UTF-8 (cut from a validated chunk at character boundaries) as text.
#[inline]
pub(crate) fn utf8_valid(bytes: &[u8], valid: bool) -> Cow<'_, str> {
    if valid {
        // SAFETY: the caller validated the chunk these bytes were cut from, at character boundaries.
        Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(bytes) })
    } else {
        utf8(bytes)
    }
}

/// A run of text: references decoded when `references`, borrowed when there are none.
pub(crate) fn text_of(raw: &[u8], references: bool, valid: bool) -> Cow<'_, str> {
    if references && memchr::memchr(b'&', raw).is_some() {
        let mut out = String::with_capacity(raw.len());
        decode_into_valid(raw, false, &mut out, valid);
        Cow::Owned(out)
    } else {
        utf8_valid(raw, valid)
    }
}

/// `bytes` with their character references decoded, appended to `out`.
#[cfg(test)]
pub(crate) fn decode_into(bytes: &[u8], attribute: bool, out: &mut String) {
    decode_into_valid(bytes, attribute, out, false)
}

/// [`decode_into`] for bytes known to be valid UTF-8 (`valid`).
pub(crate) fn decode_into_valid(bytes: &[u8], attribute: bool, out: &mut String, valid: bool) {
    let mut at = 0;
    while let Some(amp) = memchr::memchr(b'&', &bytes[at..]) {
        let amp = at + amp;
        out.push_str(&utf8_valid(&bytes[at..amp], valid));
        match reference(&bytes[amp + 1..], attribute) {
            Some((len, text)) => {
                out.push_str(&text);
                at = amp + 1 + len;
            }
            None => {
                out.push('&');
                at = amp + 1;
            }
        }
    }
    out.push_str(&utf8_valid(&bytes[at..], valid));
}

/// `bytes` decoded.
#[cfg(test)]
pub(crate) fn decode(bytes: &[u8], attribute: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    decode_into(bytes, attribute, &mut out);
    out
}

/// How many bytes after an `&` a reference could still need: a streaming
/// reader keeps a trailing `&` this close to the end until more arrives.
pub(crate) const LOOKAHEAD: usize = LONGEST + 1;

/// The reference right after an `&`: how many bytes it takes and its text.
fn reference(after: &[u8], attribute: bool) -> Option<(usize, Cow<'static, str>)> {
    if after.first() == Some(&b'#') {
        return numeric(after);
    }
    // The references nearly every page uses, first.
    for (name, text) in COMMON {
        if after.starts_with(name.as_bytes()) {
            return Some((name.len(), Cow::Borrowed(*text)));
        }
    }
    let max = after.len().min(LONGEST);
    let candidate = &after[..max];
    let name_len = candidate.iter().position(|b| !b.is_ascii_alphanumeric()).unwrap_or(max);
    if name_len == 0 {
        return None;
    }
    // Only names with this first letter can match.
    let first = after[0];
    let lo = ENTITIES.partition_point(|(n, _)| n.as_bytes()[0] < first);
    let hi = ENTITIES.partition_point(|(n, _)| n.as_bytes()[0] <= first);
    let table = &ENTITIES[lo..hi];
    // Longest first: `notin;` before `not`.
    let with_semicolon = candidate.get(name_len) == Some(&b';');
    let mut len = if with_semicolon { name_len + 1 } else { name_len };
    while len > 0 {
        let name = std::str::from_utf8(&after[..len]).ok()?;
        if let Ok(i) = table.binary_search_by(|(n, _)| n.cmp(&name)) {
            let terminated = name.ends_with(';');
            if !terminated && attribute {
                let next = after.get(len).copied();
                if next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'=') {
                    return None;
                }
            }
            return Some((len, Cow::Borrowed(table[i].1)));
        }
        len -= 1;
    }
    None
}

/// Common references, terminated (so no longer name can match instead).
const COMMON: &[(&str, &str)] = &[
    ("quot;", "\""),
    ("amp;", "&"),
    ("nbsp;", "\u{a0}"),
    ("lt;", "<"),
    ("gt;", ">"),
    ("apos;", "'"),
    ("#39;", "'"),
    ("#34;", "\""),
];

fn numeric(after: &[u8]) -> Option<(usize, Cow<'static, str>)> {
    let hex = matches!(after.get(1), Some(b'x' | b'X'));
    let start = if hex { 2 } else { 1 };
    let digits = after[start..]
        .iter()
        .take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() })
        .count();
    if digits == 0 {
        return None;
    }
    let mut code: u32 = 0;
    for &b in &after[start..start + digits] {
        let d = (b as char).to_digit(if hex { 16 } else { 10 }).unwrap_or(0);
        code = code.saturating_mul(if hex { 16 } else { 10 }).saturating_add(d);
    }
    let mut len = start + digits;
    if after.get(len) == Some(&b';') {
        len += 1;
    }
    Some((len, Cow::Owned(replacement(code).to_string())))
}

/// The character a numeric reference stands for, as HTML replaces it.
fn replacement(code: u32) -> char {
    const C1: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D,
        0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
        0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let code = match code {
        0 => 0xFFFD,
        0x80..=0x9F => C1[(code - 0x80) as usize],
        _ => code,
    };
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn references() {
        assert_eq!(decode(b"Tom &amp; Jerry &lt;3", false), "Tom & Jerry <3");
        assert_eq!(decode(b"caf&eacute; caf&#233; caf&#xE9;", false), "café café café");
        assert_eq!(decode(b"&euro;25 &#128;25", false), "€25 €25");
        assert_eq!(decode(b"&notin; &notit; &not", false), "∉ ¬it; ¬");
        assert_eq!(decode(b"&nosuch; & &#; &#x;", false), "&nosuch; & &#; &#x;");
        assert_eq!(decode(b"&#0; &#xD800; &#x110000;", false), "\u{FFFD} \u{FFFD} \u{FFFD}");
        assert_eq!(decode(b"&nbsp", false), "\u{a0}");
    }

    #[test]
    fn legacy_references_in_attributes() {
        assert_eq!(decode(b"?a=1&copy=2", true), "?a=1&copy=2");
        assert_eq!(decode(b"?a=1&copyx", true), "?a=1&copyx");
        assert_eq!(decode(b"?a=1&copy;x", true), "?a=1\u{a9}x");
        assert_eq!(decode(b"&copy 2026", true), "\u{a9} 2026");
        assert_eq!(decode(b"?q=a&amp=b", true), "?q=a&amp=b");
    }
}
