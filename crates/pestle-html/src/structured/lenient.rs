//! JSON as sites write it into `<script type="application/ld+json">`.
//!
//! Valid JSON takes the fast, strict path. Anything else is read by a
//! forgiving parser that takes what a person or a template meant:
//! `//` and `/* */` comments, `<!--` and `<![CDATA[` wrappers, trailing and
//! missing commas, single-quoted strings, unquoted keys, raw newlines and
//! control characters in strings, unknown escapes, `NaN`/`Infinity`/
//! `undefined` (as null), hex and `+`/`.`-led numbers, Python's
//! `True`/`False`/`None`, unquoted values (`https://…`, `3px`, `2024-10-25`),
//! “smart quotes”, a `var x =` or `window["x"] =` in front, several values in
//! one block (`{…}{…}`, `{…},{…}`, `{…};`), a block whose quotes were
//! HTML-escaped (`&quot;`), and a block cut short (open strings and brackets
//! closed). It never fails, and takes time in proportion to the block: what
//! cannot be read is skipped, and each repair is noted.

use serde_json::{Map, Number, Value};

/// The values of one block, and what had to be repaired to read them.
#[derive(Debug, Default)]
pub struct Read {
    pub values: Vec<Value>,
    pub repairs: Vec<&'static str>,
}

/// Every JSON value in `text`, as leniently as it takes.
pub fn read(text: &str) -> Read {
    read_once(text, true)
}

fn read_once(text: &str, entities: bool) -> Read {
    let trimmed = strip_wrappers(text);
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Read {
            values: vec![v],
            repairs: Vec::new(),
        };
    }
    // Quotes escaped as entities (`{&quot;@type&quot;: …}`): the block decoded, once, first. Only when
    // the first quote is one and none is left raw: in `{"a": "say &quot;hi&quot;"}` they are text.
    let opened = trimmed.trim_start_matches(|c: char| c == '{' || c == '[' || c.is_whitespace());
    if entities && !trimmed.contains('"') && (opened.starts_with("&quot;") || opened.starts_with("&#34;")) {
        let decoded = crate::decode::text_of(trimmed.as_bytes(), true, false).into_owned();
        if decoded != trimmed {
            let mut again = read_once(&decoded, false);
            if !again.values.is_empty() {
                again.repairs.push("html entities");
                return again;
            }
        }
    }
    let mut p = Lenient {
        s: trimmed.as_bytes(),
        src: trimmed,
        i: 0,
        repairs: Vec::new(),
        depth: 0,
    };
    let values = p.values();
    Read {
        values,
        repairs: p.repairs,
    }
}

/// Leading and trailing wrappers sites put around a block.
fn strip_wrappers(text: &str) -> &str {
    let mut t = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    loop {
        let before = t;
        for p in ["<!--", "//<![CDATA[", "// <![CDATA[", "/*<![CDATA[*/", "<![CDATA["] {
            if let Some(rest) = t.strip_prefix(p) {
                t = rest.trim_start();
            }
        }
        for s in ["-->", "//]]>", "// ]]>", "/*]]>*/", "]]>"] {
            if let Some(rest) = t.strip_suffix(s) {
                t = rest.trim_end();
            }
        }
        if t == before {
            return t;
        }
    }
}

const MAX_DEPTH: usize = 256;

const LEFT_QUOTE: &[u8] = "\u{201C}".as_bytes();
const RIGHT_QUOTE: &[u8] = "\u{201D}".as_bytes();

struct Lenient<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    repairs: Vec<&'static str>,
    depth: usize,
}

impl Lenient<'_> {
    fn note(&mut self, what: &'static str) {
        if !self.repairs.contains(&what) {
            self.repairs.push(what);
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Whitespace, comments and markup wrappers.
    fn skip(&mut self) {
        loop {
            while let Some(b) = self.peek() {
                if b.is_ascii_whitespace() {
                    self.i += 1;
                } else if b == 0xC2 && self.s.get(self.i + 1) == Some(&0xA0) {
                    self.i += 2;
                } else if b == 0xEF && self.s.get(self.i + 1) == Some(&0xBB) && self.s.get(self.i + 2) == Some(&0xBF) {
                    self.i += 3;
                } else {
                    break;
                }
            }
            let rest = &self.s[self.i..];
            if rest.starts_with(b"//") {
                self.note("comments");
                while self.peek().is_some_and(|b| b != b'\n') {
                    self.i += 1;
                }
            } else if rest.starts_with(b"/*") {
                self.note("comments");
                match memchr::memmem::find(&rest[2..], b"*/") {
                    Some(e) => self.i += e + 4,
                    None => self.i = self.s.len(),
                }
            } else if rest.starts_with(b"<!--") || rest.starts_with(b"-->") {
                self.i += if rest.starts_with(b"<!--") { 4 } else { 3 };
            } else if rest.starts_with(b"<![CDATA[") {
                self.i += 9;
            } else if rest.starts_with(b"]]>") {
                self.i += 3;
            } else {
                return;
            }
        }
    }

    /// Every top-level value, separators and junk between them skipped.
    fn values(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        loop {
            self.skip();
            let Some(b) = self.peek() else { break };
            match b {
                b'{' | b'[' => {
                    if let Some(v) = self.value() {
                        out.push(v);
                    }
                }
                b',' | b';' => {
                    self.note("several values");
                    self.i += 1;
                }
                _ if is_ident_start(b) => {
                    // `var data = {…}`, `window.x = […]`, `window["x"] = {…}`: what follows the `=`.
                    let start = self.i;
                    loop {
                        while self.peek().is_some_and(|b| is_ident(b) || b == b'.' || b == b' ') {
                            self.i += 1;
                        }
                        if !self.subscript() {
                            break;
                        }
                    }
                    if self.peek() == Some(b'=') {
                        self.note("assignment");
                        self.i += 1;
                    } else if self.i == start {
                        self.i += 1;
                    }
                }
                _ => {
                    // A stray value (`"x"`, `42`) at the top is still a value; anything else is junk.
                    let at = self.i;
                    match self.value() {
                        Some(v) if !v.is_null() => out.push(v),
                        _ if self.i == at => self.i += 1,
                        _ => {}
                    }
                }
            }
        }
        // Beside an object or array, a number or string at the top is script around it
        // (`"use strict";`, `x = 5;`), not data.
        if out.iter().any(|v| v.is_object() || v.is_array()) {
            out.retain(|v| v.is_object() || v.is_array());
        }
        if out.len() > 1 {
            self.note("several values");
        }
        out
    }

    /// At `["x"]` or `['x']` right after a name: past it.
    fn subscript(&mut self) -> bool {
        let rest = &self.s[self.i..];
        let Some(&q @ (b'"' | b'\'')) = rest.get(1) else {
            return false;
        };
        if rest[0] != b'[' {
            return false;
        }
        match memchr::memchr(q, &rest[2..]) {
            Some(e) if rest.get(e + 3) == Some(&b']') => {
                self.i += e + 4;
                true
            }
            _ => false,
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.skip();
        let b = self.peek()?;
        match b {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' | b'\'' => Some(Value::String(self.string(b))),
            b'-' | b'+' | b'.' | b'0'..=b'9' => self.number(),
            // The end of the value's container, or of a missing value: left for the container.
            b'}' | b']' | b',' | b':' => None,
            _ if self.s[self.i..].starts_with(LEFT_QUOTE) => Some(Value::String(self.smart_quoted())),
            _ if is_ident_start(b) => {
                let start = self.i;
                let word = self.ident();
                if self.s[self.i..].starts_with(b"://") {
                    // `url: https://x.com/a`: a URL someone forgot to quote, to where it ends.
                    self.note("unquoted string");
                    return Some(Value::String(self.bare(start)));
                }
                Some(match word.as_str() {
                    "true" | "True" => Value::Bool(true),
                    "false" | "False" => Value::Bool(false),
                    "null" | "None" | "undefined" | "NaN" => {
                        if word != "null" {
                            self.note("non-JSON literal");
                        }
                        Value::Null
                    }
                    "Infinity" => {
                        self.note("non-JSON literal");
                        Value::Null
                    }
                    // A bare word as a value: a string someone forgot to quote.
                    _ => {
                        self.note("unquoted string");
                        Value::String(word)
                    }
                })
            }
            _ => {
                self.i += 1;
                None
            }
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.i += 1;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.i = self.s.len();
            return None;
        }
        let mut map = Map::new();
        loop {
            self.skip();
            match self.peek() {
                None => {
                    self.note("truncated");
                    break;
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                    continue;
                }
                Some(b']') => {
                    // A bracket where a brace belongs.
                    self.note("mismatched brackets");
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            let key = match self.peek() {
                Some(q @ (b'"' | b'\'')) => {
                    if q == b'\'' {
                        self.note("single quotes");
                    }
                    self.string(q)
                }
                Some(_) if self.s[self.i..].starts_with(LEFT_QUOTE) => self.smart_quoted(),
                Some(b) if is_ident_start(b) || b == b'@' || b.is_ascii_digit() => {
                    self.note("unquoted keys");
                    let start = self.i;
                    // `schema:name: "x"` is one key; in `b: "x"` and `b:1` the colon separates.
                    while let Some(b) = self.peek() {
                        let compact = b == b':' && self.compact_key_colon();
                        if !(is_ident(b) || matches!(b, b'@' | b'-' | b'.') || compact) {
                            break;
                        }
                        self.i += 1;
                    }
                    self.src[start..self.i].to_owned()
                }
                _ => {
                    self.i += 1;
                    continue;
                }
            };
            self.skip();
            if self.peek() == Some(b':') || self.peek() == Some(b'=') {
                self.i += 1;
            } else {
                self.note("missing colon");
            }
            let value = self.value().unwrap_or(Value::Null);
            // The first of a key wins, as browsers' JSON.parse keeps the last: keep the last too.
            map.insert(key, value);
            self.skip();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.skip();
                    if self.peek() == Some(b'}') {
                        self.note("trailing commas");
                    }
                }
                Some(b'}') | None => {}
                Some(_) => self.note("missing commas"),
            }
        }
        self.depth -= 1;
        Some(Value::Object(map))
    }

    fn array(&mut self) -> Option<Value> {
        self.i += 1;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.i = self.s.len();
            return None;
        }
        let mut items = Vec::new();
        loop {
            self.skip();
            match self.peek() {
                None => {
                    self.note("truncated");
                    break;
                }
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                    continue;
                }
                Some(b'}') => {
                    self.note("mismatched brackets");
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            let at = self.i;
            if let Some(v) = self.value() {
                items.push(v);
            } else if self.i == at {
                self.i += 1;
            }
            self.skip();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.skip();
                    if self.peek() == Some(b']') {
                        self.note("trailing commas");
                    }
                }
                Some(b']') | None => {}
                Some(_) => self.note("missing commas"),
            }
        }
        self.depth -= 1;
        Some(Value::Array(items))
    }

    fn string(&mut self, quote: u8) -> String {
        self.i += 1;
        let mut out = String::new();
        let mut from = self.i;
        loop {
            let Some(b) = self.peek() else {
                self.note("truncated");
                out.push_str(&self.src[from..self.i]);
                return out;
            };
            if b == quote {
                // `"The "Best" Night"`: a quote followed, on the same line, by more than the end of a
                // value (`,` `:` `}` `]` `;` `)`, another string, a comment) is part of the text.
                let mut j = self.i + 1;
                let mut newline = false;
                while let Some(&w) = self.s.get(j) {
                    if !w.is_ascii_whitespace() {
                        break;
                    }
                    newline |= w == b'\n';
                    j += 1;
                }
                let ends = newline
                    || matches!(
                        self.s.get(j),
                        None | Some(b',' | b':' | b'}' | b']' | b'"' | b'\'' | b'/' | b';' | b')')
                    );
                if ends {
                    out.push_str(&self.src[from..self.i]);
                    self.i += 1;
                    return out;
                }
                self.note("unescaped quotes");
                self.i += 1;
                continue;
            }
            if b < 0x20 {
                self.note("control characters");
            }
            if b != b'\\' {
                self.i += 1;
                continue;
            }
            out.push_str(&self.src[from..self.i]);
            self.i += 1;
            let Some(e) = self.peek() else { return out };
            self.i += 1;
            match e {
                b'"' => out.push('"'),
                b'\'' => out.push('\''),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => out.push(self.unicode_escape()),
                b'x' => {
                    let hex = self
                        .s
                        .get(self.i..self.i + 2)
                        .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                        .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
                    match hex {
                        Some(v) => {
                            out.push(v as char);
                            self.i += 2;
                        }
                        None => out.push('x'),
                    }
                }
                b'\n' => {}
                _ => {
                    // An escape JSON does not know: the character itself.
                    self.note("unknown escapes");
                    let c = self.src[self.i - 1..].chars().next().unwrap_or('\u{FFFD}');
                    self.i += c.len_utf8() - 1;
                    out.push(c);
                }
            }
            from = self.i;
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = self.s.get(self.i..self.i + 4)?;
        if !h.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let v = u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
        self.i += 4;
        Some(v)
    }

    fn unicode_escape(&mut self) -> char {
        let Some(hi) = self.hex4() else { return 'u' };
        if (0xD800..0xDC00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
            let save = self.i;
            self.i += 2;
            if let Some(lo) = self.hex4() {
                if (0xDC00..0xE000).contains(&lo) {
                    return char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)).unwrap_or('\u{FFFD}');
                }
            }
            self.i = save;
        }
        char::from_u32(hi).unwrap_or('\u{FFFD}')
    }

    /// A number, a bare token that starts like one (`3px`, `2024-10-25`, `10:30`) as a string,
    /// or nothing when no digit follows (`-`, `.`), left where it stands.
    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let neg = self.s[start] == b'-';
        if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0X") {
            self.i += 2;
            let hs = self.i;
            while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                self.i += 1;
            }
            self.note("non-JSON numbers");
            let digits = &self.src[hs..self.i];
            if digits.is_empty() {
                return Some(Value::Number(0.into()));
            }
            return Some(match i64::from_str_radix(digits, 16) {
                Ok(v) => Value::Number(Number::from(if neg { -v } else { v })),
                // Past i64: as near as a float comes.
                Err(_) => {
                    let v = digits
                        .bytes()
                        .fold(0f64, |n, d| n * 16.0 + f64::from((d as char).to_digit(16).unwrap_or(0)));
                    Number::from_f64(if neg { -v } else { v }).map_or(Value::Null, Value::Number)
                }
            });
        }
        if self.s[self.i..].starts_with(b"Infinity") {
            self.i += 8;
            self.note("non-JSON literal");
            return Some(Value::Null);
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            // A sign belongs to the number only right after an exponent.
            let b = self.s[self.i];
            if matches!(b, b'+' | b'-') && !matches!(self.s[self.i - 1], b'e' | b'E') {
                break;
            }
            self.i += 1;
        }
        if !self.s[start..self.i].iter().any(u8::is_ascii_digit) {
            self.i = start;
            return None;
        }
        let digit_next = self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit);
        match self.peek() {
            Some(b) if b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 => {}
            Some(b'-' | b':' | b'/') if digit_next => {}
            _ => {
                let text = self.src[start..self.i].trim_start_matches('+');
                if let Ok(v) = serde_json::from_str::<Value>(text) {
                    return Some(v);
                }
                self.note("non-JSON numbers");
                let fixed = text.trim_end_matches('.');
                let fixed = if fixed.starts_with('.') {
                    format!("0{fixed}")
                } else {
                    fixed.replace("-.", "-0.")
                };
                return Some(
                    fixed
                        .parse::<f64>()
                        .ok()
                        .and_then(Number::from_f64)
                        .map_or_else(|| Value::String(text.to_owned()), Value::Number),
                );
            }
        }
        self.note("unquoted string");
        Some(Value::String(self.bare(start)))
    }

    /// An unquoted token from `start` to where a value ends: whitespace, a separator, a quote.
    fn bare(&mut self, start: usize) -> String {
        while self
            .peek()
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b',' | b'}' | b']' | b'"' | b'\'' | b';' | b')'))
        {
            self.i += 1;
        }
        self.src[start..self.i].to_owned()
    }

    /// `“text”`, as a word processor or CMS writes quotes: the text between them.
    fn smart_quoted(&mut self) -> String {
        self.note("smart quotes");
        self.i += LEFT_QUOTE.len();
        let from = self.i;
        match memchr::memmem::find(&self.s[from..], RIGHT_QUOTE) {
            Some(e) => {
                self.i = from + e + RIGHT_QUOTE.len();
                self.src[from..from + e].to_owned()
            }
            None => {
                self.note("truncated");
                self.i = self.s.len();
                self.src[from..].to_owned()
            }
        }
    }

    /// At a colon inside an unquoted key: whether it joins the key, as in
    /// `schema:name: "x"`, rather than ending it, as in `a:1` — only when
    /// another colon (or `=`) follows the name after it.
    fn compact_key_colon(&self) -> bool {
        let mut j = self.i + 1;
        if !self.s.get(j).is_some_and(|&b| is_ident(b)) {
            return false;
        }
        while self.s.get(j).is_some_and(|&b| is_ident(b) || b == b'@' || b == b'-') {
            j += 1;
        }
        while self.s.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        // Not `url:https://…`, where the second colon is a URL's.
        match self.s.get(j) {
            Some(b':') => self.s.get(j + 1) != Some(&b'/'),
            Some(b'=') => true,
            _ => false,
        }
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.peek().is_some_and(is_ident) {
            self.i += 1;
        }
        self.src[start..self.i].to_owned()
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_ident(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::read;

    fn one(text: &str) -> serde_json::Value {
        let r = read(text);
        assert_eq!(r.values.len(), 1, "{text:?} → {:?}", r.values);
        r.values.into_iter().next().unwrap()
    }

    #[test]
    fn valid_json_takes_the_strict_path() {
        let r = read(r#" {"@type": "Event", "name": "Nora"} "#);
        assert!(r.repairs.is_empty());
        assert_eq!(r.values, [json!({ "@type": "Event", "name": "Nora" })]);
    }

    #[test]
    fn quotes_inside_text() {
        assert_eq!(
            one(r#"{"name": "The "Best" Night", "x": 1}"#),
            json!({ "name": "The \"Best\" Night", "x": 1 })
        );
        // A quote that ends a value still does: before `,` `}` `]` `:`, another string, or a line break.
        assert_eq!(one("{\"a\": \"x\" \"b\": 1}"), json!({ "a": "x", "b": 1 }));
        assert_eq!(one("{\"a\": \"x\"\n b: 1}"), json!({ "a": "x", "b": 1 }));
    }

    #[test]
    fn what_sites_write() {
        assert_eq!(one("<!--\n{\"a\": 1,}\n-->"), json!({ "a": 1 }));
        assert_eq!(one("//<![CDATA[\n{\"a\": [1, 2,],}\n//]]>"), json!({ "a": [1, 2] }));
        assert_eq!(
            one("{'a': 'it\\'s', b: \"x\" /* c */ // d\n}"),
            json!({ "a": "it's", "b": "x" })
        );
        assert_eq!(one("{\"d\": \"line\nbreak\\.\"}"), json!({ "d": "line\nbreak." }));
        assert_eq!(
            one("{\"a\": NaN, \"b\": undefined, \"c\": True, \"d\": .5, \"e\": +3, \"f\": 0x1F}"),
            json!({ "a": null, "b": null, "c": true, "d": 0.5, "e": 3, "f": 31 })
        );
        assert_eq!(one("{\"a\": 1 \"b\": 2}"), json!({ "a": 1, "b": 2 }));
        assert_eq!(one("var data = {\"a\": 1};"), json!({ "a": 1 }));
        assert_eq!(
            one("{&quot;@type&quot;: &quot;Event&quot;}"),
            json!({ "@type": "Event" })
        );
        assert_eq!(
            one("{\"name\": \"Nora\", \"offers\": [{\"price\": 25"),
            json!({ "name": "Nora", "offers": [{ "price": 25 }] })
        );
        assert_eq!(one("{\"e\": \"\\ud83c\\udfb8\"}"), json!({ "e": "🎸" }));
    }

    #[test]
    fn unquoted_keys_with_no_space_after_the_colon() {
        assert_eq!(
            one("{a:1, b:true, c:null, d:x, e:\"y\", f:-2}"),
            json!({ "a": 1, "b": true, "c": null, "d": "x", "e": "y", "f": -2 })
        );
        assert_eq!(one("{schema:name: \"Nora\"}"), json!({ "schema:name": "Nora" }));
        assert_eq!(
            one("{schema:name:\"Nora\", og:url :1}"),
            json!({ "schema:name": "Nora", "og:url": 1 })
        );
        assert_eq!(one("{a:b:c:1}"), json!({ "a:b:c": 1 }));
    }

    #[test]
    fn several_values_in_one_block() {
        let r = read("{\"a\": 1}\n{\"b\": 2}");
        assert_eq!(r.values, [json!({ "a": 1 }), json!({ "b": 2 })]);
        let r = read("{\"a\": 1},{\"b\": 2};");
        assert_eq!(r.values.len(), 2);
        assert!(r.repairs.contains(&"several values"));
    }

    #[test]
    fn junk_is_skipped_never_fatal() {
        assert!(read("").values.is_empty());
        assert!(read("}}}]]] ::: ''").values.len() <= 1);
        let deep = "[".repeat(10_000);
        assert!(read(&deep).values.len() <= 1);
    }

    #[test]
    fn a_missing_value_leaves_its_container_closed() {
        assert_eq!(one("{\"a\": }"), json!({ "a": null }));
        assert_eq!(
            read("{\"a\": } {\"b\": 1}").values,
            [json!({ "a": null }), json!({ "b": 1 })]
        );
        assert_eq!(one("{\"a\":, \"b\": 1}"), json!({ "a": null, "b": 1 }));
        assert_eq!(one("{\"a\": [1, ], \"b\": 2}"), json!({ "a": [1], "b": 2 }));
        assert_eq!(one("{\"a\": ]"), json!({ "a": null }));
        assert_eq!(one("[1, -, 2]"), json!([1, 2]));
        assert_eq!(one("[., 1]"), json!([1]));
    }

    #[test]
    fn bare_values() {
        assert_eq!(
            one("{url: https://x.com/a?b=1, b:2}"),
            json!({ "url": "https://x.com/a?b=1", "b": 2 })
        );
        assert_eq!(one("{url:https://x.com}"), json!({ "url": "https://x.com" }));
        assert_eq!(
            one("{w: 3px, d: 2024-10-25, t: 10:30, r: 1/2, n: 1e3}"),
            json!({ "w": "3px", "d": "2024-10-25", "t": "10:30", "r": "1/2", "n": 1000.0 })
        );
        assert_eq!(one("{og.title: 1}"), json!({ "og.title": 1 }));
    }

    #[test]
    fn numbers() {
        assert_eq!(
            one("[0xFFFFFFFFFFFFFFFFFF, -0x10, 0x]"),
            json!([4.722366482869645e21, -16, 0])
        );
        assert_eq!(
            one("[-Infinity, +Infinity, 1., -.5, 01, 1.2.3]"),
            json!([null, null, 1.0, -0.5, 1.0, "1.2.3"])
        );
        assert_eq!(one("{\"a\": 5e, \"b\": 1}"), json!({ "a": "5e", "b": 1 }));
    }

    #[test]
    fn escapes() {
        assert_eq!(
            one("{\"a\": \"\\x41\\x+1\\u+041\\u12\", b:1}"),
            json!({ "a": "Ax+1u+041u12", "b": 1 })
        );
        assert_eq!(one("{\"a\": \"\\ud83c\", b:1}"), json!({ "a": "\u{FFFD}", "b": 1 }));
        assert_eq!(one("{\"a\": \"\\é\\q\", b:1}"), json!({ "a": "éq", "b": 1 }));
        assert_eq!(one("{\"a\": \"x\\"), json!({ "a": "x" }));
    }

    #[test]
    fn smart_quotes() {
        assert_eq!(
            one("{\u{201C}name\u{201D}: \u{201C}Nora \"live\"\u{201D}, \u{201C}x\u{201D}: 1}"),
            json!({ "name": "Nora \"live\"", "x": 1 })
        );
        assert_eq!(one("{\u{201C}a\u{201D}: \u{201C}cut"), json!({ "a": "cut" }));
    }

    #[test]
    fn script_around_the_data() {
        assert_eq!(
            read("\"use strict\"; var x = 5; var data = {\"a\": 1};").values,
            [json!({ "a": 1 })]
        );
        assert_eq!(read("window[\"__DATA__\"] = {\"a\": 1}").values, [json!({ "a": 1 })]);
        assert_eq!(read("window['x'].y = [1]").values, [json!([1])]);
        assert_eq!(read("cb({\"a\": 1});").values, [json!({ "a": 1 })]);
        assert_eq!(read("\"just a string\"; junk").values, [json!("just a string")]);
    }

    #[test]
    fn entities() {
        // Text that holds entities beside raw quotes keeps them.
        assert_eq!(
            one("{\"a\": \"say &quot;hi&quot;\",}"),
            json!({ "a": "say &quot;hi&quot;" })
        );
        // Decoded once: a chain of `&amp;` is not followed down, however long.
        let deep = format!("{{&quot;a&quot;: &{}quot;x&quot;}}", "amp;".repeat(200_000));
        let r = read(&deep);
        assert_eq!(r.values.len(), 1);
        assert!(r.values[0].get("a").is_some());
        // Single-quoted text that holds one is not decoded either.
        assert_eq!(one("{'a': '&quot;x', }"), json!({ "a": "&quot;x" }));
    }

    /// A small, fixed random source, so a failure reproduces.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn chance(&mut self, in_n: usize) -> bool {
            self.below(in_n) == 0
        }
    }

    fn text(r: &mut Rng) -> String {
        const PIECES: &[&str] = &[
            "a", "Nora", " ", "'", "\"", "\\", "/", "//", "/*", "*/", "{", "}", "[", "]", ",", ":", "é", "🎸", "\n",
            "\t", "&quot;", "<!--", "-->", "]]>", "\u{201C}", "\u{201D}", "1", "-", "http://", "\u{a0}", "\u{2028}",
        ];
        (0..r.below(6)).map(|_| PIECES[r.below(PIECES.len())]).collect()
    }

    fn key(r: &mut Rng) -> String {
        const KEYS: &[&str] = &[
            "a",
            "name",
            "@type",
            "schema:name",
            "$x",
            "_y",
            "startDate",
            "og.title",
            "url",
        ];
        if r.chance(3) {
            text(r)
        } else {
            KEYS[r.below(KEYS.len())].to_owned()
        }
    }

    fn value(r: &mut Rng, depth: usize) -> serde_json::Value {
        use serde_json::Value;
        match r.below(if depth > 4 { 5 } else { 7 }) {
            0 => Value::Null,
            1 => Value::Bool(r.chance(2)),
            2 => json!(r.next() as i64 >> r.below(64)),
            3 => serde_json::Number::from_f64((r.next() % 100_000) as f64 / 64.0).map_or(Value::Null, Value::Number),
            4 => Value::String(text(r)),
            5 => Value::Array((0..r.below(4)).map(|_| value(r, depth + 1)).collect()),
            _ => Value::Object((0..r.below(4)).map(|_| (key(r), value(r, depth + 1))).collect()),
        }
    }

    /// Between tokens: nothing, space, a line break or a comment.
    fn gap(r: &mut Rng, out: &mut String) {
        match r.below(8) {
            0 => out.push(' '),
            1 => out.push('\n'),
            2 => out.push_str(" /* c */ "),
            3 => out.push_str(" // c\n"),
            _ => {}
        }
    }

    fn plain(s: &str, quote: char) -> bool {
        !s.chars().any(|c| c == quote || c == '\\' || c < ' ' || c == '\u{2028}')
    }

    /// `v` written as sites write it: quotes of either kind or none, Python literals, comments,
    /// commas trailing or left out.
    fn sloppy(r: &mut Rng, v: &serde_json::Value, out: &mut String) {
        use serde_json::Value;
        match v {
            Value::Null if r.chance(3) => out.push_str("None"),
            Value::Bool(b) if r.chance(3) => out.push_str(if *b { "True" } else { "False" }),
            Value::Number(n) if r.chance(3) && n.as_f64().is_some_and(|f| f >= 0.0) => {
                out.push('+');
                out.push_str(&n.to_string());
            }
            Value::String(s) => string(r, s, out),
            Value::Array(items) => {
                out.push('[');
                for (k, item) in items.iter().enumerate() {
                    gap(r, out);
                    sloppy(r, item, out);
                    gap(r, out);
                    if k + 1 < items.len() || r.chance(3) {
                        out.push(',');
                    }
                }
                out.push(']');
            }
            Value::Object(map) => {
                out.push('{');
                let bare: Vec<bool> = map
                    .keys()
                    .map(|key| {
                        key.bytes()
                            .next()
                            .is_some_and(|b| b.is_ascii_alphabetic() || b"@$_".contains(&b))
                            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"@$_:.".contains(&b))
                            && !key.ends_with(':')
                            && !key.contains("::")
                            && r.chance(2)
                    })
                    .collect();
                for (k, (key, item)) in map.iter().enumerate() {
                    gap(r, out);
                    if bare[k] {
                        out.push_str(key);
                    } else {
                        string(r, key, out);
                    }
                    if r.chance(2) {
                        out.push(' ');
                    }
                    out.push(':');
                    if r.chance(2) {
                        out.push(' ');
                    }
                    sloppy(r, item, out);
                    gap(r, out);
                    // A comma left out only before a quoted key, where nothing else can be meant.
                    if k + 1 < map.len() && !bare[k + 1] && r.chance(4) {
                        out.push(' ');
                    } else if k + 1 < map.len() || r.chance(3) {
                        out.push(',');
                    }
                }
                out.push('}');
            }
            _ => out.push_str(&v.to_string()),
        }
    }

    fn string(r: &mut Rng, s: &str, out: &mut String) {
        if plain(s, '\'') && r.chance(3) {
            out.push('\'');
            out.push_str(s);
            out.push('\'');
        } else {
            out.push_str(&serde_json::Value::String(s.to_owned()).to_string());
        }
    }

    #[test]
    fn what_is_written_sloppily_reads_back_exactly() {
        let mut r = Rng(0x5eed_1234_abcd_0001);
        for n in 0..20_000 {
            let v = if r.chance(2) {
                serde_json::Value::Object((0..1 + r.below(4)).map(|_| (key(&mut r), value(&mut r, 1))).collect())
            } else {
                serde_json::Value::Array((0..1 + r.below(4)).map(|_| value(&mut r, 1)).collect())
            };
            let mut s = String::new();
            sloppy(&mut r, &v, &mut s);
            let got = read(&s);
            assert_eq!(got.values, std::slice::from_ref(&v), "case {n}: {s:?}");
        }
    }

    #[test]
    fn never_panics() {
        let mut r = Rng(0x0dd_ba11_cafe_f00d);
        // Every prefix of sloppy text: a block cut short anywhere.
        for _ in 0..300 {
            let v = value(&mut r, 0);
            let mut s = String::new();
            sloppy(&mut r, &v, &mut s);
            for (at, _) in s.char_indices() {
                read(&s[..at]);
            }
        }
        // Soup of the pieces that steer the parser.
        const SOUP: &[&str] = &[
            "{",
            "}",
            "[",
            "]",
            ",",
            ":",
            ";",
            "=",
            "\"",
            "'",
            "\\",
            "\\u",
            "\\ud83c",
            "\\x4",
            "/",
            "//",
            "/*",
            "*/",
            "<!--",
            "-->",
            "<![CDATA[",
            "]]>",
            "0x",
            "-",
            "+",
            ".",
            "e",
            "1",
            "Infinity",
            "NaN",
            "true",
            "a",
            "@",
            "schema:",
            "http://",
            "&quot;",
            "&amp;",
            "\u{201C}",
            "\u{201D}",
            "é",
            "\u{feff}",
            "\u{a0}",
            "\n",
            " ",
            "window[\"x\"]",
            "var x =",
            "(",
            ")",
        ];
        for _ in 0..30_000 {
            let s: String = (0..r.below(40)).map(|_| SOUP[r.below(SOUP.len())]).collect();
            read(&s);
        }
    }

    #[test]
    fn hostile_blocks_stay_linear() {
        // Each of these, quadratic or recursive, would take minutes or overflow the stack.
        for s in [
            "{".repeat(200_000),
            "[".repeat(200_000) + &"]".repeat(200_000),
            "\"a\" ".repeat(200_000),
            format!("{{\"a\": \"{}", "\" x".repeat(200_000)),
            format!("{{{}1}}", "a:".repeat(200_000)),
            "/*".repeat(200_000),
            "{a:b:".repeat(100_000),
            "x = ".repeat(200_000),
            "window[\"".repeat(100_000),
            "\u{201C}".repeat(200_000),
            "1-".repeat(200_000),
        ] {
            read(&s);
        }
    }
}
