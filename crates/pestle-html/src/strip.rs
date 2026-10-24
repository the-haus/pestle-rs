//! A page stripped down for a reader with a budget (a model's prompt), as
//! festimap's `cleanForLearn` does, while it streams: the `<title>`, the
//! OpenGraph and description `<meta>`s and the JSON-LD (cut) up front, then
//! the body without scripts, styles, forms, media and comments; only the
//! attributes a selector can name (and `data-*`) kept; text runs cut; more
//! than a few alike siblings (a listing's 200 cards) collapsed to
//! `<!-- N more alike -->`; wrappers left empty dropped; the whole stopped
//! at the budget, open tags closed.

use regex::Regex;

use crate::skeleton::DEFAULT_IGNORE;
use crate::text::{collapse, escape_attr, escape_text};
use crate::tree::{Handler, Text, Tree};

/// Options of the strip pass (festimap's defaults).
#[derive(Clone, Debug)]
pub struct StripOptions {
    /// The output's budget, in characters (festimap: 12k tokens, 3 characters each).
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
    /// Its estimated tokens: 3 characters each.
    pub tokens: usize,
    /// The budget cut it short.
    pub truncated: bool,
}

/// Elements written without an end tag.
fn bare(name: &str) -> bool {
    matches!(name, "area" | "br" | "col" | "embed" | "hr" | "img" | "meta" | "wbr")
}

/// At most `max` characters, `…` when cut.
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", s[..at].trim_end()),
        None => s.to_owned(),
    }
}

#[derive(Default)]
struct Level {
    /// Its tag was written (not dropped, collapsed, or `<html>`/`<body>`).
    written: bool,
    /// Nothing inside is written.
    skip: bool,
    /// Where its start tag begins in the body, and where it ends.
    mark: usize,
    open_end: usize,
    close: String,
    /// It may go when empty.
    droppable: bool,
    /// Something was written inside it (even if dropped again): festimap's
    /// single-pass regex keeps `<div></div>` once its empty `<span>` is gone.
    wrote: bool,
    alike: Vec<(String, usize)>,
    collapsed: usize,
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
    body: String,
    levels: Vec<Level>,
    top: Level,
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
        if self.full(s.len()) {
            self.truncated = true;
            return;
        }
        self.parent().wrote = true;
        self.body.push_str(&s);
    }

    fn length(&self) -> usize {
        self.title.as_ref().map_or(0, String::len) + self.metas.len() + self.json_ld.len() + self.body.len()
    }

    fn full(&self, reserve: usize) -> bool {
        self.length() + reserve > self.o.max_chars
    }

    fn parent(&mut self) -> &mut Level {
        self.levels.last_mut().unwrap_or(&mut self.top)
    }

    /// The comment for a level's collapsed children, written inside it.
    fn flush_collapsed(level: &mut Level, body: &mut String) {
        if level.collapsed > 0 {
            body.push_str(&format!("<!-- {} more alike -->", level.collapsed));
            level.wrote = true;
        }
        level.collapsed = 0;
    }

    pub(crate) fn finish(mut self) -> Stripped {
        self.flush_text();
        if !self.truncated {
            let mut top = std::mem::take(&mut self.top);
            Self::flush_collapsed(&mut top, &mut self.body);
        }
        let mut html = String::with_capacity(self.length());
        if let Some(t) = &self.title {
            html.push_str(t);
        }
        html.push_str(&self.metas);
        html.push_str(&self.json_ld);
        html.push_str(&self.body);
        let tokens = html.chars().count().div_ceil(3);
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
            if data && value.chars().count() > self.o.max_attr {
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
        Self::flush_collapsed(self.parent(), &mut body);
        self.body = body;
        if el.kind == crate::tags::HTML || el.kind == crate::tags::BODY {
            self.levels.push(Level::default());
            return;
        }
        let attrs = self.attributes(el);
        let open = format!("<{name}{attrs}>");
        let close = if bare(name) {
            String::new()
        } else {
            format!("</{name}>")
        };
        if self.full(open.len() + close.len()) {
            self.truncated = true;
            self.levels.push(Level {
                skip: true,
                ..Level::default()
            });
            return;
        }
        let droppable = self.o.drop_empty.iter().any(|n| n == name)
            && (attrs.is_empty() || {
                // Exactly ` class="…"`, as festimap's regex reads it.
                attrs
                    .strip_prefix(" class=\"")
                    .is_some_and(|rest| rest.find('"').is_some_and(|q| q + 1 == rest.len()))
            });
        self.parent().wrote = true;
        let mark = self.body.len();
        self.body.push_str(&open);
        self.levels.push(Level {
            written: true,
            mark,
            open_end: self.body.len(),
            close,
            droppable,
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
            self.title = Some(t);
            self.in_title = None;
        }
        if self.in_json_ld == Some(d) {
            let json = collapse(&self.json_text);
            self.json_ld.push_str("<script type=\"application/ld+json\">");
            self.json_ld.push_str(&cut(&json, self.o.max_json_ld));
            self.json_ld.push_str("</script>\n");
            self.json_text.clear();
            self.in_json_ld = None;
        }
        let Some(mut level) = self.levels.pop() else { return };
        if level.skip {
            return;
        }
        if !self.truncated {
            Self::flush_collapsed(&mut level, &mut self.body);
        }
        if level.written {
            if level.droppable && !level.wrote {
                debug_assert_eq!(self.body.len(), level.open_end);
                self.body.truncate(level.mark);
            } else {
                self.body.push_str(&level.close);
            }
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
