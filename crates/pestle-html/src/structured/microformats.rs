//! Microformats: microformats2 (http://microformats.org/wiki/microformats2-parsing)
//! and the classic formats it reads back-compatibly (hCalendar's `vevent`,
//! hCard's `vcard`, `adr`, `geo`, `hentry`, `hfeed`, `hnews`, `hproduct`,
//! `hrecipe`, `hresume`, `hreview`, `hreview-aggregate`).
//!
//! The document streams by; under a microformat's root element (and only
//! there) a small tree is kept, since the parsing rules look at children
//! (`:only-child`, implied properties) and copy inner HTML (`e-*`). `rel`
//! links are collected from the whole page. Output: the microformats2 JSON
//! (`{ items, rels, rel-urls }`), and items for the common list.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Map, Value};
use url::Url;

use super::graph::{Format, Graph, Node, Val};
use crate::tree::Tree;

// ── A small tree ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct DNode {
    /// Empty for a text node.
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<usize>,
}

#[derive(Debug, Default)]
struct Dom {
    nodes: Vec<DNode>,
}

impl Dom {
    fn attr(&self, n: usize, a: &str) -> Option<&str> {
        self.nodes[n]
            .attrs
            .iter()
            .find(|(k, _)| k == a)
            .map(|(_, v)| v.as_str())
    }

    fn classes(&self, n: usize) -> Vec<&str> {
        self.attr(n, "class")
            .map(|c| c.split_ascii_whitespace().collect())
            .unwrap_or_default()
    }

    fn has_class(&self, n: usize, c: &str) -> bool {
        self.classes(n).contains(&c)
    }

    fn elements(&self, n: usize) -> impl Iterator<Item = usize> + '_ {
        // A template's content is not part of the document.
        self.nodes[n]
            .children
            .iter()
            .copied()
            .filter(move |&c| !self.nodes[c].name.is_empty() && self.nodes[c].name != "template")
    }

    /// `textContent`, with nested `<img>` read as their `alt` (else ` src `) when `images`,
    /// `<script>`/`<style>` dropped.
    fn text(&self, n: usize, images: Option<&str>, out: &mut String) {
        let node = &self.nodes[n];
        if node.name.is_empty() {
            out.push_str(&node.text);
            return;
        }
        match node.name.as_str() {
            "script" | "style" | "template" => return,
            "img" if images.is_some() || images.is_none() => {
                if let Some(alt) = self.attr(n, "alt") {
                    out.push_str(alt);
                } else if let Some(src) = self.attr(n, "src") {
                    out.push(' ');
                    out.push_str(&resolve(src, images));
                    out.push(' ');
                }
                return;
            }
            _ => {}
        }
        for &c in &node.children {
            self.text(c, images, out);
        }
    }

    fn text_of(&self, n: usize, base: Option<&str>) -> String {
        let mut s = String::new();
        self.text(n, base, &mut s);
        trim(&s)
    }

    /// `textContent` with nested `<img>` read as their `alt` only (an implied name's text).
    fn alt_text(&self, n: usize, out: &mut String) {
        let node = &self.nodes[n];
        if node.name.is_empty() {
            out.push_str(&node.text);
            return;
        }
        match node.name.as_str() {
            "script" | "style" | "template" => return,
            "img" => {
                if let Some(alt) = self.attr(n, "alt") {
                    out.push_str(alt);
                }
                return;
            }
            _ => {}
        }
        for &c in &node.children {
            self.alt_text(c, out);
        }
    }

    /// Plain `textContent` (scripts and styles left out).
    fn raw_text(&self, n: usize, out: &mut String) {
        let node = &self.nodes[n];
        if node.name.is_empty() {
            out.push_str(&node.text);
            return;
        }
        if matches!(node.name.as_str(), "script" | "style" | "template") {
            return;
        }
        for &c in &node.children {
            self.raw_text(c, out);
        }
    }

    /// Inner HTML, serialized as browsers (and parse5) do.
    /// Relative `href`s and `src`s are resolved against `base`.
    fn inner_html(&self, n: usize, base: Option<&str>, out: &mut String) {
        let raw = matches!(
            self.nodes[n].name.as_str(),
            "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext"
        );
        for &c in &self.nodes[n].children {
            let node = &self.nodes[c];
            if node.name.is_empty() {
                if raw {
                    out.push_str(&node.text);
                } else {
                    escape(&node.text, false, out);
                }
                continue;
            }
            out.push('<');
            out.push_str(&node.name);
            for (k, v) in &node.attrs {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                match base {
                    Some(_) if matches!(k.as_str(), "href" | "src") => escape(&resolve(v, base), true, out),
                    _ => escape(v, true, out),
                }
                out.push('"');
            }
            out.push('>');
            if crate::tree::is_void(&node.name) {
                continue;
            }
            self.inner_html(c, base, out);
            out.push_str("</");
            out.push_str(&node.name);
            out.push('>');
        }
    }
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Whitespace off both ends (HTML's, as microformats parsers trim).
/// `p-name` → (`p`, `name`).
fn split_prop(p: &str) -> (&'static str, &str) {
    for prefix in ["p", "u", "dt", "e"] {
        if let Some(name) = p.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')) {
            return (prefix, name);
        }
    }
    ("p", p)
}

fn trim(s: &str) -> String {
    s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
        .to_owned()
}

/// A URL against the base, resolved as RFC 3986 has it (no normalization, as the microformats2
/// tests expect: `""` against `http://example.test` is `http://example.test`).
fn resolve(href: &str, base: Option<&str>) -> String {
    let href = href.trim();
    if is_absolute(href) {
        return href.to_owned();
    }
    match base {
        Some(b) => super::iri::resolve(b, href),
        None => href.to_owned(),
    }
}

fn is_absolute(href: &str) -> bool {
    match href.find(':') {
        Some(i) if i > 0 => {
            let scheme = &href[..i];
            scheme.as_bytes()[0].is_ascii_alphabetic()
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
                && Url::parse(href).is_ok()
        }
        _ => false,
    }
}

// ── Class names ──────────────────────────────────────────────────────────

/// `h-x`, `p-x`…: a prefix, then (vendor-)lowercase words.
fn mf2_name(class: &str, prefix: &str) -> Option<String> {
    let rest = class.strip_prefix(prefix)?;
    // `([a-z0-9]+-)?[a-z]+(-[a-z]+)*`
    let word = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase());
    let parts: Vec<&str> = rest.split('-').collect();
    let ok = parts.iter().all(|p| word(p))
        || (parts.len() > 1
            && !parts[0].is_empty()
            && parts[0].bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && parts[1..].iter().all(|p| word(p)));
    ok.then(|| rest.to_owned())
}

/// The classic formats: root class → (type, property class → property names).
fn backcompat_root(class: &str) -> Option<&'static str> {
    Some(match class {
        "adr" => "h-adr",
        "geo" => "h-geo",
        "hentry" => "h-entry",
        "hfeed" => "h-feed",
        "hnews" => "h-news",
        "hproduct" => "h-product",
        "hrecipe" => "h-recipe",
        "hresume" => "h-resume",
        "hreview" => "h-review",
        "hreview-aggregate" => "h-review-aggregate",
        "vcard" => "h-card",
        "vevent" => "h-event",
        _ => return None,
    })
}

/// A classic property class as microformats2 property classes, for a classic type.
fn backcompat_props(kind: &str, class: &str) -> &'static [&'static str] {
    match (kind, class) {
        ("h-adr" | "h-card", "post-office-box") => &["p-post-office-box"],
        ("h-adr" | "h-card", "extended-address") => &["p-extended-address"],
        ("h-adr" | "h-card", "street-address") => &["p-street-address"],
        ("h-adr" | "h-card", "locality") => &["p-locality"],
        ("h-adr" | "h-card", "region") => &["p-region"],
        ("h-adr" | "h-card", "postal-code") => &["p-postal-code"],
        ("h-adr" | "h-card", "country-name") => &["p-country-name"],
        ("h-geo" | "h-card" | "h-entry", "latitude") => &["p-latitude"],
        ("h-geo" | "h-card" | "h-entry", "longitude") => &["p-longitude"],
        ("h-card", "fn") => &["p-name"],
        ("h-card", "honorific-prefix") => &["p-honorific-prefix"],
        ("h-card", "given-name") => &["p-given-name"],
        ("h-card", "additional-name") => &["p-additional-name"],
        ("h-card", "family-name") => &["p-family-name"],
        ("h-card", "honorific-suffix") => &["p-honorific-suffix"],
        ("h-card", "nickname") => &["p-nickname"],
        ("h-card", "email") => &["u-email"],
        ("h-card", "logo") => &["u-logo"],
        ("h-card" | "h-product" | "h-recipe", "photo") => &["u-photo"],
        ("h-card" | "h-event" | "h-product" | "h-feed" | "h-review" | "h-review-aggregate", "url") => &["u-url"],
        ("h-card" | "h-event", "uid") => &["u-uid"],
        ("h-card" | "h-event" | "h-entry" | "h-product" | "h-recipe", "category") => &["p-category"],
        ("h-card", "adr") => &["p-adr"],
        ("h-card", "label") => &["p-label"],
        ("h-card" | "h-entry" | "h-news", "geo") => &["p-geo"],
        ("h-card", "tel") => &["p-tel"],
        ("h-card", "note") => &["p-note"],
        ("h-card", "bday") => &["dt-bday"],
        ("h-card", "key") => &["p-key"],
        ("h-card", "mailer") => &["p-mailer"],
        ("h-card", "org") => &["p-org"],
        ("h-card", "organization-name") => &["p-organization-name"],
        ("h-card", "organization-unit") => &["p-organization-unit"],
        ("h-card", "title") => &["p-job-title"],
        ("h-card", "role") => &["p-role"],
        ("h-card", "tz") => &["p-tz"],
        ("h-card", "rev") => &["p-rev"],
        ("h-card", "sort-string") => &["p-sort-string"],
        ("h-card", "sound") => &["u-sound"],
        ("h-card", "class") => &["p-class"],
        ("h-card", "agent") => &["p-agent"],
        ("h-item", "fn") => &["p-name"],
        ("h-item", "url") => &["u-url"],
        ("h-item", "photo") => &["u-photo"],
        ("h-event", "summary") => &["p-name"],
        ("h-event", "dtstart") => &["dt-start"],
        ("h-event", "dtend") => &["dt-end"],
        ("h-event" | "h-recipe", "duration") => &["dt-duration"],
        ("h-event" | "h-product", "description") => &["p-description"],
        ("h-event", "location") => &["p-location"],
        ("h-event", "attendee") => &["p-attendee"],
        ("h-event", "contact") => &["p-contact"],
        ("h-event", "organizer") => &["p-organizer"],
        ("h-entry", "entry-title") => &["p-name"],
        ("h-entry", "entry-summary") => &["p-summary"],
        ("h-entry", "entry-content") => &["e-content"],
        ("h-entry", "published") => &["dt-published"],
        ("h-entry", "updated") => &["dt-updated"],
        ("h-entry" | "h-feed" | "h-recipe", "author") => &["p-author"],
        ("h-feed", "category") => &["p-category"],
        ("h-feed", "photo") => &["u-photo"],
        ("h-news", "entry") => &["p-entry"],
        ("h-news", "source-org") => &["p-source-org"],
        ("h-news", "dateline") => &["p-dateline"],
        ("h-news", "post-office-box") => &["p-post-office-box"],
        ("h-product", "fn") => &["p-name"],
        ("h-product", "brand") => &["p-brand"],
        ("h-product", "identifier") => &["u-identifier"],
        ("h-product", "review") => &["p-review"],
        ("h-product", "price") => &["p-price"],
        ("h-recipe", "fn") => &["p-name"],
        ("h-recipe", "ingredient") => &["p-ingredient"],
        ("h-recipe", "yield") => &["p-yield"],
        ("h-recipe", "instructions") => &["e-instructions"],
        ("h-recipe", "summary") => &["p-summary"],
        ("h-recipe", "nutrition") => &["p-nutrition"],
        ("h-resume", "summary") => &["p-summary"],
        ("h-resume", "contact") => &["p-contact"],
        ("h-resume", "education") => &["p-education"],
        ("h-resume", "experience") => &["p-experience"],
        ("h-resume", "skill") => &["p-skill"],
        ("h-resume", "affiliation") => &["p-affiliation"],
        ("h-review" | "h-review-aggregate", "summary") => &["p-name"],
        ("h-review" | "h-review-aggregate", "item") => &["p-item"],
        ("h-review" | "h-review-aggregate", "rating") => &["p-rating"],
        ("h-review" | "h-review-aggregate", "best") => &["p-best"],
        ("h-review" | "h-review-aggregate", "worst") => &["p-worst"],
        ("h-review", "description") => &["e-content"],
        ("h-review", "reviewer") => &["p-author"],
        ("h-review", "dtreviewed") => &["dt-published"],
        ("h-review-aggregate", "average") => &["p-average"],
        ("h-review-aggregate", "fn") => &["p-name"],
        ("h-review-aggregate", "count") => &["p-count"],
        ("h-review-aggregate", "votes") => &["p-votes"],
        _ => &[],
    }
}

/// The classic `rel` values as properties (`rel="tag"` a category, `rel="bookmark"` the url).
fn backcompat_rel(kind: &str, rel: &str) -> Option<&'static str> {
    match (kind, rel) {
        ("h-entry" | "h-review" | "h-feed" | "h-news" | "h-product" | "h-card", "tag") => Some("p-category"),
        ("h-news", "principles") => Some("u-principles"),
        ("h-entry" | "h-review" | "h-news", "bookmark") => Some("u-url"),
        _ => None,
    }
}

// ── Parsing ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct Item {
    types: Vec<String>,
    id: Option<String>,
    props: Vec<(String, Vec<PVal>)>,
    children: Vec<Item>,
    /// A nested item's `value` (as a property value).
    value: Option<Value>,
    /// A nested item's `html` (as an `e-*` property value).
    html: Option<String>,
    /// It has a `p-*` or `e-*` property (no implied name then).
    explicit_name: bool,
    /// The prefix each property was first stated with.
    prefixes: Vec<(String, &'static str)>,
    lang: Option<String>,
}

#[derive(Debug, Clone)]
enum PVal {
    Text(String),
    Url { value: String, alt: Option<String> },
    Html { html: String, value: String },
    Item(Box<Item>),
}

impl Item {
    fn push(&mut self, name: &str, v: PVal) {
        match self.props.iter_mut().find(|(k, _)| k == name) {
            Some((_, vs)) => vs.push(v),
            None => self.props.push((name.to_owned(), vec![v])),
        }
    }

    fn note(&mut self, name: &str, prefix: &'static str) {
        if matches!(prefix, "p" | "e") {
            self.explicit_name = true;
        }
        if !self.prefixes.iter().any(|(k, _)| k == name) {
            self.prefixes.push((name.to_owned(), prefix));
        }
    }

    fn has(&self, name: &str) -> bool {
        self.props.iter().any(|(k, _)| k == name)
    }

    /// The first value of `name`, when it was stated with `prefix`.
    /// The first `u-url`, as its value is written out (`{ value, alt }` for an image's).
    /// The first `u-url`, or the implied `url` (not a `url` stated as `p-url`, `dt-url`, `e-url`):
    /// a nested item's value under a `u-*` property (microformats2-parsing issue 66).
    fn first_url(&self) -> Option<Value> {
        if self.prefixes.iter().any(|(k, p)| k == "url" && *p != "u") {
            return None;
        }
        let (_, vs) = self.props.iter().find(|(k, _)| k == "url")?;
        Some(match vs.first()? {
            PVal::Url { value, alt: Some(alt) } => json!({ "value": value, "alt": alt }),
            PVal::Url { value, alt: None } => json!(value),
            PVal::Text(t) => json!(t),
            PVal::Html { value, .. } => json!(value),
            PVal::Item(i) => i.value.clone().unwrap_or(Value::Null),
        })
    }

    /// The first `p-name`, or the implied `name`: a nested item's value under a `p-*` property.
    fn first_name(&self) -> Option<String> {
        if self.prefixes.iter().any(|(k, p)| k == "name" && *p != "p") {
            return None;
        }
        self.first_text("name")
    }

    fn first_text(&self, name: &str) -> Option<String> {
        self.props
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.first())
            .map(|v| match v {
                PVal::Text(t) => t.clone(),
                PVal::Url { value, .. } => value.clone(),
                PVal::Html { value, .. } => value.clone(),
                PVal::Item(i) => i.value.as_ref().and_then(Value::as_str).unwrap_or("").to_owned(),
            })
    }
}

/// What kind of item an element is: microformats2 or classic.
#[derive(Clone, Copy)]
enum Mode {
    Mf2,
    Classic,
}

struct Parser<'d> {
    dom: &'d Dom,
    base: Option<&'d str>,
    depth: usize,
}

const MAX_NESTING: usize = 32;

impl Parser<'_> {
    /// The root classes of an element and how to read it, if it is an item.
    fn roots(&self, n: usize) -> Option<(Vec<String>, Mode)> {
        let classes = self.dom.classes(n);
        let mut mf2: Vec<String> = classes
            .iter()
            .filter_map(|c| mf2_name(c, "h-"))
            .map(|c| format!("h-{c}"))
            .collect();
        if !mf2.is_empty() {
            mf2.sort();
            mf2.dedup();
            return Some((mf2, Mode::Mf2));
        }
        let mut classic: Vec<String> = classes
            .iter()
            .filter_map(|c| backcompat_root(c))
            .map(str::to_owned)
            .collect();
        if classic.is_empty() {
            return None;
        }
        classic.sort();
        classic.dedup();
        Some((classic, Mode::Classic))
    }

    /// The property classes of an element (`p-name` …), as read under an item of `kind` in `mode`.
    fn props_of(&self, n: usize, mode: Mode, kinds: &[String]) -> Vec<String> {
        let classes = self.dom.classes(n);
        let mut out: Vec<String> = Vec::new();
        match mode {
            Mode::Mf2 => {
                for c in classes {
                    for prefix in ["p-", "u-", "dt-", "e-"] {
                        if let Some(name) = mf2_name(c, prefix) {
                            out.push(format!("{prefix}{name}"));
                        }
                    }
                }
            }
            Mode::Classic => {
                for c in classes {
                    for kind in kinds {
                        for p in backcompat_props(kind, c) {
                            if !out.iter().any(|x| x == p) {
                                out.push((*p).to_owned());
                            }
                        }
                    }
                }
                if let Some(rels) = self.dom.attr(n, "rel") {
                    for rel in rels.split_ascii_whitespace() {
                        for kind in kinds {
                            if let Some(p) = backcompat_rel(kind, rel) {
                                if !out.iter().any(|x| x == p) {
                                    out.push(p.to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn item(&mut self, n: usize, types: Vec<String>, mode: Mode) -> Item {
        let mut item = Item {
            types: types.clone(),
            // microformats2 roots only, as the tests and microformats-parser have it.
            id: matches!(mode, Mode::Mf2)
                .then(|| self.dom.attr(n, "id").filter(|s| !s.is_empty()).map(str::to_owned))
                .flatten(),
            props: Vec::new(),
            children: Vec::new(),
            value: None,
            html: None,
            explicit_name: false,
            prefixes: Vec::new(),
            lang: None,
        };
        self.depth += 1;
        if self.depth < MAX_NESTING {
            let mut nested_props = false;
            for c in self.dom.elements(n).collect::<Vec<_>>() {
                self.walk(c, &mut item, mode, &types, &mut nested_props);
            }
            self.implied(n, &mut item, nested_props, mode);
        }
        self.depth -= 1;
        item
    }

    /// An element under an item: its properties, its nested items, then its children.
    fn walk(&mut self, n: usize, item: &mut Item, mode: Mode, kinds: &[String], nested: &mut bool) {
        // hReview's `item` that is no microformat of its own is an `h-item` (its `fn`, `url`, `photo`).
        if matches!(mode, Mode::Classic)
            && kinds.iter().any(|k| k.starts_with("h-review"))
            && self.dom.has_class(n, "item")
            && self.roots(n).is_none()
        {
            *nested = true;
            let mut child = self.item(n, vec!["h-item".into()], Mode::Classic);
            child.value = Some(json!(child.first_text("name").unwrap_or_else(|| self.p_value(n))));
            item.push("item", PVal::Item(Box::new(child)));
            return;
        }
        let props = self.props_of(n, mode, kinds);
        // A classic item inside a classic one only nests where a property says so (or as a child).
        if let Some((types, child_mode)) = self.roots(n) {
            *nested = true;
            let root_types = types.clone();
            let root = Some((root_types.as_slice(), child_mode));
            let mut child = self.item(n, types, child_mode);
            if props.is_empty() {
                item.children.push(child);
                return;
            }
            for p in &props {
                let (prefix, name) = split_prop(p);
                let mut c = child.clone();
                c.value = Some(match prefix {
                    "p" => match self.value_class_in(n, false, root) {
                        Some(vs) => json!(vs.concat()),
                        None => child
                            .first_name()
                            .map(Value::String)
                            .unwrap_or_else(|| json!(self.p_value_in(n, root))),
                    },
                    "u" => match child.first_url() {
                        Some(u) => u,
                        // A `url` the child states as `p-url`, `dt-url` or `e-url`: the element
                        // parsed that way (the tests' `nested-microformat-mistyped`).
                        None => match child.prefixes.iter().find(|(k, _)| k == "url").map(|(_, p)| *p) {
                            Some("p") => json!(self.p_value(n)),
                            Some("dt") => json!(self.dt_value(n, None)),
                            Some("e") => json!(self.dom.text_of(n, self.base)),
                            _ => match self.u_value(n) {
                                (value, Some(alt)) => json!({ "value": value, "alt": alt }),
                                (value, None) => json!(value),
                            },
                        },
                    },
                    "dt" => json!(self.dt_value(n, None)),
                    _ => {
                        let mut html = String::new();
                        self.dom.inner_html(n, self.base, &mut html);
                        c.html = Some(trim(&html));
                        json!(self.dom.text_of(n, self.base))
                    }
                });
                item.note(name, prefix);
                item.push(name, PVal::Item(Box::new(c)));
            }
            child.value = None;
            return;
        }
        let mut last_date: Option<String> =
            item.props
                .iter()
                .find(|(k, _)| k == "start")
                .and_then(|(_, v)| match v.first() {
                    Some(PVal::Text(t)) => date_part(t),
                    _ => None,
                });
        for p in &props {
            let (prefix, name) = split_prop(p);
            item.note(name, prefix);
            let v = match prefix {
                "p" => PVal::Text(self.p_value(n)),
                "u" => {
                    let (value, alt) = self.u_value(n);
                    // An image's `alt` with its URL is microformats2's; classic properties are the URL.
                    PVal::Url {
                        value,
                        alt: alt.filter(|_| matches!(mode, Mode::Mf2)),
                    }
                }
                "dt" => {
                    let v = self.dt_value(n, last_date.as_deref());
                    if last_date.is_none() {
                        last_date = date_part(&v);
                    }
                    PVal::Text(v)
                }
                "e" => {
                    let mut html = String::new();
                    self.dom.inner_html(n, self.base, &mut html);
                    PVal::Html {
                        html: trim(&html),
                        value: self.dom.text_of(n, self.base),
                    }
                }
                _ => continue,
            };
            item.push(name, v);
        }
        for c in self.dom.elements(n).collect::<Vec<_>>() {
            self.walk(c, item, mode, kinds, nested);
        }
    }

    /// The value-class pattern: the values of `.value` / `.value-title` descendants (not inside nested items).
    fn value_class(&self, n: usize, dt: bool) -> Option<Vec<String>> {
        self.value_class_in(n, dt, None)
    }

    /// The value-class pattern of an element that is also an item (`root`: its types and kind),
    /// leaving out the `.value`s inside that item's own properties, classic ones included.
    fn value_class_in(&self, n: usize, dt: bool, root: Option<(&[String], Mode)>) -> Option<Vec<String>> {
        let mut out = Vec::new();
        self.collect_values(n, dt, &mut out, root);
        (!out.is_empty()).then_some(out)
    }

    fn collect_values(&self, n: usize, dt: bool, out: &mut Vec<String>, root: Option<(&[String], Mode)>) {
        for c in self.dom.elements(n).collect::<Vec<_>>() {
            if self.roots(c).is_some() && !self.dom.has_class(c, "value") {
                continue;
            }
            // A `.value` inside another property is that property's.
            let is_prop = self.dom.attr(c, "class").is_some_and(|cl| {
                cl.split_ascii_whitespace().any(|k| {
                    ["p-", "u-", "dt-", "e-"]
                        .iter()
                        .any(|p| k.starts_with(p) && k.len() > p.len())
                })
            }) || root.is_some_and(|(kinds, mode)| !self.props_of(c, mode, kinds).is_empty());
            if is_prop && !self.dom.has_class(c, "value") && !self.dom.has_class(c, "value-title") {
                continue;
            }
            if self.dom.has_class(c, "value-title") {
                if let Some(t) = self.dom.attr(c, "title") {
                    out.push(t.to_owned());
                }
                continue;
            }
            if self.dom.has_class(c, "value") {
                let name = self.dom.nodes[c].name.as_str();
                let v = match name {
                    "img" | "area" => self.dom.attr(c, "alt").map(str::to_owned),
                    "data" => self.dom.attr(c, "value").map(str::to_owned),
                    "abbr" => self.dom.attr(c, "title").map(str::to_owned),
                    "del" | "ins" | "time" if dt => self.dom.attr(c, "datetime").map(str::to_owned),
                    _ => None,
                };
                let v = v.unwrap_or_else(|| {
                    let mut s = String::new();
                    self.dom.raw_text(c, &mut s);
                    if dt {
                        trim(&s)
                    } else {
                        s
                    }
                });
                out.push(v);
                continue;
            }
            self.collect_values(c, dt, out, root);
        }
    }

    fn p_value(&self, n: usize) -> String {
        self.p_value_in(n, None)
    }

    fn p_value_in(&self, n: usize, root: Option<(&[String], Mode)>) -> String {
        if let Some(vs) = self.value_class_in(n, false, root) {
            return vs.concat();
        }
        let name = self.dom.nodes[n].name.as_str();
        if matches!(name, "abbr" | "link") {
            if let Some(t) = self.dom.attr(n, "title") {
                return t.to_owned();
            }
        }
        if matches!(name, "data" | "input") {
            if let Some(v) = self.dom.attr(n, "value") {
                return v.to_owned();
            }
        }
        if matches!(name, "img" | "area") {
            if let Some(a) = self.dom.attr(n, "alt") {
                return a.to_owned();
            }
            // Its own textContent is empty: images stand in for their text only inside an element.
            return String::new();
        }
        self.dom.text_of(n, self.base)
    }

    fn u_value(&self, n: usize) -> (String, Option<String>) {
        let name = self.dom.nodes[n].name.as_str();
        let attr = |a: &str| self.dom.attr(n, a).map(|v| resolve(v, self.base));
        let found = match name {
            "a" | "area" | "link" => attr("href"),
            "img" => {
                if let Some(src) = attr("src") {
                    return (src, self.dom.attr(n, "alt").map(str::to_owned));
                }
                None
            }
            "audio" | "video" | "source" | "iframe" => attr("src"),
            "object" => attr("data"),
            _ => None,
        };
        if let Some(v) = found {
            return (v, None);
        }
        if name == "video" {
            if let Some(p) = attr("poster") {
                return (p, None);
            }
        }
        // Every way of getting the value ends in its absolute URL (microformats2 parsing, `u-*`).
        if let Some(vs) = self.value_class(n, false) {
            return (resolve(&vs.concat(), self.base), None);
        }
        if name == "abbr" {
            if let Some(t) = self.dom.attr(n, "title") {
                return (resolve(t, self.base), None);
            }
        }
        if matches!(name, "data" | "input") {
            if let Some(v) = self.dom.attr(n, "value") {
                return (resolve(v, self.base), None);
            }
        }
        (resolve(&self.plain_text(n), self.base), None)
    }

    fn dt_value(&self, n: usize, implied_date: Option<&str>) -> String {
        // The value-class pattern, when its parts make a date or time.
        if let Some(parts) = self.value_class(n, true) {
            if parts.iter().any(|p| date_part(p).is_some() || is_time(p)) {
                return combine_datetime(&parts, implied_date);
            }
        }
        let name = self.dom.nodes[n].name.as_str();
        let v = match name {
            "time" | "ins" | "del" => self.dom.attr(n, "datetime").map(str::to_owned),
            "abbr" => self.dom.attr(n, "title").map(str::to_owned),
            "data" | "input" => self.dom.attr(n, "value").map(str::to_owned),
            _ => None,
        };
        let v = v.unwrap_or_else(|| {
            let mut s = String::new();
            self.dom.raw_text(n, &mut s);
            trim(&s)
        });
        // A time alone takes the date of an earlier date-time of the item.
        match implied_date {
            Some(d) if is_time(&v) => format!("{d} {}", normalize_time(&v)),
            _ => v,
        }
    }

    fn implied(&self, n: usize, item: &mut Item, nested: bool, mode: Mode) {
        let has_u = item
            .props
            .iter()
            .any(|(_, vs)| vs.iter().any(|v| matches!(v, PVal::Url { .. })));
        // Classic formats imply nothing (their parser predates implied properties), but names.
        let classic = matches!(mode, Mode::Classic);
        if !item.has("name") && !item.explicit_name && !nested && !classic {
            item.push("name", PVal::Text(self.implied_name(n)));
        }
        if !item.has("photo") && !has_u && !nested && !classic {
            if let Some((src, alt)) = self.implied_photo(n) {
                item.push("photo", PVal::Url { value: src, alt });
            }
        }
        if !item.has("url") && !has_u && !nested && !classic {
            if let Some(u) = self.implied_url(n) {
                item.push("url", PVal::Url { value: u, alt: None });
            }
        }
    }

    fn only_child(&self, n: usize) -> Option<usize> {
        let els: Vec<usize> = self.dom.elements(n).collect();
        (els.len() == 1).then(|| els[0])
    }

    fn only_of_type(&self, n: usize, name: &str) -> Option<usize> {
        let els: Vec<usize> = self
            .dom
            .elements(n)
            .filter(|&c| self.dom.nodes[c].name == name)
            .collect();
        (els.len() == 1 && self.roots(els[0]).is_none()).then(|| els[0])
    }

    fn implied_name(&self, n: usize) -> String {
        // The root itself: `img.h-x` and `area.h-x` by their `alt`, `abbr.h-x[title]` by its title,
        // even empty.
        match self.dom.nodes[n].name.as_str() {
            "img" | "area" => return self.dom.attr(n, "alt").map(trim).unwrap_or_default(),
            "abbr" => {
                if let Some(t) = self.dom.attr(n, "title") {
                    return trim(t);
                }
            }
            _ => {}
        }
        let alt_or_title = |e: usize| -> Option<String> {
            let name = self.dom.nodes[e].name.as_str();
            match name {
                "img" | "area" => self.dom.attr(e, "alt").map(trim).filter(|a| !a.is_empty()),
                "abbr" => self.dom.attr(e, "title").map(trim).filter(|a| !a.is_empty()),
                _ => None,
            }
        };
        if let Some(v) = alt_or_title(n) {
            return v;
        }
        if let Some(c) = self.only_child(n).filter(|&c| self.roots(c).is_none()) {
            if let Some(v) = alt_or_title(c) {
                return v;
            }
            if let Some(g) = self.only_child(c).filter(|&g| self.roots(g).is_none()) {
                if let Some(v) = alt_or_title(g) {
                    return v;
                }
            }
        }
        // `textContent` with images as their `alt` (a `src` only stands in when parsing `p-*`).
        let mut s = String::new();
        self.dom.alt_text(n, &mut s);
        trim(&s)
    }

    fn plain_text(&self, n: usize) -> String {
        let mut s = String::new();
        self.dom.raw_text(n, &mut s);
        trim(&s)
    }

    fn implied_photo(&self, n: usize) -> Option<(String, Option<String>)> {
        let photo = |e: usize| -> Option<(String, Option<String>)> {
            match self.dom.nodes[e].name.as_str() {
                "img" => self
                    .dom
                    .attr(e, "src")
                    .map(|s| (resolve(s, self.base), self.dom.attr(e, "alt").map(str::to_owned))),
                "object" => self.dom.attr(e, "data").map(|d| (resolve(d, self.base), None)),
                _ => None,
            }
        };
        if let Some(p) = photo(n) {
            return Some(p);
        }
        for tag in ["img", "object"] {
            if let Some(c) = self.only_of_type(n, tag) {
                if let Some(p) = photo(c) {
                    return Some(p);
                }
            }
        }
        let c = self.only_child(n).filter(|&c| self.roots(c).is_none())?;
        for tag in ["img", "object"] {
            if let Some(g) = self.only_of_type(c, tag) {
                if let Some(p) = photo(g) {
                    return Some(p);
                }
            }
        }
        None
    }

    fn implied_url(&self, n: usize) -> Option<String> {
        let href = |e: usize| -> Option<String> {
            match self.dom.nodes[e].name.as_str() {
                "a" | "area" => self.dom.attr(e, "href").map(|h| resolve(h, self.base)),
                _ => None,
            }
        };
        if let Some(u) = href(n) {
            return Some(u);
        }
        for tag in ["a", "area"] {
            if let Some(c) = self.only_of_type(n, tag) {
                if let Some(u) = href(c) {
                    return Some(u);
                }
            }
        }
        let c = self.only_child(n).filter(|&c| self.roots(c).is_none())?;
        for tag in ["a", "area"] {
            if let Some(g) = self.only_of_type(c, tag) {
                if let Some(u) = href(g) {
                    return Some(u);
                }
            }
        }
        None
    }
}

fn date_part(v: &str) -> Option<String> {
    let d = v.get(..10)?;
    (d.as_bytes()[4] == b'-'
        && d.as_bytes()[7] == b'-'
        && d.bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()))
    .then(|| d.to_owned())
}

/// A timezone offset as the value-class pattern writes it: "If the offset uses XX:YY format,
/// remove the colon so it is XXYY format" (`-08:00` → `-0800`); `-0800`, `-08` and `Z` as they are.
fn compact_zone(z: &str) -> String {
    let z = z.trim();
    if z.eq_ignore_ascii_case("z") {
        return "Z".into();
    }
    z.replace(':', "")
}

/// `19:00-08:00` → (`19:00`, `-0800`).
fn split_zone(v: &str) -> (&str, Option<String>) {
    let v = v.trim();
    if let Some(t) = v.strip_suffix(['Z', 'z']) {
        return (t, Some("Z".into()));
    }
    if let Some(i) = v.rfind(['+', '-']) {
        let (t, z) = v.split_at(i);
        if i > 0 && is_zone(z) && (t.contains(':') || t.len() <= 2) {
            return (t, Some(compact_zone(z)));
        }
    }
    (v, None)
}

fn is_time(v: &str) -> bool {
    let v = split_zone(v).0;
    v.chars().next().is_some_and(|c| c.is_ascii_digit())
        && (v.contains(':') || v.to_ascii_lowercase().ends_with("am") || v.to_ascii_lowercase().ends_with("pm"))
        && !v.contains('-')
}

fn is_date(v: &str) -> bool {
    let v = v.trim();
    v.len() >= 8 && v.as_bytes().get(4) == Some(&b'-') && !v.contains(':')
}

fn is_zone(v: &str) -> bool {
    let v = v.trim();
    v == "Z"
        || ((v.starts_with('+') || v.starts_with('-')) && v[1..].replace(':', "").bytes().all(|b| b.is_ascii_digit()))
}

/// `9pm` → `21:00`, `9:30 am` → `09:30`.
fn normalize_time(v: &str) -> String {
    let l = v.trim().to_ascii_lowercase().replace(['.', ' '], "");
    let (body, pm) = if let Some(b) = l.strip_suffix("pm") {
        (b, Some(true))
    } else if let Some(b) = l.strip_suffix("am") {
        (b, Some(false))
    } else {
        (l.as_str(), None)
    };
    let Some(pm) = pm else { return v.trim().to_owned() };
    let mut parts = body.split(':');
    let h: u32 = parts.next().and_then(|h| h.parse().ok()).unwrap_or(0);
    let rest: Vec<&str> = parts.collect();
    let h = match (h, pm) {
        (12, false) => 0,
        (h, true) if h < 12 => h + 12,
        (h, _) => h,
    };
    let mut out = format!("{h:02}");
    if rest.is_empty() {
        out.push_str(":00");
    }
    for r in rest {
        out.push(':');
        out.push_str(r);
    }
    out
}

/// The value-class pattern for dates: a date, a time and a zone, wherever they are among the values.
fn combine_datetime(parts: &[String], implied_date: Option<&str>) -> String {
    let date = parts.iter().find(|p| is_date(p)).cloned();
    let (time, own_zone) = match parts.iter().find(|p| is_time(p)) {
        Some(t) => {
            let (t, z) = split_zone(t);
            (Some(normalize_time(t)), z)
        }
        None => (None, None),
    };
    let zone = own_zone.or_else(|| parts.iter().find(|p| is_zone(p)).map(|z| compact_zone(z)));
    match (date.or_else(|| implied_date.map(str::to_owned)), time) {
        (Some(d), Some(t)) => format!("{d} {t}{}", zone.unwrap_or_default()),
        (Some(d), None) => d,
        (None, Some(t)) => format!("{t}{}", zone.unwrap_or_default()),
        (None, None) => parts.concat(),
    }
}

/// `n` of `src` (and all under it) copied under `parent` in `out`.
fn copy_subtree(src: &Dom, n: usize, out: &mut Dom, parent: usize) {
    out.nodes.push(DNode {
        name: src.nodes[n].name.clone(),
        attrs: src.nodes[n].attrs.clone(),
        text: src.nodes[n].text.clone(),
        children: Vec::new(),
    });
    let i = out.nodes.len() - 1;
    out.nodes[parent].children.push(i);
    for &c in &src.nodes[n].children {
        copy_subtree(src, c, out, i);
    }
}

/// HTML's "special" elements (those that can be a furthest block), as an open element can be one.
fn is_special(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "applet"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "button"
            | "caption"
            | "center"
            | "colgroup"
            | "dd"
            | "details"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "frameset"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "header"
            | "hgroup"
            | "html"
            | "iframe"
            | "li"
            | "listing"
            | "main"
            | "marquee"
            | "menu"
            | "nav"
            | "noembed"
            | "noframes"
            | "noscript"
            | "object"
            | "ol"
            | "p"
            | "plaintext"
            | "pre"
            | "script"
            | "section"
            | "select"
            | "style"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "template"
            | "textarea"
            | "tfoot"
            | "th"
            | "thead"
            | "title"
            | "tr"
            | "ul"
            | "xmp"
    )
}

/// Text appended under `p` (adjacent text is one text node).
fn push_text(dom: &mut Dom, p: usize, text: &str, max: usize) {
    if dom.nodes.len() >= max {
        return;
    }
    if let Some(&last) = dom.nodes[p].children.last() {
        if dom.nodes[last].name.is_empty() {
            dom.nodes[last].text.push_str(text);
            return;
        }
    }
    dom.nodes.push(DNode {
        text: text.to_owned(),
        ..DNode::default()
    });
    let i = dom.nodes.len() - 1;
    dom.nodes[p].children.push(i);
}

/// Copies `n` of `src` under `parent` in `out`, the include pattern applied inside classic items:
/// an including `<a>`/`<object>` is replaced by the element it refers to, the elements an
/// `itemref` or `headers` refers to are appended to it. An element is never included inside
/// itself (`ancestors`: the ids of the elements being copied).
#[allow(clippy::too_many_arguments)]
fn graft(
    src: &Dom,
    n: usize,
    inc: &Includes,
    out: &mut Dom,
    parent: Option<usize>,
    classic: bool,
    ancestors: &mut Vec<String>,
    max: usize,
    depth: usize,
) {
    // Includes inside includes: what they make is bounded as deep as a kept tree.
    if out.nodes.len() >= max || depth >= MAX_TREE_DEPTH {
        return;
    }
    let node = &src.nodes[n];
    let parent_of = |out: &mut Dom, i: usize| {
        if let Some(p) = parent {
            out.nodes[p].children.push(i);
        }
    };
    if node.name.is_empty() {
        out.nodes.push(DNode {
            text: node.text.clone(),
            ..DNode::default()
        });
        let i = out.nodes.len() - 1;
        parent_of(out, i);
        return;
    }
    let attr = |a: &str| node.attrs.iter().find(|(k, _)| k == a).map(|(_, v)| v.as_str());
    // Inside a classic item (its root, or below it with no microformats2 root in between).
    let classes: Vec<&str> = attr("class")
        .map(|c| c.split_ascii_whitespace().collect())
        .unwrap_or_default();
    let classic = if classes.iter().any(|c| mf2_name(c, "h-").is_some()) {
        false
    } else {
        classic || classes.iter().any(|c| backcompat_root(c).is_some())
    };
    let target =
        |id: &str, ancestors: &[String]| inc.ids.get(id).copied().filter(|_| !ancestors.iter().any(|a| a == id));
    let refs = if classic {
        include_refs(&node.name, attr)
    } else {
        Vec::new()
    };
    let replaced = classes.contains(&"include") && matches!(node.name.as_str(), "a" | "object");
    if replaced {
        if let Some(t) = refs.first().and_then(|id| target(id, ancestors)) {
            ancestors.push(refs[0].to_owned());
            graft(&inc.side, t, inc, out, parent, classic, ancestors, max, depth + 1);
            ancestors.pop();
            return;
        }
    }
    out.nodes.push(DNode {
        name: node.name.clone(),
        attrs: node.attrs.clone(),
        ..DNode::default()
    });
    let i = out.nodes.len() - 1;
    parent_of(out, i);
    let own_id = attr("id").map(str::to_owned);
    if let Some(id) = &own_id {
        ancestors.push(id.clone());
    }
    for &c in &node.children {
        graft(src, c, inc, out, Some(i), classic, ancestors, max, depth + 1);
    }
    if !replaced {
        for id in refs {
            if let Some(t) = target(id, ancestors) {
                ancestors.push(id.to_owned());
                graft(&inc.side, t, inc, out, Some(i), classic, ancestors, max, depth + 1);
                ancestors.pop();
            }
        }
    }
    if own_id.is_some() {
        ancestors.pop();
    }
}

// ── The streaming pass ───────────────────────────────────────────────────

/// How microformats are read.
#[derive(Clone, Copy, Debug)]
pub struct MicroformatsOptions {
    /// At most this many nodes in the small trees kept under items (a guard).
    pub max_nodes: usize,
}

impl Default for MicroformatsOptions {
    fn default() -> Self {
        MicroformatsOptions { max_nodes: 200_000 }
    }
}

pub struct Microformats {
    opts: MicroformatsOptions,
    /// The tree being kept under the current top-level item, and its open path.
    dom: Option<Dom>,
    open: Vec<usize>,
    /// The tree node each element of the document under the item made (None: past `max_nodes`),
    /// to close it when the document closes it, unless it is closed already.
    frames: Vec<Option<usize>>,
    /// The depth (in the document) of the top-level item's element.
    root_depth: usize,
    items: Vec<Item>,
    rels: BTreeMap<String, Vec<String>>,
    rel_urls: BTreeMap<String, Map<String, Value>>,
    /// The `rel` link whose text is being collected.
    rel_text: Option<(String, usize, String)>,
    /// The include pattern (classic microformats): the ids items refer to, copies of the elements
    /// that may be referred to (by id), and the items that refer to some, parsed at the end.
    includes: Includes,
}

/// What the include pattern needs: `<a class="include" href="#id">`, `<object class="include"
/// data="#id">`, `itemref="id …"` and table cells' `headers="id …"` take the element of that id
/// into a classic item, as if it were written there.
#[derive(Default)]
struct Includes {
    /// Ids referred to so far.
    wanted: HashSet<String>,
    /// Copies of the elements that may be referred to: those of a wanted id, classic classes, or
    /// table cells with an id (a cell's `headers` refers back).
    side: Dom,
    ids: HashMap<String, usize>,
    /// The open copied elements, with their depth in the document.
    open: Vec<(usize, usize)>,
    /// The current item refers to something.
    refers: bool,
    /// Items that refer to something: where they go among the items, and their tree.
    pending: Vec<(usize, Dom)>,
}

/// At most this deep a kept tree (elements deeper still give their text to the deepest one
/// kept): the parse of a tree recurses as deep as it goes.
const MAX_TREE_DEPTH: usize = 256;

/// At most this many nodes kept for the include pattern.
const MAX_INCLUDE_NODES: usize = 50_000;

/// The classic types, for telling whether a class is a classic property.
const CLASSIC_KINDS: &[&str] = &[
    "h-adr",
    "h-card",
    "h-entry",
    "h-event",
    "h-feed",
    "h-geo",
    "h-item",
    "h-news",
    "h-product",
    "h-recipe",
    "h-resume",
    "h-review",
    "h-review-aggregate",
];

fn is_classic_class(class: &str) -> bool {
    class == "include"
        || backcompat_root(class).is_some()
        || CLASSIC_KINDS.iter().any(|k| !backcompat_props(k, class).is_empty())
}

/// The ids an element refers to by the include pattern.
fn include_refs<'a>(name: &str, attr: impl Fn(&str) -> Option<&'a str>) -> Vec<&'a str> {
    let mut out = Vec::new();
    if attr("class").is_some_and(|c| c.split_ascii_whitespace().any(|k| k == "include")) {
        let target = match name {
            "a" => attr("href"),
            "object" => attr("data"),
            _ => None,
        };
        if let Some(id) = target
            .and_then(|t| t.trim().strip_prefix('#'))
            .filter(|i| !i.is_empty())
        {
            out.push(id);
        }
    }
    if let Some(r) = attr("itemref") {
        out.extend(r.split_ascii_whitespace());
    }
    if matches!(name, "td" | "th") {
        if let Some(h) = attr("headers") {
            out.extend(h.split_ascii_whitespace());
        }
    }
    out
}

fn is_root_class(class: &str) -> bool {
    mf2_name(class, "h-").is_some() || backcompat_root(class).is_some()
}

impl Microformats {
    pub fn new(opts: MicroformatsOptions) -> Microformats {
        Microformats {
            opts,
            dom: None,
            open: Vec::new(),
            frames: Vec::new(),
            root_depth: 0,
            items: Vec::new(),
            rels: BTreeMap::new(),
            rel_urls: BTreeMap::new(),
            rel_text: None,
            includes: Includes::default(),
        }
    }

    pub fn wants_text(&self) -> bool {
        self.dom.is_some() || self.rel_text.is_some() || !self.includes.open.is_empty()
    }

    pub fn open(&mut self, tree: &Tree, base: Option<&str>) {
        let depth = tree.frames.len() - 1;
        let el = &tree.frames[depth].el;
        let link = matches!(el.name(), "a" | "area" | "link");
        if let (true, Some(rel), Some(href)) = (
            link,
            link.then(|| el.attr("rel")).flatten(),
            link.then(|| el.attr("href")).flatten(),
        ) {
            let url = resolve(href, base);
            let rels: Vec<String> = rel.split_ascii_whitespace().map(|r| r.to_ascii_lowercase()).collect();
            for r in &rels {
                let list = self.rels.entry(r.clone()).or_default();
                if !list.contains(&url) {
                    list.push(url.clone());
                }
            }
            let entry = self.rel_urls.entry(url.clone()).or_default();
            let all = entry.entry("rels").or_insert_with(|| json!([]));
            if let Value::Array(a) = all {
                for r in &rels {
                    if !a.iter().any(|x| x == r) {
                        a.push(json!(r));
                    }
                }
                // Sorted, as the microformats tests and microformats-parser have them.
                a.sort_by(|x, y| x.as_str().cmp(&y.as_str()));
            }
            for a in ["hreflang", "media", "title", "type"] {
                if let Some(v) = el.attr(a) {
                    entry.entry(a).or_insert_with(|| json!(v));
                }
            }
            if el.name() != "link" && !entry.contains_key("text") {
                self.rel_text = Some((url, depth, String::new()));
            }
        }
        self.capture_open(el, depth);
        if self.dom.is_some() && el.name() == "a" {
            self.adopt_a(base);
        }
        let starts = self.dom.is_none()
            && el
                .attr("class")
                .is_some_and(|c| c.split_ascii_whitespace().any(is_root_class));
        if starts {
            self.dom = Some(Dom::default());
            self.open.clear();
            self.frames.clear();
            self.root_depth = depth;
        }
        if let Some(dom) = &mut self.dom {
            if dom.nodes.len() >= self.opts.max_nodes || self.open.len() >= MAX_TREE_DEPTH {
                self.frames.push(None);
                return;
            }
            let node = DNode {
                name: el.name().to_owned(),
                attrs: el.attrs().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
                ..DNode::default()
            };
            dom.nodes.push(node);
            let i = dom.nodes.len() - 1;
            if let Some(&p) = self.open.last() {
                dom.nodes[p].children.push(i);
            }
            self.open.push(i);
            self.frames.push(Some(i));
            let refs = include_refs(el.name(), |a| el.attr(a));
            if !refs.is_empty() {
                self.includes.refers = true;
                self.includes.wanted.extend(refs.into_iter().map(str::to_owned));
            }
        }
    }

    /// Keeps a copy of an element that the include pattern may refer to (and of all inside it).
    fn capture_open(&mut self, el: &crate::tree::Element, depth: usize) {
        let inc = &mut self.includes;
        if inc.side.nodes.len() >= MAX_INCLUDE_NODES {
            return;
        }
        let id = el.attr("id").filter(|i| !i.is_empty());
        let candidate = id.is_some_and(|id| {
            inc.wanted.contains(id)
                || matches!(el.name(), "td" | "th")
                || el
                    .attr("class")
                    .is_some_and(|c| c.split_ascii_whitespace().any(is_classic_class))
        });
        if (!candidate && inc.open.is_empty()) || inc.open.len() >= MAX_TREE_DEPTH {
            return;
        }
        inc.side.nodes.push(DNode {
            name: el.name().to_owned(),
            attrs: el.attrs().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
            ..DNode::default()
        });
        let i = inc.side.nodes.len() - 1;
        if let Some(&(p, _)) = inc.open.last() {
            inc.side.nodes[p].children.push(i);
        }
        inc.open.push((i, depth));
        if let Some(id) = id {
            inc.ids.entry(id.to_owned()).or_insert(i);
        }
    }

    pub fn text(&mut self, text: &str) {
        if let Some((_, _, t)) = &mut self.rel_text {
            if t.len() < 4096 {
                t.push_str(text);
            }
        }
        if let Some(&(p, _)) = self.includes.open.last() {
            push_text(&mut self.includes.side, p, text, MAX_INCLUDE_NODES);
        }
        let Some(dom) = &mut self.dom else { return };
        let Some(&p) = self.open.last() else { return };
        if dom.nodes.len() >= self.opts.max_nodes {
            return;
        }
        // Adjacent text is one text node.
        if let Some(&last) = dom.nodes[p].children.last() {
            if dom.nodes[last].name.is_empty() {
                dom.nodes[last].text.push_str(text);
                return;
            }
        }
        dom.nodes.push(DNode {
            text: text.to_owned(),
            ..DNode::default()
        });
        let i = dom.nodes.len() - 1;
        dom.nodes[p].children.push(i);
    }

    pub fn close(&mut self, tree: &Tree, base: Option<&str>) {
        let depth = tree.frames.len() - 1;
        if self.rel_text.as_ref().is_some_and(|(_, d, _)| *d == depth) {
            if let Some((url, _, text)) = self.rel_text.take() {
                if let Some(e) = self.rel_urls.get_mut(&url) {
                    e.entry("text").or_insert_with(|| json!(text));
                }
            }
        }
        if self.includes.open.last().is_some_and(|&(_, d)| d == depth) {
            self.includes.open.pop();
        }
        if self.dom.is_none() {
            return;
        }
        if let Some(Some(node)) = self.frames.pop() {
            if let Some(at) = self.open.iter().rposition(|&x| x == node) {
                self.open.truncate(at);
            }
        }
        if depth == self.root_depth {
            if let Some(dom) = self.dom.take() {
                self.end_item(dom, base);
            }
        }
    }

    /// An item's tree complete: parsed (or kept for the end, when it uses the include pattern).
    fn end_item(&mut self, dom: Dom, base: Option<&str>) {
        if std::mem::take(&mut self.includes.refers) {
            // What it refers to may come later: parsed at the end.
            self.includes.pending.push((self.items.len(), dom));
        } else if !dom.nodes.is_empty() {
            let mut p = Parser {
                dom: &dom,
                base,
                depth: 0,
            };
            if let Some((types, mode)) = p.roots(0) {
                let item = p.item(0, types, mode);
                self.items.push(item);
            }
        }
    }

    /// An `<a>` inside an open `<a>`: HTML's tree construction closes the outer one first (the
    /// adoption agency algorithm, run for an `a` start tag), where htmlparser2's tree nests them.
    /// microformats are defined on HTML's tree, so the item's tree is made to follow it: with no
    /// block element in between the outer `a` just closes; with one (the furthest block), that
    /// block moves after the outer `a`, and an empty copy of the `a` takes the block's children.
    fn adopt_a(&mut self, base: Option<&str>) {
        let Some(dom) = &mut self.dom else { return };
        let Some(i) = self.open.iter().rposition(|&x| dom.nodes[x].name == "a") else {
            return;
        };
        // A marker (a cell, a caption, an object …) between them: the outer `a` is out of reach.
        if self.open[i + 1..].iter().any(|&x| {
            matches!(
                dom.nodes[x].name.as_str(),
                "td" | "th" | "caption" | "applet" | "object" | "marquee" | "template"
            )
        }) {
            return;
        }
        let a = self.open[i];
        let block = self.open[i + 1..]
            .iter()
            .position(|&x| is_special(&dom.nodes[x].name))
            .map(|k| i + 1 + k);
        let copy = |dom: &mut Dom, children: Vec<usize>| {
            dom.nodes.push(DNode {
                name: dom.nodes[a].name.clone(),
                attrs: dom.nodes[a].attrs.clone(),
                children,
                ..DNode::default()
            });
            dom.nodes.len() - 1
        };
        if i > 0 {
            if let Some(j) = block {
                let fb = self.open[j];
                let (old_parent, parent) = (self.open[j - 1], self.open[i - 1]);
                dom.nodes[old_parent].children.retain(|&c| c != fb);
                let at = dom.nodes[parent]
                    .children
                    .iter()
                    .position(|&c| c == a)
                    .map_or(0, |k| k + 1);
                dom.nodes[parent].children.insert(at, fb);
                let children = std::mem::take(&mut dom.nodes[fb].children);
                let clone = copy(dom, children);
                dom.nodes[fb].children = vec![clone];
                self.open.truncate(i);
                self.open.push(fb);
            } else {
                self.open.truncate(i);
            }
            return;
        }
        // The outer `a` is the item's root: the item ends here; with a furthest block, the copy
        // of the root (with the block's children) is an item of its own after it.
        let mut copied = None;
        if let Some(j) = block {
            let fb = self.open[j];
            let old_parent = self.open[j - 1];
            dom.nodes[old_parent].children.retain(|&c| c != fb);
            let mut out = Dom::default();
            out.nodes.push(DNode {
                name: dom.nodes[a].name.clone(),
                attrs: dom.nodes[a].attrs.clone(),
                ..DNode::default()
            });
            for &c in &dom.nodes[fb].children {
                copy_subtree(dom, c, &mut out, 0);
            }
            copied = Some(out);
        }
        self.open.clear();
        self.frames.clear();
        if let Some(dom) = self.dom.take() {
            self.end_item(dom, base);
        }
        if let Some(out) = copied {
            self.end_item(out, base);
        }
    }

    /// The items that use the include pattern, with what they include, among the others.
    pub fn finish(&mut self, base: Option<&str>) {
        let pending = std::mem::take(&mut self.includes.pending);
        // What all the items together may grow to by what they include (a page of many items
        // including one large element stays small).
        let mut budget = self
            .opts
            .max_nodes
            .max(pending.iter().map(|(_, d)| d.nodes.len()).sum::<usize>() * 4);
        for (at, dom) in pending.into_iter().rev() {
            let mut out = Dom::default();
            let mut ancestors = Vec::new();
            if !dom.nodes.is_empty() {
                let max = self.opts.max_nodes.min(budget.max(dom.nodes.len()));
                graft(&dom, 0, &self.includes, &mut out, None, false, &mut ancestors, max, 0);
                budget = budget.saturating_sub(out.nodes.len());
            }
            let mut p = Parser {
                dom: &out,
                base,
                depth: 0,
            };
            if let Some((types, mode)) = (!out.nodes.is_empty()).then(|| p.roots(0)).flatten() {
                let item = p.item(0, types, mode);
                self.items.insert(at, item);
            }
        }
    }

    /// The microformats2 JSON: `{ items, rels, rel-urls }`.
    pub fn json(&self) -> Value {
        let mut rel_urls = Map::new();
        for (k, v) in &self.rel_urls {
            rel_urls.insert(k.clone(), Value::Object(v.clone()));
        }
        json!({
            "items": self.items.iter().map(item_json).collect::<Vec<_>>(),
            "rels": self.rels,
            "rel-urls": rel_urls,
        })
    }

    pub fn into_graph(&self, graph: &mut Graph) {
        for item in &self.items {
            let i = add_item(graph, item);
            graph.nodes[i].top = true;
        }
    }
}

fn item_json(item: &Item) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), json!(item.types));
    if let Some(id) = &item.id {
        o.insert("id".into(), json!(id));
    }
    let mut props = Map::new();
    for (k, vs) in &item.props {
        props.insert(
            k.clone(),
            Value::Array(
                vs.iter()
                    .map(|v| match v {
                        PVal::Text(t) => json!(t),
                        PVal::Url { value, alt: None } => json!(value),
                        PVal::Url { value, alt: Some(alt) } => json!({ "value": value, "alt": alt }),
                        PVal::Html { html, value } => json!({ "html": html, "value": value }),
                        PVal::Item(i) => {
                            let mut j = item_json(i);
                            if let Some(h) = &i.html {
                                j["html"] = json!(h);
                            }
                            if let Some(v) = &i.value {
                                j["value"] = v.clone();
                            }
                            j
                        }
                    })
                    .collect(),
            ),
        );
    }
    o.insert("properties".into(), Value::Object(props));
    if !item.children.is_empty() {
        o.insert(
            "children".into(),
            Value::Array(item.children.iter().map(item_json).collect()),
        );
    }
    if let Some(l) = &item.lang {
        o.insert("lang".into(), json!(l));
    }
    Value::Object(o)
}

fn add_item(graph: &mut Graph, item: &Item) -> usize {
    let mut n = Node::new(Format::Microformats);
    n.types = item.types.clone();
    n.id = item.id.clone().map(|id| format!("#{id}"));
    let i = graph.add(n);
    for (k, vs) in &item.props {
        for v in vs {
            let val = match v {
                PVal::Text(t) => Val::Text(t.clone()),
                PVal::Url { value, .. } => Val::Iri(value.clone()),
                PVal::Html { value, .. } => Val::Text(value.clone()),
                PVal::Item(child) => Val::Node(add_item(graph, child)),
            };
            graph.nodes[i].push(k, val);
        }
    }
    for child in &item.children {
        let c = add_item(graph, child);
        graph.nodes[i].push("children", Val::Node(c));
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_names() {
        assert_eq!(mf2_name("h-event", "h-"), Some("event".into()));
        assert_eq!(mf2_name("h-x-test", "h-"), Some("x-test".into()));
        assert_eq!(mf2_name("h-", "h-"), None);
        assert_eq!(mf2_name("h-Event", "h-"), None);
        assert_eq!(mf2_name("h-123", "h-"), None);
        assert_eq!(mf2_name("dt-start", "dt-"), Some("start".into()));
    }

    #[test]
    fn date_times() {
        assert_eq!(normalize_time("9pm"), "21:00");
        assert_eq!(normalize_time("9:30 am"), "09:30");
        assert_eq!(
            combine_datetime(&["2026-11-14".into(), "20:00".into(), "+01:00".into()], None),
            "2026-11-14 20:00+0100"
        );
        assert_eq!(
            combine_datetime(&["2026-11-14".into(), "20:00-08".into()], None),
            "2026-11-14 20:00-08"
        );
        assert_eq!(
            combine_datetime(&["23:00".into()], Some("2026-11-14")),
            "2026-11-14 23:00"
        );
    }
}
