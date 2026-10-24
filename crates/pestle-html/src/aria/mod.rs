//! The page's accessibility tree as YAML, the snapshot Playwright's
//! `ariaSnapshot()` gives, from markup alone: roles, accessible names,
//! states, links' URLs, form fields' values, in the same pass as the rest.
//!
//! ```
//! use pestle_html::aria::{snapshot, AriaOptions};
//!
//! let html = br#"<nav aria-label="Main"><a href="/agenda">Agenda</a></nav>
//!   <main><h1>Nora &amp; Co</h1><input aria-label="Tickets" value="2"><button disabled>Buy</button></main>"#;
//! assert_eq!(
//!     snapshot(html, None, AriaOptions::default()).yaml,
//!     r#"- navigation "Main":
//!   - link "Agenda":
//!     - /url: /agenda
//! - main:
//!   - heading "Nora & Co" [level=1]
//!   - textbox "Tickets": "2"
//!   - button "Buy" [disabled]"#
//! );
//! ```
//!
//! Its rules are Playwright's (roles, names and the passes that distill the
//! tree, ported from its injected script), and `tests/aria.rs` holds the
//! output to Playwright's snapshots in `fixtures/aria`. What only a browser
//! knows is read from the markup:
//!
//! - **Hidden**: `hidden`, `aria-hidden`, browsers' own style sheet, inline
//!   styles, and the page's own `<style>` blocks.
//! - **Size**: estimated from content, where Playwright measures layout. In
//!   `ai` mode it decides which elements get a ref.
//! - **Not applied**: style sheets the page links to, CSS generated content,
//!   scripts.
//!
//! Each ref comes with a CSS selector for it.

mod css;
mod dom;
mod name;
mod role;
mod snapshot;

use std::collections::HashMap;
use std::fmt::Write;

use dom::{Dom, Tag, ROOT};

use crate::tree::{Handler, Text, Tree};

/// Which snapshot: Playwright's `"default"` or its `"ai"` one (refs, generic containers kept, name
/// repeats cut).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AriaMode {
    #[default]
    Default,
    Ai,
}

/// Options of the snapshot.
#[derive(Clone, Debug)]
pub struct AriaOptions {
    pub mode: AriaMode,
    /// How deep nodes are written at most (Playwright's `depth`).
    pub depth: Option<usize>,
    /// Apply the page's own `<style>` blocks: their `display`, `visibility`, `content-visibility`,
    /// `cursor` and `pointer-events` rules. Style sheets the page links to are never fetched.
    pub style_blocks: bool,
    /// Also leave out what inline styles conceal from sight (no size, no opacity, far off screen,
    /// clipped) and what the common frameworks' hiding classes hide (`hidden`, `d-none`,
    /// `is-hidden`): text a page hides from people but a model would read.
    pub concealed: bool,
}

impl Default for AriaOptions {
    fn default() -> Self {
        AriaOptions {
            mode: AriaMode::Default,
            depth: None,
            style_blocks: true,
            concealed: false,
        }
    }
}

/// An element a ref stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AriaRef {
    /// `e3`.
    pub id: String,
    /// A CSS selector for the element in the page as browsers build it.
    pub selector: String,
}

/// The snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AriaSnapshot {
    pub yaml: String,
    /// In `ai` mode, each ref and where it is.
    pub refs: Vec<AriaRef>,
}

/// Run `f` with room on the stack: the snapshot walks nested pages by recursion, and grows the
/// stack (on the heap) rather than overflow it.
#[inline]
pub(crate) fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

/// Open elements without a node: content skipped.
const SKIP: u32 = u32::MAX - 1;

pub(crate) struct AriaPass {
    opts: AriaOptions,
    dom: Dom,
    /// Per open element, its node (or [`SKIP`]).
    stack: Vec<u32>,
    /// `<head>`, once there is one.
    head: u32,
    /// The body has begun: what comes now is in it.
    body: bool,
    sheet: css::Sheet,
    /// The `<style>` being read: its depth and text.
    style: Option<(usize, String)>,
    /// Style sheet text read so far.
    css_read: usize,
}

impl AriaPass {
    /// `url` is the page's address, if known.
    pub(crate) fn new(opts: AriaOptions, url: Option<&url::Url>) -> AriaPass {
        let mut dom = Dom::new(opts.concealed);
        dom.based = url.is_some();
        AriaPass {
            dom,
            opts,
            stack: Vec::with_capacity(64),
            head: dom::NONE,
            body: false,
            sheet: css::Sheet::default(),
            style: None,
            css_read: 0,
        }
    }

    fn head(&mut self) -> u32 {
        if self.head == dom::NONE {
            let el = crate::tree::Element::new("head");
            self.head = self.dom.element(ROOT, &el, &css::Cascaded::default());
        }
        self.head
    }

    /// Where an element of this name opening in `parent` goes, as browsers place it.
    fn place(&mut self, parent: u32, tag: Tag) -> u32 {
        let in_head = tag.in_head();
        if parent == ROOT && !self.body {
            if in_head {
                return self.head();
            }
            self.body = true;
        } else if parent == self.head && !in_head {
            self.body = true;
            return ROOT;
        }
        self.dom.place(parent, tag)
    }

    pub(crate) fn finish(self) -> AriaSnapshot {
        let built = snapshot::build(&self.dom, &self.opts);
        let mut ids: HashMap<&str, u32> = HashMap::new();
        let mut index = Vec::new();
        if !built.refs.is_empty() {
            let dom = &self.dom;
            index = vec![0u32; dom.nodes.len()];
            for i in 0..dom.nodes.len() as u32 {
                if dom.is_text(i) {
                    continue;
                }
                if let Some(id) = dom.attr(i, dom::Attr::Id) {
                    *ids.entry(id).or_default() += 1;
                }
                // Each element's place among its parent's element children (`<head>` is not the body's).
                let mut k = 0;
                for c in dom.elements(i) {
                    if i == ROOT && dom.tag(c) == Tag::Head {
                        continue;
                    }
                    k += 1;
                    index[c as usize] = k;
                }
            }
        }
        let refs = built
            .refs
            .into_iter()
            .map(|(id, el)| AriaRef {
                id,
                selector: selector(&self.dom, &ids, &index, el),
            })
            .collect();
        AriaSnapshot { yaml: built.yaml, refs }
    }
}

impl Handler for AriaPass {
    fn open(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        let parent = if depth == 0 { ROOT } else { self.stack[depth - 1] };
        if parent == SKIP || self.dom.skips(parent) {
            self.stack.push(SKIP);
            return;
        }
        let el = &tree.frames[depth].el;
        let sheet = if self.sheet.is_empty() {
            css::Cascaded::default()
        } else {
            self.sheet.cascade(tree, el)
        };
        let id = match el.name() {
            "html" => ROOT,
            "body" if !self.dom.in_svg(parent) => {
                self.body = true;
                self.dom.body(el, &sheet);
                ROOT
            }
            "head" if parent == ROOT && !self.body => {
                if self.head == dom::NONE {
                    self.head = self.dom.element(ROOT, el, &sheet);
                }
                self.head
            }
            name => {
                if name == "base" && el.attr("href").is_some_and(|h| url::Url::parse(h.trim()).is_ok()) {
                    self.dom.based = true;
                }
                if name == "style"
                    && self.opts.style_blocks
                    && !self.dom.in_svg(parent)
                    && css::media_matches(el.attr("media").unwrap_or(""))
                {
                    self.style = Some((depth, String::new()));
                }
                let tag = Tag::of(name);
                let at = self.place(parent, tag);
                self.dom.element_as(at, el, tag, &sheet)
            }
        };
        self.stack.push(id);
    }

    fn close(&mut self, tree: &Tree) {
        self.stack.pop();
        if self.style.as_ref().is_some_and(|(d, _)| *d + 1 == tree.frames.len()) {
            if let Some((_, css)) = self.style.take() {
                self.sheet.add(&css);
            }
        }
    }

    fn text(&mut self, tree: &Tree, text: &Text<'_>) {
        if let Some((d, css)) = &mut self.style {
            if *d + 1 == tree.frames.len() {
                let t = text.get();
                if self.css_read + t.len() <= css::MAX_CSS {
                    self.css_read += t.len();
                    css.push_str(t);
                }
                return;
            }
        }
        let mut parent = self.stack.last().copied().unwrap_or(ROOT);
        if parent == SKIP || self.dom.skips(parent) {
            return;
        }
        let t = text.get();
        if parent == self.head || parent == ROOT && !self.body {
            if t.bytes().all(|b| b.is_ascii_whitespace()) {
                return;
            }
            self.body = true;
            parent = ROOT;
        }
        self.dom.add_text(parent, t);
    }

    fn markup(&mut self, tree: &Tree, raw: &[u8]) {
        // Only comments: every other token comes as an element or text.
        let comment = raw.starts_with(b"<?")
            || raw.starts_with(b"<!") && !raw.get(2..9).is_some_and(|d| d.eq_ignore_ascii_case(b"doctype"));
        if !comment {
            return;
        }
        let parent = self.stack.last().copied().unwrap_or(ROOT);
        let raw_text = tree.current().is_some_and(|f| {
            matches!(
                f.el.name(),
                "textarea" | "title" | "xmp" | "plaintext" | "iframe" | "noembed" | "noframes"
            )
        });
        if parent == SKIP || raw_text || self.dom.skips(parent) || parent == self.head || parent == ROOT && !self.body {
            return;
        }
        self.dom.add_comment(parent);
    }
}

/// A CSS selector for a node: from the nearest ancestor with an id of its own, else from `body`,
/// by `:nth-child()`.
fn selector(dom: &Dom, ids: &HashMap<&str, u32>, index: &[u32], n: u32) -> String {
    let mut path: smallvec::SmallVec<[u32; 32]> = smallvec::SmallVec::new();
    let mut anchor = None;
    let mut at = n;
    while at != ROOT {
        if let Some(id) = dom
            .attr(at, dom::Attr::Id)
            .filter(|id| !id.is_empty() && ids.get(id) == Some(&1))
        {
            anchor = Some(id);
            break;
        }
        path.push(at);
        match dom.parent(at) {
            Some(p) => at = p,
            None => break,
        }
    }
    let mut out = String::with_capacity(8 + path.len() * 20);
    match anchor {
        Some(id) => {
            out.push('#');
            css_escape_into(&mut out, id);
        }
        None => out.push_str("body"),
    }
    for &e in path.iter().rev() {
        out.push_str(" > ");
        let name = dom.name(e);
        if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            out.push_str(name);
        } else {
            css_escape_into(&mut out, name);
        }
        let _ = write!(out, ":nth-child({})", index[e as usize]);
    }
    out
}

/// CSS's `CSS.escape`, appended to `out`.
fn css_escape_into(out: &mut String, s: &str) {
    let first_dash = s.starts_with('-');
    for (i, c) in s.chars().enumerate() {
        match c {
            '\0' => out.push('\u{fffd}'),
            '\u{1}'..='\u{1f}' | '\u{7f}' => {
                let _ = write!(out, "\\{:x} ", c as u32);
            }
            '0'..='9' if i == 0 || i == 1 && first_dash => {
                let _ = write!(out, "\\{:x} ", c as u32);
            }
            '-' if i == 0 && s.len() == 1 => out.push_str("\\-"),
            c if c >= '\u{80}' || c == '-' || c == '_' || c.is_ascii_alphanumeric() => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
}

/// The snapshot of a whole page; `url` is its address, if known.
pub fn snapshot(html: &[u8], url: Option<&str>, options: AriaOptions) -> AriaSnapshot {
    let x = crate::Extractor::new(
        &serde_json::json!({}),
        &Default::default(),
        crate::ExtractorOptions {
            aria: Some(options),
            ..Default::default()
        },
    )
    .expect("an empty spec compiles");
    x.extract(html, url).aria.unwrap_or_default()
}

#[cfg(test)]
mod tests;
