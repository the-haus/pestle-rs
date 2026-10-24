//! JSON as sites write it into `<script type="application/ld+json">`.
//!
//! Valid JSON takes the fast, strict path. Anything else is read by a
//! forgiving parser that takes what a person or a template meant:
//! `//` and `/* */` comments, `<!--` and `<![CDATA[` wrappers, trailing and
//! missing commas, single-quoted strings, unquoted keys, raw newlines and
//! control characters in strings, unknown escapes, `NaN`/`Infinity`/
//! `undefined` (as null), hex and `+`/`.`-led numbers, Python's
//! `True`/`False`/`None`, a `var x =` in front, several values in one block
//! (`{…}{…}`, `{…},{…}`, `{…};`), a block whose quotes were HTML-escaped
//! (`&quot;`), and a block cut short (open strings and brackets closed). It
//! never fails: what cannot be read is skipped, and each repair is noted.

use serde_json::{Map, Number, Value};

/// The values of one block, and what had to be repaired to read them.
#[derive(Debug, Default)]
pub struct Read {
    pub values: Vec<Value>,
    pub repairs: Vec<&'static str>,
}

/// Every JSON value in `text`, as leniently as it takes.
pub fn read(text: &str) -> Read {
    let trimmed = strip_wrappers(text);
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Read {
            values: vec![v],
            repairs: Vec::new(),
        };
    }
    // Quotes escaped as entities (`{&quot;@type&quot;: …}`): the block decoded, first.
    if trimmed.contains("&quot;") || trimmed.contains("&#34;") {
        let decoded = crate::decode::text_of(trimmed.as_bytes(), true, false).into_owned();
        if decoded != trimmed {
            let mut again = read(&decoded);
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
                    // `var data = {…}`, `window.x = […]`: what follows the `=`.
                    let start = self.i;
                    while self.peek().is_some_and(|b| is_ident(b) || b == b'.' || b == b' ') {
                        self.i += 1;
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
        if out.len() > 1 {
            self.note("several values");
        }
        out
    }

    fn value(&mut self) -> Option<Value> {
        self.skip();
        let b = self.peek()?;
        match b {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' | b'\'' => Some(Value::String(self.string(b))),
            b'-' | b'+' | b'.' | b'0'..=b'9' => Some(self.number()),
            _ if is_ident_start(b) => {
                let word = self.ident();
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
                Some(b) if is_ident_start(b) || b == b'@' || b.is_ascii_digit() => {
                    self.note("unquoted keys");
                    let start = self.i;
                    // `schema:name` is one key; in `b: "x"` the colon separates.
                    while let Some(b) = self.peek() {
                        let compact = b == b':' && self.s.get(self.i + 1).is_some_and(|&n| is_ident(n));
                        if !(is_ident(b) || b == b'@' || b == b'-' || compact) {
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
                // value (`,` `:` `}` `]`, another string) is part of the text.
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
                        None | Some(b',' | b':' | b'}' | b']' | b'"' | b'\'' | b'/')
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
                        .src
                        .get(self.i..self.i + 2)
                        .and_then(|h| u8::from_str_radix(h, 16).ok());
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
        let h = self.src.get(self.i..self.i + 4)?;
        let v = u32::from_str_radix(h, 16).ok()?;
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

    fn number(&mut self) -> Value {
        let start = self.i;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.i += 1;
        }
        if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0X") {
            self.i += 2;
            let hs = self.i;
            while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                self.i += 1;
            }
            self.note("non-JSON numbers");
            let v = i64::from_str_radix(&self.src[hs..self.i], 16).unwrap_or(0);
            let neg = self.s[start] == b'-';
            return Value::Number(Number::from(if neg { -v } else { v }));
        }
        if self.s[self.i..].starts_with(b"Infinity") {
            self.i += 8;
            self.note("non-JSON literal");
            return Value::Null;
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
        let text = self.src[start..self.i].trim_start_matches('+');
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            return v;
        }
        self.note("non-JSON numbers");
        let fixed = text.trim_end_matches('.');
        let fixed = if fixed.starts_with('.') {
            format!("0{fixed}")
        } else {
            fixed.replace("-.", "-0.")
        };
        fixed
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map_or_else(|| Value::String(text.to_owned()), Value::Number)
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
}
