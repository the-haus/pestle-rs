//! The element tree as htmlparser2 (version 12, HTML mode) builds it, kept
//! as no more than the stack of open elements: a start tag closes the
//! elements it implies closed (`<li>` an open `<li>`, `<div>` an open `<p>`,
//! `<td>` an open `<td>`, …, only while they are the innermost), void
//! elements hold nothing, `/>` closes only inside `<svg>` and `<math>`, an
//! end tag closes up to the innermost open element of its name (and is
//! ignored when there is none, but `</p>` and `</br>`, which make the
//! element), a second `<form>` is ignored. No `<html>`, `<head>`, `<body>`
//! or `<tbody>` is made up. That is the tree css-select queries in
//! htmlparser2's DOM, so selectors match here what they match there.
//!
//! Each open element knows its position among its siblings (for
//! `:nth-child` and the like); when a selector needs them, each also keeps
//! its earlier siblings (for `+` and `~`).

use std::borrow::Cow;
use std::cell::OnceCell;
use std::rc::Rc;

use smallvec::SmallVec;

use crate::decode::text_of;
use crate::tokenizer::TokenSink;

/// An element: its name (lower case) and attributes, in source order, held
/// in one buffer (the name, then each attribute's name and value).
#[derive(Clone, Debug)]
pub struct Element {
    /// The name and attributes when owned: the name, then each attribute's name and value.
    buf: String,
    /// When borrowed: the start tag in the input it was read from (null when owned). Offsets
    /// below are into this region, or into `buf`.
    ptr: *const u8,
    len: u32,
    name: (u32, u32),
    /// A hash of the name ([`tag_hash`]): tag comparisons check it first.
    hash: u32,
    /// Which tag the tree rules know it as ([`crate::tags`]), or none.
    pub(crate) kind: u8,
    /// Per attribute: where its name starts and ends, and where its value starts and ends.
    attrs: SmallVec<[(u32, u32, u32, u32); 2]>,
}

impl Default for Element {
    fn default() -> Element {
        Element {
            buf: String::new(),
            ptr: std::ptr::null(),
            len: 0,
            name: (0, 0),
            hash: 0,
            kind: 0,
            attrs: SmallVec::new(),
        }
    }
}

/// Short names compared inline (a library call costs more than the comparison).
#[inline(always)]
pub(crate) fn eq_short(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && (a.len() > 16 && a == b || a.iter().zip(b).all(|(x, y)| x == y))
}

/// FNV-1a of a tag name: what [`Element::hash`] and tag tests compare first.
#[inline]
pub fn tag_hash(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in name.as_bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

impl Element {
    /// An element of this name, without attributes.
    pub fn new(name: &str) -> Element {
        Element::begin(name.to_owned())
    }

    /// An element whose name is all of `buf`; attributes are appended to the buffer and ended with [`Element::end_attr`].
    pub(crate) fn begin(buf: String) -> Element {
        let name = (0, buf.len() as u32);
        let hash = tag_hash(&buf);
        let kind = crate::tags::kind(&buf, hash);
        Element {
            buf,
            name,
            hash,
            kind,
            ..Element::default()
        }
    }

    /// An element read in place: `tag` is its start tag in the input, the name and the
    /// attributes' spans are offsets into it, all of them valid UTF-8, the names lower case
    /// and the values without references. It points into the input until [`Element::own`].
    pub(crate) fn borrowed(tag: &[u8], name: (u32, u32), attrs: SmallVec<[(u32, u32, u32, u32); 2]>) -> Element {
        // SAFETY: the caller checked that the tag is valid UTF-8 (and split it at ASCII bytes).
        let n = unsafe { std::str::from_utf8_unchecked(&tag[name.0 as usize..name.1 as usize]) };
        let hash = tag_hash(n);
        let kind = crate::tags::kind(n, hash);
        Element {
            buf: String::new(),
            ptr: tag.as_ptr(),
            len: tag.len() as u32,
            name,
            hash,
            kind,
            attrs,
        }
    }

    /// Whether it still points into the input it was read from.
    pub(crate) fn is_borrowed(&self) -> bool {
        !self.ptr.is_null()
    }

    /// Copies what it points to into its own buffer (before that input goes away).
    pub(crate) fn own(&mut self) {
        if !self.is_borrowed() {
            return;
        }
        let mut owned = Element::begin(self.name().to_owned());
        for (n, v) in self.attrs() {
            let start = owned.buf.len();
            owned.buf.push_str(n);
            let end = owned.buf.len();
            owned.buf.push_str(v);
            owned
                .attrs
                .push((start as u32, end as u32, end as u32, owned.buf.len() as u32));
        }
        owned.buf.shrink_to_fit();
        *self = owned;
    }

    #[inline]
    fn data(&self) -> &[u8] {
        if self.ptr.is_null() {
            self.buf.as_bytes()
        } else {
            // SAFETY: a borrowed element is owned (`own`) before the input it points into is
            // released: at the end of every chunk, unless the whole input outlives the pass.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len as usize) }
        }
    }

    #[inline]
    fn str(&self, (a, b): (u32, u32)) -> &str {
        // SAFETY: spans are cut from valid UTF-8 at ASCII bytes (owned: a `String`).
        unsafe { std::str::from_utf8_unchecked(&self.data()[a as usize..b as usize]) }
    }

    /// Whether its name is `name`, whose [`tag_hash`] is `hash`.
    #[inline]
    pub fn is(&self, name: &str, hash: u32) -> bool {
        self.hash == hash && eq_short(self.name().as_bytes(), name.as_bytes())
    }

    pub(crate) fn buf_mut(&mut self) -> &mut String {
        &mut self.buf
    }

    pub(crate) fn buf_capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Its buffer, emptied, for the next element (it is not read again).
    pub(crate) fn take_buf(&mut self) -> String {
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        buf
    }

    /// Its buffer, emptied, for the next element.
    pub(crate) fn into_buf(self) -> String {
        let mut buf = self.buf;
        buf.clear();
        buf
    }

    /// Ends the attribute whose name is `buf[start..name_end]` and value the rest; a
    /// second of a name is dropped (htmlparser2 keeps the first).
    pub(crate) fn end_attr(&mut self, start: usize, name_end: usize) {
        let name = &self.buf[start..name_end];
        if self
            .attrs
            .iter()
            .any(|&(a, b, _, _)| &self.buf[a as usize..b as usize] == name)
        {
            self.buf.truncate(start);
            return;
        }
        self.attrs
            .push((start as u32, name_end as u32, name_end as u32, self.buf.len() as u32));
    }

    #[inline]
    pub fn name(&self) -> &str {
        self.str(self.name)
    }

    /// The same element under another name (`<image>` read as `<img>`).
    pub(crate) fn renamed(&self, name: &str) -> Element {
        let mut e = Element::new(name);
        for (n, v) in self.attrs() {
            let start = e.buf.len();
            e.buf.push_str(n);
            let end = e.buf.len();
            e.buf.push_str(v);
            e.end_attr(start, end);
        }
        e
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|&&(a, b, _, _)| eq_short(self.str((a, b)).as_bytes(), name.as_bytes()))
            .map(|&(_, _, c, d)| self.str((c, d)))
    }

    /// Every attribute, in source order.
    pub fn attrs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attrs
            .iter()
            .map(|&(a, b, c, d)| (self.str((a, b)), self.str((c, d))))
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_ascii_whitespace().any(|c| c == class))
    }
}

/// A run of text, decoded only when a handler reads it.
pub struct Text<'a> {
    raw: &'a [u8],
    references: bool,
    valid: bool,
    decoded: OnceCell<Cow<'a, str>>,
}

impl<'a> Text<'a> {
    pub(crate) fn new(raw: &'a [u8], references: bool, valid: bool) -> Text<'a> {
        Text {
            raw,
            references,
            valid,
            decoded: OnceCell::new(),
        }
    }

    /// The text, references decoded.
    pub fn get(&self) -> &str {
        self.decoded
            .get_or_init(|| text_of(self.raw, self.references, self.valid))
    }
}

/// The element children of an element (or of the document) seen so far.
#[derive(Debug, Default)]
pub struct Children {
    pub count: u32,
    /// Counts per name and the children themselves, only when a selector needs them.
    more: Option<Box<More>>,
}

#[derive(Debug, Default)]
struct More {
    types: Vec<(String, u32)>,
    list: Vec<Rc<Element>>,
}

impl Children {
    /// How many children so far have this name.
    pub fn of_type(&self, name: &str) -> u32 {
        self.more
            .as_ref()
            .and_then(|m| m.types.iter().find(|(n, _)| n == name))
            .map_or(0, |&(_, c)| c)
    }

    /// The children so far, when sibling selectors keep them.
    pub fn list(&self) -> &[Rc<Element>] {
        self.more.as_ref().map_or(&[], |m| &m.list)
    }

    fn add(&mut self, el: &Element, keep: bool, types: bool) -> (u32, u32) {
        self.count += 1;
        if !keep && !types {
            return (self.count, 0);
        }
        let more = self.more.get_or_insert_with(Box::default);
        let of_type = if !types {
            0
        } else {
            match more.types.iter_mut().find(|(n, _)| n == el.name()) {
                Some((_, c)) => {
                    *c += 1;
                    *c
                }
                None => {
                    more.types.push((el.name().to_owned(), 1));
                    1
                }
            }
        };
        if keep {
            let mut copy = el.clone();
            copy.own();
            more.list.push(Rc::new(copy));
        }
        (self.count, of_type)
    }
}

/// An open element.
#[derive(Debug)]
pub struct Frame {
    pub el: Element,
    /// Its position among its parent's element children, and among those of its name (1-based).
    pub index: u32,
    pub type_index: u32,
    pub children: Children,
    /// How many `<template>` elements it is or is inside.
    pub templates: u32,
}

/// The open elements, outermost first, and the document's own children.
#[derive(Debug, Default)]
pub struct Tree {
    pub frames: Vec<Frame>,
    pub top: Children,
    keep_siblings: bool,
    keep_types: bool,
}

impl Tree {
    /// The children of the element at `frame` (`None`: the document's).
    pub fn children_of(&self, frame: Option<usize>) -> &Children {
        match frame {
            Some(i) => &self.frames[i].children,
            None => &self.top,
        }
    }

    /// The innermost open element.
    pub fn current(&self) -> Option<&Frame> {
        self.frames.last()
    }

    /// Whether the element at `frame` is inside a `<template>`'s content as
    /// seen from `scope` (`None`: the document): css-select does not search
    /// there, as `querySelectorAll` does not. A template itself can match,
    /// and a template scope's content is searched.
    pub fn in_template(&self, frame: usize, scope: Option<usize>) -> bool {
        let f = &self.frames[frame];
        let above = f.templates - u32::from(f.el.kind == t::TEMPLATE);
        above > scope.map_or(0, |s| self.frames[s].templates)
    }
}

/// What a pass over the tree does.
pub trait Handler {
    /// An element opened: the innermost frame.
    fn open(&mut self, tree: &Tree);
    /// An element closes: the innermost frame, still on the stack.
    fn close(&mut self, tree: &Tree);
    fn text(&mut self, tree: &Tree, text: &Text<'_>);
    /// The raw markup of each token, just before its effect (a start tag
    /// after the closes it implies): what an outer-HTML capture copies.
    fn markup(&mut self, tree: &Tree, raw: &[u8]);
    /// Whether text (and its markup) is wanted now; when not, text is not handed over.
    fn wants_text(&self) -> bool {
        true
    }
}

/// How deep elements nest at most; deeper start tags are ignored (their text is kept).
pub const DEFAULT_MAX_DEPTH: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Foreign {
    None,
    Svg,
    Math,
}

/// Counts of one pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeStats {
    pub elements: u64,
    pub max_depth: usize,
    /// Start tags ignored past the depth limit.
    pub too_deep: u64,
}

/// Builds the tree from tokens and tells a [`Handler`].
pub struct Builder<H> {
    pub tree: Tree,
    pub handler: H,
    pub stats: TreeStats,
    foreign: Vec<Foreign>,
    max_depth: usize,
    /// Buffers of closed elements, for the next ones.
    pool: Vec<String>,
    /// The frames below this one own their elements (nothing in them points into the input).
    owned: usize,
}

use crate::tags as t;

/// The kinds a start tag of this kind closes while they are the innermost.
fn implies_close(kind: u8) -> &'static [u8] {
    const FORM: &[u8] = &[
        t::INPUT,
        t::OPTION,
        t::OPTGROUP,
        t::SELECT,
        t::BUTTON,
        t::DATALIST,
        t::TEXTAREA,
    ];
    const P: &[u8] = &[t::P];
    const HEADING: &[u8] = &[t::H1, t::H2, t::H3, t::H4, t::H5, t::H6, t::P];
    match kind {
        t::TR => &[t::TR, t::TH, t::TD],
        t::TH => &[t::TH],
        t::TD => &[t::THEAD, t::TH, t::TD],
        t::BODY => &[t::HEAD, t::LINK, t::SCRIPT],
        t::A => &[t::A],
        t::LI => &[t::LI],
        t::P => P,
        t::H1 | t::H2 | t::H3 | t::H4 | t::H5 | t::H6 => HEADING,
        t::SELECT | t::INPUT | t::OUTPUT | t::BUTTON | t::DATALIST | t::TEXTAREA => FORM,
        t::OPTION => &[t::OPTION],
        t::OPTGROUP => &[t::OPTGROUP, t::OPTION],
        t::DD | t::DT => &[t::DD, t::DT],
        t::ADDRESS
        | t::ARTICLE
        | t::ASIDE
        | t::BLOCKQUOTE
        | t::DETAILS
        | t::DIV
        | t::DL
        | t::FIELDSET
        | t::FIGCAPTION
        | t::FIGURE
        | t::FOOTER
        | t::FORM
        | t::HEADER
        | t::HR
        | t::MAIN
        | t::NAV
        | t::OL
        | t::PRE
        | t::SECTION
        | t::TABLE
        | t::UL => P,
        t::RT | t::RP => &[t::RT, t::RP],
        t::TBODY | t::TFOOT => &[t::THEAD, t::TBODY],
        _ => &[],
    }
}

/// Elements that never hold anything, by kind.
fn is_void_kind(kind: u8) -> bool {
    matches!(
        kind,
        t::AREA
            | t::BASE
            | t::BASEFONT
            | t::BR
            | t::COL
            | t::COMMAND
            | t::EMBED
            | t::FRAME
            | t::HR
            | t::IMG
            | t::INPUT
            | t::ISINDEX
            | t::KEYGEN
            | t::LINK
            | t::META
            | t::PARAM
            | t::SOURCE
            | t::TRACK
            | t::WBR
    )
}

/// Elements that never hold anything.
pub fn is_void(name: &str) -> bool {
    is_void_kind(t::kind(name, tag_hash(name)))
}

fn foreign_context(kind: u8) -> Option<Foreign> {
    match kind {
        t::SVG => Some(Foreign::Svg),
        t::MATH => Some(Foreign::Math),
        t::MI | t::MO | t::MN | t::MS | t::MTEXT | t::ANNOTATION_XML | t::FOREIGNOBJECT | t::DESC | t::TITLE => {
            Some(Foreign::None)
        }
        _ => None,
    }
}

impl<H: Handler> Builder<H> {
    /// `keep_siblings`: each element's earlier siblings (for `+`, `~`); `keep_types`: counts per name (for `*-of-type`).
    pub fn new(handler: H, keep_siblings: bool, keep_types: bool, max_depth: usize) -> Builder<H> {
        Builder {
            tree: Tree {
                keep_siblings,
                keep_types,
                ..Tree::default()
            },
            handler,
            stats: TreeStats::default(),
            foreign: vec![Foreign::None],
            max_depth,
            pool: Vec::new(),
            owned: 0,
        }
    }

    fn push(&mut self, el: Element) {
        let keep = self.tree.keep_siblings;
        let types = self.tree.keep_types;
        let parent = match self.tree.frames.last_mut() {
            Some(f) => &mut f.children,
            None => &mut self.tree.top,
        };
        let (index, type_index) = parent.add(&el, keep, types);
        let templates = self.tree.frames.last().map_or(0, |f| f.templates) + u32::from(el.kind == t::TEMPLATE);
        self.tree.frames.push(Frame {
            el,
            index,
            type_index,
            children: Children::default(),
            templates,
        });
        self.stats.elements += 1;
        self.stats.max_depth = self.stats.max_depth.max(self.tree.frames.len());
    }

    /// Opens and closes an element at once (a void one, `</p>`, `</br>`).
    fn empty(&mut self, el: Element) {
        self.push(el);
        self.handler.open(&self.tree);
        self.handler.close(&self.tree);
        if let Some(f) = self.tree.frames.pop() {
            self.recycle(f.el);
        }
    }

    fn pop(&mut self) {
        self.handler.close(&self.tree);
        let Some(f) = self.tree.frames.last_mut() else { return };
        if foreign_context(f.el.kind).is_some() && self.foreign.len() > 1 {
            self.foreign.pop();
        }
        // The buffer of an owned element is kept for the next one; the frame is dropped in place.
        if f.el.buf_capacity() > 0 && self.pool.len() < 256 {
            let buf = f.el.take_buf();
            self.pool.push(buf);
        }
        let n = self.tree.frames.len() - 1;
        self.tree.frames.truncate(n);
        self.owned = self.owned.min(n);
    }

    fn recycle(&mut self, el: Element) {
        if el.buf_capacity() > 0 && self.pool.len() < 256 {
            self.pool.push(el.into_buf());
        }
    }

    /// Every element still open, closed (the end of the input).
    pub fn finish(&mut self) {
        while !self.tree.frames.is_empty() {
            self.pop();
        }
    }
}

impl<H: Handler> TokenSink for Builder<H> {
    fn buffer(&mut self) -> String {
        self.pool.pop().unwrap_or_default()
    }

    fn chunk_end(&mut self) {
        let from = self.owned.min(self.tree.frames.len());
        for f in &mut self.tree.frames[from..] {
            f.el.own();
        }
        self.owned = self.tree.frames.len();
    }

    fn start_tag(&mut self, el: Element, self_closing: bool, raw: &[u8]) {
        let el = if el.kind == t::IMAGE && !self.in_foreign() {
            el.renamed("img")
        } else {
            el
        };
        if el.kind == t::FORM && self.tree.frames.iter().any(|f| f.el.kind == t::FORM) {
            self.handler.markup(&self.tree, raw);
            return;
        }
        let closes = implies_close(el.kind);
        if !closes.is_empty() {
            while self.tree.frames.last().is_some_and(|f| closes.contains(&f.el.kind)) {
                self.pop();
            }
        }
        self.handler.markup(&self.tree, raw);
        if is_void_kind(el.kind) {
            self.empty(el);
            return;
        }
        if self.tree.frames.len() >= self.max_depth {
            self.stats.too_deep += 1;
            return;
        }
        let context = foreign_context(el.kind);
        self.push(el);
        if let Some(c) = context {
            self.foreign.push(c);
        }
        self.handler.open(&self.tree);
        if self_closing && self.in_foreign() {
            self.pop();
        }
    }

    fn end_tag(&mut self, name: &str, raw: &[u8]) {
        self.handler.markup(&self.tree, raw);
        let hash = tag_hash(name);
        let mut kind = t::kind(name, hash);
        let (name, hash) = if kind == t::IMAGE && !self.in_foreign() {
            kind = t::IMG;
            ("img", tag_hash("img"))
        } else {
            (name, hash)
        };
        if is_void_kind(kind) {
            if kind == t::BR {
                self.empty(Element::new("br"));
            }
            return;
        }
        match self.tree.frames.iter().rposition(|f| f.el.is(name, hash)) {
            Some(at) => {
                while self.tree.frames.len() > at {
                    self.pop();
                }
            }
            None if kind == t::P => self.empty(Element::new("p")),
            None => {}
        }
    }

    fn text(&mut self, raw: &[u8], references: bool, valid: bool) {
        if !self.handler.wants_text() {
            return;
        }
        self.handler.markup(&self.tree, raw);
        self.handler.text(&self.tree, &Text::new(raw, references, valid));
    }

    fn comment(&mut self, raw: &[u8]) {
        self.handler.markup(&self.tree, raw);
    }

    fn in_foreign(&self) -> bool {
        self.foreign.last().is_some_and(|f| *f != Foreign::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::Tokenizer;

    /// The tree printed as nested tags, the way htmlparser2's DOM serialises.
    #[derive(Default)]
    struct Print(String);

    impl Handler for Print {
        fn open(&mut self, tree: &Tree) {
            self.0.push_str(&format!("<{}>", tree.current().unwrap().el.name()));
        }
        fn close(&mut self, tree: &Tree) {
            self.0.push_str(&format!("</{}>", tree.current().unwrap().el.name()));
        }
        fn text(&mut self, _: &Tree, text: &Text<'_>) {
            self.0.push_str(text.get());
        }
        fn markup(&mut self, _: &Tree, _: &[u8]) {}
    }

    fn tree(html: &str) -> String {
        let mut b = Builder::new(Print::default(), false, false, DEFAULT_MAX_DEPTH);
        let mut t = Tokenizer::new(crate::tokenizer::AttrFilter::all());
        t.write(html.as_bytes(), &mut b);
        t.end(&mut b);
        b.finish();
        b.handler.0
    }

    #[test]
    fn implied_closes() {
        assert_eq!(tree("<ul><li>a<li>b</ul>"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(tree("<p>a<div>b</div>"), "<p>a</p><div>b</div>");
        assert_eq!(tree("<p><span>a<div>b</div>"), "<p><span>a<div>b</div></span></p>");
        assert_eq!(
            tree("<table><tr><td>a<td>b<tr><td>c</table>"),
            "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>"
        );
        assert_eq!(
            tree("<dl><dt>a<dd>b<dt>c</dl>"),
            "<dl><dt>a</dt><dd>b</dd><dt>c</dt></dl>"
        );
        assert_eq!(tree("<a>x<a>y"), "<a>x</a><a>y</a>");
        assert_eq!(tree("<h1>a<h2>b"), "<h1>a</h1><h2>b</h2>");
    }

    #[test]
    fn end_tags_void_and_foreign() {
        assert_eq!(tree("<div><span>a</div>b"), "<div><span>a</span></div>b");
        assert_eq!(tree("a</span>b</p>c</br>"), "ab<p></p>c<br></br>");
        assert_eq!(tree("<img src=x><br/><div/>x"), "<img></img><br></br><div>x</div>");
        assert_eq!(
            tree("<svg><path/><g/></svg><b/>x"),
            "<svg><path></path><g></g></svg><b>x</b>"
        );
        assert_eq!(tree("<form><form><i>x</i></form></form>"), "<form><i>x</i></form>");
        assert_eq!(tree("<image>"), "<img></img>");
    }

    #[test]
    fn depth_is_bounded() {
        let mut b = Builder::new(Print::default(), false, false, 3);
        let mut t = Tokenizer::new(crate::tokenizer::AttrFilter::all());
        t.write(b"<a><b><c><d>x</d></c></b></a>", &mut b);
        t.end(&mut b);
        b.finish();
        assert_eq!(b.handler.0, "<a><b><c>x</c></b></a>");
        assert_eq!(b.stats.too_deep, 1);
    }
}
