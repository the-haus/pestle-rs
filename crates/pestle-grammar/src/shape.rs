//! A parse tree as a plain value, by its node tags.
//!
//! A pair with tagged pairs beneath it (untagged pairs in between are looked
//! through) is an object of them, by tag; a tag met twice, or named in
//! [`ShapeOptions::lists`], is an array. A pair with none is its text, or a
//! number when its tag is in [`ShapeOptions::numbers`]. So a grammar names
//! what it returns where it matches it:
//!
//! ```text
//! show  = { #date = date ~ ("·" ~ #doors = time)? }
//! date  = { #day = num ~ #month = month ~ #year = num? }
//! ```
//!
//! gives `{ "date": { "day": "14", "month": "nov", "year": "2026" }, "doors": "20:00" }`.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::number::lenient_number;
use crate::tree::{Tree, NO_TAG};

/// How [`crate::Grammar::shape`] builds its value.
#[derive(Clone, Debug, Default)]
pub struct ShapeOptions {
    /// Tags whose value is always an array, even of one.
    pub lists: HashSet<String>,
    /// Tags whose text is read as a number ([`lenient_number`]); null when it is none.
    pub numbers: HashSet<String>,
    /// Key every pair by its rule instead (no tags needed); silent rules make no pairs.
    pub by_rule: bool,
    /// Trim whitespace off every text.
    pub trim: bool,
}

/// `names` keys every pair by its rule ([`ShapeOptions::by_rule`]); else tags key them.
pub(crate) fn shape(tree: &Tree, input: &str, tags: &[String], names: &[String], options: &ShapeOptions) -> Value {
    let names = options.by_rule.then_some(names);
    let cx = Shaper {
        tree,
        input,
        tags,
        options,
        names,
    };
    let roots: Vec<usize> = tree.roots().collect();
    if roots.len() == 1 {
        return cx.node(roots[0]);
    }
    // Several top-level pairs (a silent start rule), or none: shaped as one pair holding them.
    let mut fields = Map::new();
    for i in roots {
        cx.collect(i, &mut fields);
    }
    if fields.is_empty() {
        let start = tree.nodes.first().map_or(0, |n| n.start as usize);
        cx.text(&input[start.min(tree.end)..tree.end])
    } else {
        Value::Object(fields)
    }
}

struct Shaper<'a> {
    tree: &'a Tree,
    input: &'a str,
    tags: &'a [String],
    options: &'a ShapeOptions,
    /// Keyed by rule (these names) instead of by tag.
    names: Option<&'a [String]>,
}

impl Shaper<'_> {
    fn key(&self, i: usize) -> Option<&str> {
        let n = &self.tree.nodes[i];
        match self.names {
            Some(names) => Some(names[n.rule as usize].as_str()),
            None if n.tag == NO_TAG => None,
            None => Some(self.tags[n.tag as usize].as_str()),
        }
    }

    fn text(&self, s: &str) -> Value {
        Value::String(if self.options.trim { s.trim() } else { s }.to_owned())
    }

    fn node(&self, i: usize) -> Value {
        let mut fields = Map::new();
        for c in self.tree.children(i) {
            self.collect(c, &mut fields);
        }
        if !fields.is_empty() {
            return Value::Object(fields);
        }
        let text = self.tree.text(i, self.input);
        match self.key(i) {
            Some(k) if self.options.numbers.contains(k) => lenient_number(text)
                .and_then(serde_json::Number::from_f64)
                .map_or(Value::Null, Value::Number),
            _ => self.text(text),
        }
    }

    /// A keyed pair into `fields`; an unkeyed one's keyed pairs, looked through.
    fn collect(&self, i: usize, fields: &mut Map<String, Value>) {
        let Some(key) = self.key(i) else {
            for c in self.tree.children(i) {
                self.collect(c, fields);
            }
            return;
        };
        let value = self.node(i);
        // A node's own value is never an array, so an array under a key is one made here.
        match fields.get_mut(key) {
            Some(Value::Array(items)) => items.push(value),
            Some(old) => {
                let first = old.take();
                *old = Value::Array(vec![first, value]);
            }
            None if self.options.lists.contains(key) => {
                fields.insert(key.to_owned(), Value::Array(vec![value]));
            }
            None => {
                fields.insert(key.to_owned(), value);
            }
        }
    }
}

/// [`shape`], written straight to JSON text: no value tree built (what a
/// JavaScript caller turns into objects fastest, with `JSON.parse`).
pub(crate) fn shape_json(
    tree: &Tree,
    input: &str,
    tags: &[String],
    names: &[String],
    options: &ShapeOptions,
    out: &mut String,
) {
    let names = options.by_rule.then_some(names);
    let cx = Shaper {
        tree,
        input,
        tags,
        options,
        names,
    };
    let mut scratch = Scratch::new();
    let mut roots = tree.roots();
    if let (Some(only), None) = (roots.next(), roots.next()) {
        cx.write_node(only, out, &mut scratch);
        return;
    }
    for i in tree.roots() {
        cx.gather(i, &mut scratch);
    }
    if scratch.is_empty() {
        let start = tree.nodes.first().map_or(0, |n| n.start as usize);
        cx.write_text(&input[start.min(tree.end)..tree.end], out);
    } else {
        cx.write_object(0, out, &mut scratch);
    }
}

/// Keyed pairs being written, on the stack for small values.
type Scratch = smallvec::SmallVec<[(u32, u32); 16]>;

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    let bytes = s.as_bytes();
    let mut from = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b >= 0x20 && b != b'"' && b != b'\\' {
            continue;
        }
        out.push_str(&s[from..i]);
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            _ => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push_str("\\u00");
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
        from = i + 1;
    }
    out.push_str(&s[from..]);
    out.push('"');
}

impl Shaper<'_> {
    /// A pair's key id: its tag (or rule, keyed by rule); `u32::MAX` for none.
    #[inline]
    fn key_id(&self, i: usize) -> u32 {
        let n = &self.tree.nodes[i];
        match self.names {
            Some(_) => n.rule,
            None => n.tag,
        }
    }

    #[inline]
    fn key_str(&self, i: usize) -> &str {
        let n = &self.tree.nodes[i];
        match self.names {
            Some(names) => names[n.rule as usize].as_str(),
            None => self.tags[n.tag as usize].as_str(),
        }
    }

    /// Keyed pairs under `i` (unkeyed ones looked through), appended as (key, pair).
    fn gather(&self, i: usize, scratch: &mut Scratch) {
        let k = self.key_id(i);
        if k != NO_TAG {
            scratch.push((k, i as u32));
            return;
        }
        for c in self.tree.children(i) {
            self.gather(c, scratch);
        }
    }

    fn write_text(&self, s: &str, out: &mut String) {
        write_str(if self.options.trim { s.trim() } else { s }, out);
    }

    /// The pairs gathered from `scratch[from..]`, as an object grouped by key in first-seen order.
    fn write_object(&self, from: usize, out: &mut String, scratch: &mut Scratch) {
        let end = scratch.len();
        out.push('{');
        let mut first = true;
        for a in from..end {
            let (k, node) = scratch[a];
            if scratch[from..a].iter().any(|&(x, _)| x == k) {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            let key = self.key_str(node as usize);
            write_str(key, out);
            out.push(':');
            let count = scratch[a..end].iter().filter(|&&(x, _)| x == k).count();
            let list = count > 1 || (!self.options.lists.is_empty() && self.options.lists.contains(key));
            if list {
                out.push('[');
                let mut m = 0;
                for b in a..end {
                    let (x, n) = scratch[b];
                    if x == k {
                        if m > 0 {
                            out.push(',');
                        }
                        m += 1;
                        self.write_node(n as usize, out, scratch);
                    }
                }
                out.push(']');
            } else {
                self.write_node(node as usize, out, scratch);
            }
        }
        out.push('}');
        scratch.truncate(from);
    }

    fn write_node(&self, i: usize, out: &mut String, scratch: &mut Scratch) {
        let from = scratch.len();
        for c in self.tree.children(i) {
            self.gather(c, scratch);
        }
        if scratch.len() > from {
            return self.write_object(from, out, scratch);
        }
        let text = self.tree.text(i, self.input);
        if !self.options.numbers.is_empty()
            && self.key_id(i) != NO_TAG
            && self.options.numbers.contains(self.key_str(i))
        {
            match lenient_number(text).and_then(serde_json::Number::from_f64) {
                Some(n) => out.push_str(&n.to_string()),
                None => out.push_str("null"),
            }
            return;
        }
        self.write_text(text, out);
    }
}
