//! What a page's own `<style>` blocks say of hiding and pointing: rules that
//! set `display`, `visibility`, `content-visibility`, `cursor` or
//! `pointer-events`, cascaded by importance,
//! specificity and order, matched as each element opens. Media queries are
//! read for Playwright's default screen (1280×720, light, no print). A
//! block applies to what comes after it, as the page streams; rules whose
//! selector needs what comes later (`:has()`, `:last-child`), siblings
//! (`+`, `~`) or a live page (`:hover`, `::before`) are left out.

use std::collections::HashMap;

use crate::selector::Selector;
use crate::tree::{Element, Tree};

/// The most rules kept, and the most that every element tests (they name no id, class or tag).
const MAX_RULES: usize = 20_000;
const MAX_ANY: usize = 500;
/// The most style sheet text read.
pub(crate) const MAX_CSS: usize = 4 << 20;

/// The viewport media queries are read against.
const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 720.0;

/// A declaration's value, with what ranks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    important: bool,
    specificity: (u16, u16, u16),
    order: u32,
}

struct Rule {
    selector: Selector,
    rank: Rank,
    /// `display`, `visibility`, `content-visibility`, `cursor`, `pointer-events`.
    values: [Option<String>; 5],
}

/// The values the rules give an element, each with its rank.
#[derive(Default)]
pub(crate) struct Cascaded {
    pub display: Option<(String, bool)>,
    pub visibility: Option<(String, bool)>,
    pub content_visibility: Option<(String, bool)>,
    pub cursor: Option<(String, bool)>,
    pub pointer_events: Option<(String, bool)>,
}

const PROPS: [&str; 5] = [
    "display",
    "visibility",
    "content-visibility",
    "cursor",
    "pointer-events",
];

/// The rules read so far, by what their subject names.
#[derive(Default)]
pub(crate) struct Sheet {
    rules: Vec<Rule>,
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    by_tag: HashMap<String, Vec<u32>>,
    any: Vec<u32>,
    order: u32,
}

impl Sheet {
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Reads a `<style>` block (its `media` attribute already checked).
    pub(crate) fn add(&mut self, css: &str) {
        let css = strip_comments(css);
        self.block(&css);
    }

    fn block(&mut self, css: &str) {
        let b = css.as_bytes();
        let mut i = 0;
        while i < b.len() {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b';') {
                i += 1;
            }
            if i >= b.len() {
                break;
            }
            if b[i] == b'@' {
                let (prelude, body, next) = at_rule(css, i);
                i = next;
                let name = prelude[1..]
                    .split(|c: char| c.is_whitespace() || c == '(')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let Some(body) = body else { continue };
                match name.as_str() {
                    "media" if media_matches(prelude[6..].trim()) => self.block(body),
                    "supports" | "layer" | "container" | "document" | "scope" => self.block(body),
                    _ => {}
                }
                continue;
            }
            let Some(open) = find_outside(css, i, b'{') else { break };
            let Some(close) = matching_brace(css, open) else { break };
            let selectors = css[i..open].trim();
            self.rule(selectors, &css[open + 1..close]);
            i = close + 1;
        }
    }

    fn rule(&mut self, selectors: &str, body: &str) {
        let mut decls: [Option<(String, bool)>; 5] = Default::default();
        for decl in split_outside(body, b';') {
            let Some((prop, value)) = decl.split_once(':') else {
                continue;
            };
            let prop = prop.trim().to_ascii_lowercase();
            let Some(k) = PROPS.iter().position(|p| *p == prop) else {
                continue;
            };
            let mut value = value.trim().to_ascii_lowercase();
            let important = value.ends_with("!important") || value.ends_with("! important");
            if important {
                value = value
                    .trim_end_matches("important")
                    .trim_end()
                    .trim_end_matches('!')
                    .trim_end()
                    .to_owned();
            }
            if value.starts_with("var(") || value.is_empty() {
                continue;
            }
            // A later declaration wins, unless an earlier one is important and it is not.
            if decls[k].as_ref().is_none_or(|(_, imp)| important || !*imp) {
                decls[k] = Some((value, important));
            }
        }
        if decls.iter().all(Option::is_none) {
            return;
        }
        for sel in split_outside(selectors, b',') {
            let sel = sel.trim();
            if sel.is_empty() {
                continue;
            }
            let specificity = specificity(sel);
            // Each declaration ranks on its own importance.
            for important in [false, true] {
                let values: [Option<String>; 5] = std::array::from_fn(|k| {
                    decls[k]
                        .as_ref()
                        .filter(|(_, i)| *i == important)
                        .map(|(v, _)| v.clone())
                });
                if values.iter().all(Option::is_none) {
                    continue;
                }
                if self.rules.len() >= MAX_RULES {
                    return;
                }
                let key = subject_key(sel);
                if matches!(key, Key::Any) && self.any.len() >= MAX_ANY {
                    break;
                }
                let Ok(selector) = Selector::new(sel, false) else { break };
                if selector.defers() || selector.needs_siblings() || selector.needs_types() {
                    break;
                }
                self.order += 1;
                let id = self.rules.len() as u32;
                self.rules.push(Rule {
                    selector,
                    rank: Rank {
                        important,
                        specificity,
                        order: self.order,
                    },
                    values,
                });
                match key {
                    Key::Id(k) => self.by_id.entry(k).or_default().push(id),
                    Key::Class(k) => self.by_class.entry(k).or_default().push(id),
                    Key::Tag(k) => self.by_tag.entry(k).or_default().push(id),
                    Key::Any => self.any.push(id),
                }
            }
        }
    }

    /// The values the rules give the innermost open element.
    pub(crate) fn cascade(&self, tree: &Tree, el: &Element) -> Cascaded {
        let mut candidates: Vec<u32> = Vec::new();
        if let Some(id) = el.attr("id") {
            if let Some(r) = self.by_id.get(id) {
                candidates.extend(r);
            }
        }
        if let Some(class) = el.attr("class") {
            for c in class.split_ascii_whitespace() {
                if let Some(r) = self.by_class.get(c) {
                    candidates.extend(r);
                }
            }
        }
        if let Some(r) = self.by_tag.get(el.name()) {
            candidates.extend(r);
        }
        candidates.extend(&self.any);
        let mut best: [Option<(Rank, &str)>; 5] = [None; 5];
        for &r in &candidates {
            let rule = &self.rules[r as usize];
            let outranks = |k: usize| rule.values[k].is_some() && best[k].is_none_or(|(b, _)| rule.rank > b);
            if !(0..5).any(outranks) || !rule.selector.matches_now(tree, None) {
                continue;
            }
            for (slot, value) in best.iter_mut().zip(&rule.values) {
                if let Some(v) = value {
                    if slot.is_none_or(|(b, _)| rule.rank > b) {
                        *slot = Some((rule.rank, v));
                    }
                }
            }
        }
        let take = |b: Option<(Rank, &str)>| b.map(|(r, v)| (v.to_owned(), r.important));
        Cascaded {
            display: take(best[0]),
            visibility: take(best[1]),
            content_visibility: take(best[2]),
            cursor: take(best[3]),
            pointer_events: take(best[4]),
        }
    }
}

/// Whether a `media` attribute or query list holds on the default screen.
pub(crate) fn media_matches(list: &str) -> bool {
    let list = list.trim();
    if list.is_empty() {
        return true;
    }
    split_outside(list, b',').into_iter().any(|q| query(q.trim()))
}

fn query(q: &str) -> bool {
    let lower = q.to_ascii_lowercase();
    let (negate, rest) = match lower.strip_prefix("not ") {
        Some(r) => (true, r.trim().to_owned()),
        None => (false, lower.strip_prefix("only ").unwrap_or(&lower).trim().to_owned()),
    };
    let mut ok = true;
    for part in rest.split(" and ") {
        let part = part.trim();
        let holds = if let Some(feature) = part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
            feature_holds(feature.trim())
        } else {
            matches!(part, "all" | "screen" | "")
        };
        ok &= holds;
    }
    ok != negate
}

fn length(v: &str) -> Option<f64> {
    let v = v.trim();
    let n = |s: &str| s.trim().parse::<f64>().ok();
    if let Some(x) = v.strip_suffix("px") {
        n(x)
    } else if let Some(x) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        n(x).map(|x| x * 16.0)
    } else {
        n(v)
    }
}

fn feature_holds(f: &str) -> bool {
    // Range syntax: `width >= 768px`, `400px <= width <= 700px`.
    for op in [">=", "<=", ">", "<"] {
        if f.contains(op) && !f.contains(':') {
            let parts: Vec<&str> = f
                .split(['<', '>', '='])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let (name, value) = match parts.as_slice() {
                [a, b] if a.ends_with("width") || a.ends_with("height") => (*a, length(b)),
                [a, b] if b.ends_with("width") || b.ends_with("height") => (*b, length(a)),
                _ => return false,
            };
            let Some(v) = value else { return false };
            let size = if name.contains("height") { HEIGHT } else { WIDTH };
            let feature_first = f.trim_start().starts_with(name);
            return match (op, feature_first) {
                (">=", true) | ("<=", false) => size >= v,
                ("<=", true) | (">=", false) => size <= v,
                (">", true) | ("<", false) => size > v,
                _ => size < v,
            };
        }
    }
    let (name, value) = match f.split_once(':') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (f.trim(), None),
    };
    let px = || value.and_then(length);
    match name {
        "min-width" => px().is_some_and(|v| WIDTH >= v),
        "max-width" => px().is_some_and(|v| WIDTH <= v),
        "min-height" => px().is_some_and(|v| HEIGHT >= v),
        "max-height" => px().is_some_and(|v| HEIGHT <= v),
        "width" => px().is_some_and(|v| v == WIDTH),
        "orientation" => value == Some("landscape"),
        "prefers-color-scheme" => value == Some("light"),
        "prefers-reduced-motion" | "prefers-contrast" | "forced-colors" | "inverted-colors" => {
            matches!(value, Some("no-preference" | "none"))
        }
        "hover" | "any-hover" => value.is_none() || value == Some("hover"),
        "pointer" | "any-pointer" => value.is_none() || value == Some("fine"),
        "color" | "monochrome" if value.is_none() => name == "color",
        "min-resolution" | "-webkit-min-device-pixel-ratio" | "min--moz-device-pixel-ratio" => value
            .and_then(|v| {
                v.trim_end_matches(|c: char| c.is_ascii_alphabetic())
                    .parse::<f64>()
                    .ok()
            })
            .is_some_and(|v| v <= 1.0),
        "display-mode" => value == Some("browser"),
        "scripting" => value == Some("enabled"),
        _ => false,
    }
}

enum Key {
    Id(String),
    Class(String),
    Tag(String),
    Any,
}

/// What the selector's last compound names, to file the rule under (`a.b #c:hover` → `#c`).
fn subject_key(sel: &str) -> Key {
    let b = sel.as_bytes();
    // The last compound: after the last combinator outside brackets and parentheses.
    let mut depth = 0i32;
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b' ' | b'>' | b'+' | b'~' | b'\t' | b'\n' if depth == 0 => start = i + 1,
            _ => {}
        }
    }
    let compound = &sel[start..];
    let ident = |s: &str| -> String {
        s.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '\\')
            .collect()
    };
    let mut depth = 0;
    let cb = compound.as_bytes();
    let mut class = None;
    for (i, &c) in cb.iter().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'#' if depth == 0 => {
                let id = ident(&compound[i + 1..]);
                if !id.is_empty() && !id.contains('\\') {
                    return Key::Id(id);
                }
            }
            b'.' if depth == 0 && class.is_none() => {
                let c = ident(&compound[i + 1..]);
                if !c.is_empty() && !c.contains('\\') {
                    class = Some(c);
                }
            }
            _ => {}
        }
    }
    if let Some(c) = class {
        return Key::Class(c);
    }
    let tag = ident(compound).to_ascii_lowercase();
    if !tag.is_empty() && !tag.contains('\\') {
        return Key::Tag(tag);
    }
    Key::Any
}

/// A selector's specificity (ids, classes and attributes and pseudo-classes, types), counting what
/// is inside `:is()`/`:not()` as written.
fn specificity(sel: &str) -> (u16, u16, u16) {
    let (mut a, mut b, mut c) = (0u16, 0u16, 0u16);
    let s = sel.as_bytes();
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'#' => {
                a += 1;
                i += 1;
            }
            b'.' => {
                b += 1;
                i += 1;
            }
            b'[' => {
                b += 1;
                while i < s.len() && s[i] != b']' {
                    i += 1;
                }
            }
            b':' => {
                let double = s.get(i + 1) == Some(&b':');
                let name: String = sel[i + 1 + usize::from(double)..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-')
                    .collect();
                if double {
                    c += 1;
                } else if !matches!(name.as_str(), "is" | "not" | "where" | "matches" | "has") {
                    b += 1;
                }
                i += 1;
            }
            x if x.is_ascii_alphabetic() => {
                let prev = if i == 0 { b' ' } else { s[i - 1] };
                if matches!(prev, b' ' | b'>' | b'+' | b'~' | b'(' | b',') {
                    c += 1;
                }
                while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'-' || s[i] == b'_') {
                    i += 1;
                }
                continue;
            }
            _ => i += 1,
        }
    }
    (a, b, c)
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = match rest[i + 2..].find("*/") {
            Some(j) => &rest[i + 2 + j + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The byte `c` at or after `from`, outside strings and parentheses.
fn find_outside(s: &str, from: usize, c: u8) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0;
    let mut quote = 0u8;
    let mut i = from;
    while i < b.len() {
        let x = b[i];
        if quote != 0 {
            if x == b'\\' {
                i += 1;
            } else if x == quote {
                quote = 0;
            }
        } else if x == b'"' || x == b'\'' {
            quote = x;
        } else if x == b'(' {
            depth += 1;
        } else if x == b')' {
            depth -= 1;
        } else if x == c && depth <= 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The `}` closing the `{` at `open`.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0;
    let mut quote = 0u8;
    let mut i = open;
    while i < b.len() {
        let x = b[i];
        if quote != 0 {
            if x == b'\\' {
                i += 1;
            } else if x == quote {
                quote = 0;
            }
        } else if x == b'"' || x == b'\'' {
            quote = x;
        } else if x == b'{' {
            depth += 1;
        } else if x == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// An at-rule at `at`: its prelude, its block (none for `@import …;`), and where the next rule starts.
fn at_rule(s: &str, at: usize) -> (&str, Option<&str>, usize) {
    // The first `{` or `;` outside strings and parentheses ends the prelude.
    let b = s.as_bytes();
    let mut depth = 0;
    let mut quote = 0u8;
    let mut i = at;
    while i < b.len() {
        let x = b[i];
        if quote != 0 {
            if x == b'\\' {
                i += 1;
            } else if x == quote {
                quote = 0;
            }
        } else {
            match x {
                b'"' | b'\'' => quote = x,
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth <= 0 => return (s[at..i].trim(), None, i + 1),
                b'{' if depth <= 0 => {
                    return match matching_brace(s, i) {
                        Some(c) => (s[at..i].trim(), Some(&s[i + 1..c]), c + 1),
                        None => (s[at..i].trim(), None, s.len()),
                    };
                }
                _ => {}
            }
        }
        i += 1;
    }
    (s[at..].trim(), None, s.len())
}

/// `s` split at `c` outside strings, parentheses and brackets.
fn split_outside(s: &str, c: u8) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote = 0u8;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let x = b[i];
        if quote != 0 {
            if x == b'\\' {
                i += 1;
            } else if x == quote {
                quote = 0;
            }
        } else if x == b'"' || x == b'\'' {
            quote = x;
        } else if x == b'(' || x == b'[' {
            depth += 1;
        } else if x == b')' || x == b']' {
            depth -= 1;
        } else if x == c && depth == 0 {
            out.push(&s[start..i]);
            start = i + 1;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}
