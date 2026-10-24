//! A page stripped down for a reader with a budget (a model's prompt), as
//! festimap's `cleanForLearn` does, while it streams: the `<title>`, the
//! OpenGraph and description `<meta>`s and the JSON-LD (cut) up front, then
//! the body without scripts, styles, forms, media and comments; only the
//! attributes a selector can name (and `data-*`) kept; text runs cut; more
//! than a few alike siblings (a listing's 200 cards) collapsed to
//! `<!-- N more alike -->`; wrappers left empty dropped; the whole stopped
//! at the budget, open tags closed.

use std::fmt::Write as _;

use regex::Regex;

use crate::skeleton::DEFAULT_IGNORE;
use crate::text::{collapse, escape_attr, escape_text};
use crate::tree::{Handler, Text, Tree};

/// Options of the strip pass (festimap's defaults).
#[derive(Clone, Debug)]
pub struct StripOptions {
    /// The output's budget (festimap: 12k tokens, 3 characters each). This and
    /// every other length here is in UTF-16 units, as JavaScript's `length`
    /// counts: a character past U+FFFF (an emoji, 𝕊) is two.
    pub max_chars: usize,
    /// A text run longer than this is cut (`…`).
    pub max_text: usize,
    /// An attribute value longer than this is cut (a `data-*` one dropped).
    pub max_attr: usize,
    /// Siblings alike (same tag and classes) past this many are collapsed.
    pub max_alike: usize,
    /// A JSON-LD block longer than this is cut.
    pub max_json_ld: usize,
    /// Elements dropped with their content.
    pub drop: Vec<String>,
    /// Attributes kept, besides `data-*`.
    pub keep_attrs: Vec<String>,
    pub keep_data: bool,
    /// Title, `<meta>`s and JSON-LD up front.
    pub head: bool,
    /// The `<meta>` keys (property or name, lower case) kept up front.
    pub meta: Regex,
    /// Elements dropped when nothing is left inside them and they have no attribute but `class`.
    pub drop_empty: Vec<String>,
}

const KEPT_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "itemprop",
    "itemtype",
    "itemscope",
    "href",
    "src",
    "datetime",
    "content",
    "property",
    "name",
    "rel",
    "title",
    "alt",
    "aria-label",
    "lang",
    "role",
];

impl Default for StripOptions {
    fn default() -> Self {
        StripOptions {
            max_chars: 36_000,
            max_text: 80,
            max_attr: 120,
            max_alike: 4,
            max_json_ld: 4_000,
            drop: DEFAULT_IGNORE.iter().map(|s| s.to_string()).collect(),
            keep_attrs: KEPT_ATTRIBUTES.iter().map(|s| s.to_string()).collect(),
            keep_data: true,
            head: true,
            meta: Regex::new("^(?:og:|twitter:title|description$|event:|music:)").expect("a valid regex"),
            drop_empty: ["div", "span", "section", "p", "li", "ul"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// The stripped page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stripped {
    pub html: String,
    /// Its estimated tokens: 3 UTF-16 units each.
    pub tokens: usize,
    /// The budget cut it short.
    pub truncated: bool,
}

/// Elements written without an end tag.
fn bare(name: &str) -> bool {
    matches!(name, "area" | "br" | "col" | "embed" | "hr" | "img" | "meta" | "wbr")
}

/// The length of `s` in UTF-16 units, as JavaScript measures `length`: a
/// character past U+FFFF counts twice.
pub(crate) fn units(s: &str) -> usize {
    // Every byte but a continuation byte starts a character; a 4-byte one starts two units.
    s.bytes()
        .map(|b| usize::from((b as i8) >= -0x40) + usize::from(b >= 0xF0))
        .sum()
}

/// What JavaScript's `trimEnd` takes off: `\s` without the zero-width space.
fn trim_end(s: &str) -> &str {
    s.trim_end_matches(|c| c != '\u{200b}' && crate::text::is_space(c))
}

/// At most `max` UTF-16 units, `…` when cut: festimap's
/// `s.slice(0, max).trimEnd() + "…"`. A cut through a character past U+FFFF
/// leaves JavaScript half of it, a lone surrogate, written U+FFFD here (as
/// any UTF-8 encoder writes it).
fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        // At most one unit per byte: nothing to cut.
        return s.to_owned();
    }
    let mut n = 0;
    for (at, c) in s.char_indices() {
        let w = c.len_utf16();
        if n + w > max {
            return if n < max {
                format!("{}\u{FFFD}…", &s[..at])
            } else {
                format!("{}…", trim_end(&s[..at]))
            };
        }
        n += w;
    }
    s.to_owned()
}

/// festimap's last step, `.replace(/<(div|span|…)(?: class="[^"]*")?><\/\1>/g, "")`
/// on the whole output: each of `names` written empty, with no attribute but
/// `class`, taken out, in one pass (`<div><span></span></div>` keeps its `<div>`).
fn drop_empty(html: String, names: &[String]) -> String {
    let b = html.as_bytes();
    let matched = |i: usize| -> Option<usize> {
        names.iter().find_map(|n| {
            let mut j = i + 1;
            b[j..].starts_with(n.as_bytes()).then_some(())?;
            j += n.len();
            if b[j..].starts_with(b" class=\"") {
                j += 8;
                j += memchr::memchr(b'"', &b[j..])? + 1;
            }
            (b[j..].starts_with(b"></")
                && b[j + 3..].starts_with(n.as_bytes())
                && b.get(j + 3 + n.len()) == Some(&b'>'))
            .then_some(j + 4 + n.len())
        })
    };
    let mut out: Option<String> = None;
    let (mut from, mut at) = (0, 0);
    while let Some(i) = memchr::memchr(b'<', &b[at..]).map(|i| at + i) {
        match matched(i) {
            Some(end) => {
                out.get_or_insert_with(|| String::with_capacity(html.len()))
                    .push_str(&html[from..i]);
                from = end;
                at = end;
            }
            None => at = i + 1,
        }
    }
    match out {
        Some(mut out) => {
            out.push_str(&html[from..]);
            out
        }
        None => html,
    }
}

#[derive(Default)]
struct Level {
    /// Nothing inside is written.
    skip: bool,
    /// Its start tag was written, and it has an end tag: its [`Element`]'s index.
    element: Option<usize>,
    alike: Vec<(String, usize)>,
    collapsed: usize,
}

/// An element written: where its start tag begins in the body, how long its
/// name is, and where its end tag begins (`usize::MAX` while it is open).
struct Element {
    open: usize,
    name: usize,
    close: usize,
}

/// A budget check: the body's length in bytes when it was made, and the
/// UTF-16 units the body would have had with what was to be written.
struct Check {
    at: usize,
    need: usize,
}

pub(crate) struct StripPass {
    o: StripOptions,
    title: Option<String>,
    in_title: Option<usize>,
    title_text: String,
    metas: String,
    meta_keys: Vec<String>,
    json_ld: String,
    in_json_ld: Option<usize>,
    json_text: String,
    /// The body as festimap writes it before its last step ([`drop_empty`]):
    /// only ever appended to, so it can be cut back to any check.
    body: String,
    levels: Vec<Level>,
    top: Level,
    /// The UTF-16 units of the title, `<meta>`s and JSON-LD, and of the body.
    head_units: usize,
    body_units: usize,
    elements: Vec<Element>,
    /// Each check that needed more than every one before it. festimap writes
    /// the head first, so the whole head counts against the body from its
    /// start; here a `<title>` or a JSON-LD block late in the page is only
    /// known when it ends, so the body is cut back at the end, to the first
    /// check the whole head fails.
    checks: Vec<Check>,
    truncated: bool,
    /// Text since the last tag or comment: one text node, as htmlparser2 makes it.
    pending: String,
}

impl StripPass {
    pub(crate) fn new(o: StripOptions) -> StripPass {
        StripPass {
            o,
            title: None,
            in_title: None,
            title_text: String::new(),
            metas: String::new(),
            meta_keys: Vec::new(),
            json_ld: String::new(),
            in_json_ld: None,
            json_text: String::new(),
            body: String::new(),
            levels: Vec::new(),
            top: Level::default(),
            head_units: 0,
            body_units: 0,
            elements: Vec::new(),
            checks: Vec::new(),
            truncated: false,
            pending: String::new(),
        }
    }

    /// The text node that ended, written.
    fn flush_text(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let t = collapse(&std::mem::take(&mut self.pending));
        if t.is_empty() {
            return;
        }
        let mut s = String::new();
        escape_text(&cut(&t, self.o.max_text), &mut s);
        let n = units(&s);
        if self.full(n) {
            self.truncated = true;
            return;
        }
        self.body.push_str(&s);
        self.body_units += n;
    }

    /// Whether `reserve` more units would pass the budget (with the head read so far).
    fn full(&mut self, reserve: usize) -> bool {
        let need = self.body_units + reserve;
        if self.checks.last().is_none_or(|c| need > c.need) {
            self.checks.push(Check {
                at: self.body.len(),
                need,
            });
        }
        self.head_units + need > self.o.max_chars
    }

    fn parent(&mut self) -> &mut Level {
        self.levels.last_mut().unwrap_or(&mut self.top)
    }

    /// The comment for a level's collapsed children, written inside it.
    fn flush_collapsed(level: &mut Level, body: &mut String, units: &mut usize) {
        if level.collapsed > 0 {
            let at = body.len();
            let _ = write!(body, "<!-- {} more alike -->", level.collapsed);
            *units += body.len() - at;
        }
        level.collapsed = 0;
    }

    pub(crate) fn finish(mut self) -> Stripped {
        self.flush_text();
        // The first check that fails with the whole head is where festimap stopped:
        // the body up to it, and the end tags of the elements open there.
        let room = self.o.max_chars.saturating_sub(self.head_units);
        let stop = self.checks.partition_point(|c| c.need <= room);
        if let Some(c) = self.checks.get(stop) {
            self.truncated = true;
            let at = c.at;
            let open: Vec<&Element> = self.elements.iter().filter(|e| e.open < at && at <= e.close).collect();
            let mut body = String::with_capacity(at + open.len() * 8);
            body.push_str(&self.body[..at]);
            for e in open.iter().rev() {
                body.push_str("</");
                body.push_str(&self.body[e.open + 1..e.open + 1 + e.name]);
                body.push('>');
            }
            self.body = body;
        } else {
            debug_assert!(!self.truncated);
            let mut top = std::mem::take(&mut self.top);
            Self::flush_collapsed(&mut top, &mut self.body, &mut self.body_units);
        }
        let mut html = String::with_capacity(
            self.title.as_ref().map_or(0, String::len) + self.metas.len() + self.json_ld.len() + self.body.len(),
        );
        if let Some(t) = &self.title {
            html.push_str(t);
        }
        html.push_str(&self.metas);
        html.push_str(&self.json_ld);
        html.push_str(&self.body);
        let html = drop_empty(html, &self.o.drop_empty);
        let tokens = units(&html).div_ceil(3);
        Stripped {
            html,
            tokens,
            truncated: self.truncated,
        }
    }

    fn attributes(&self, el: &crate::tree::Element) -> String {
        let mut s = String::new();
        for (name, value) in el.attrs() {
            let data = self.o.keep_data && name.starts_with("data-");
            if !data && !self.o.keep_attrs.iter().any(|k| k == name) {
                continue;
            }
            if data && units(value) > self.o.max_attr {
                continue;
            }
            s.push(' ');
            s.push_str(name);
            if !value.is_empty() {
                s.push_str("=\"");
                escape_attr(&cut(value, self.o.max_attr), &mut s);
                s.push('"');
            }
        }
        s
    }
}

impl Handler for StripPass {
    fn open(&mut self, tree: &Tree) {
        self.flush_text();
        let d = tree.frames.len() - 1;
        let el = &tree.frames[d].el;
        let name = el.name();
        if self.o.head {
            if name == "title" && self.title.is_none() && self.in_title.is_none() {
                self.in_title = Some(d);
            }
            if name == "meta" {
                if let Some(content) = el.attr("content") {
                    let key = el
                        .attr("property")
                        .or_else(|| el.attr("name"))
                        .unwrap_or("")
                        .to_lowercase();
                    let content = collapse(content);
                    if !key.is_empty() && !content.is_empty() && !self.meta_keys.contains(&key) {
                        if self.o.meta.is_match(&key) {
                            let mut m = String::from("<meta property=\"");
                            escape_attr(&key, &mut m);
                            m.push_str("\" content=\"");
                            escape_attr(&cut(&content, self.o.max_attr), &mut m);
                            m.push_str("\">\n");
                            self.head_units += units(&m);
                            self.metas.push_str(&m);
                        }
                        self.meta_keys.push(key);
                    }
                }
            }
            if name == "script"
                && el
                    .attr("type")
                    .is_some_and(|t| t.trim().to_lowercase().starts_with("application/ld+json"))
            {
                self.in_json_ld = Some(d);
            }
        }

        let parent_skips = self.truncated || self.levels.last().is_some_and(|l| l.skip);
        let dropped = parent_skips
            || self.o.drop.iter().any(|n| n == name)
            || matches!(name, "script" | "style" | "title" | "meta")
            || el.attr("hidden").is_some();
        if dropped {
            self.levels.push(Level {
                skip: true,
                ..Level::default()
            });
            return;
        }
        let mut classes: Vec<&str> = el.attr("class").unwrap_or("").split_ascii_whitespace().collect();
        classes.sort_unstable();
        let key = format!("{name}.{}", classes.join("."));
        let max_alike = self.o.max_alike;
        let parent = self.parent();
        let seen = match parent.alike.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                parent.alike.push((key, 1));
                1
            }
        };
        if seen > max_alike {
            parent.collapsed += 1;
            self.levels.push(Level {
                skip: true,
                ..Level::default()
            });
            return;
        }
        let mut body = std::mem::take(&mut self.body);
        let mut body_units = self.body_units;
        Self::flush_collapsed(self.parent(), &mut body, &mut body_units);
        self.body = body;
        self.body_units = body_units;
        if el.kind == crate::tags::HTML || el.kind == crate::tags::BODY {
            self.levels.push(Level::default());
            return;
        }
        let attrs = self.attributes(el);
        let name_units = units(name);
        let open_units = name_units + units(&attrs) + 2;
        let close_units = if bare(name) { 0 } else { name_units + 3 };
        if self.full(open_units + close_units) {
            self.truncated = true;
            self.levels.push(Level {
                skip: true,
                ..Level::default()
            });
            return;
        }
        let element = (!bare(name)).then(|| {
            self.elements.push(Element {
                open: self.body.len(),
                name: name.len(),
                close: usize::MAX,
            });
            self.elements.len() - 1
        });
        self.body.push('<');
        self.body.push_str(name);
        self.body.push_str(&attrs);
        self.body.push('>');
        self.body_units += open_units;
        self.levels.push(Level {
            element,
            ..Level::default()
        });
    }

    fn close(&mut self, tree: &Tree) {
        self.flush_text();
        let d = tree.frames.len() - 1;
        if self.in_title == Some(d) {
            let mut t = String::from("<title>");
            escape_text(&cut(&collapse(&self.title_text), self.o.max_text), &mut t);
            t.push_str("</title>\n");
            self.head_units += units(&t);
            self.title = Some(t);
            self.in_title = None;
        }
        if self.in_json_ld == Some(d) {
            let json = collapse(&self.json_text);
            let at = self.json_ld.len();
            self.json_ld.push_str("<script type=\"application/ld+json\">");
            self.json_ld.push_str(&cut(&json, self.o.max_json_ld));
            self.json_ld.push_str("</script>\n");
            self.head_units += units(&self.json_ld[at..]);
            self.json_text.clear();
            self.in_json_ld = None;
        }
        let Some(mut level) = self.levels.pop() else { return };
        if level.skip {
            return;
        }
        if !self.truncated {
            Self::flush_collapsed(&mut level, &mut self.body, &mut self.body_units);
        }
        if let Some(i) = level.element {
            let name = tree.frames[d].el.name();
            self.elements[i].close = self.body.len();
            self.body.push_str("</");
            self.body.push_str(name);
            self.body.push('>');
            self.body_units += units(name) + 3;
        }
    }

    fn text(&mut self, _: &Tree, text: &Text<'_>) {
        let skipped = self.truncated || self.levels.last().is_some_and(|l| l.skip);
        if skipped && self.in_title.is_none() && self.in_json_ld.is_none() {
            return;
        }
        let text = text.get();
        if self.in_title.is_some() {
            self.title_text.push_str(text);
        }
        if self.in_json_ld.is_some() {
            self.json_text.push_str(text);
        }
        if self.truncated || self.levels.last().is_some_and(|l| l.skip) {
            return;
        }
        self.pending.push_str(text);
    }

    fn markup(&mut self, _: &Tree, raw: &[u8]) {
        // A comment ends a text node; so do `<?…>` and `</ …>`, which htmlparser2 reads as comments.
        let bogus_end = raw.starts_with(b"</") && !raw.get(2).is_some_and(u8::is_ascii_alphabetic);
        if raw.starts_with(b"<!") || raw.starts_with(b"<?") || bogus_end {
            self.flush_text();
        }
    }
}
