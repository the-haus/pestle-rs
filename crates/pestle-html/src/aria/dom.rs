//! The page as its accessibility tree reads it, kept as a flat table of
//! elements and text: per element its tag, the attributes ARIA reads and
//! what its markup says of its rendering (hidden, inline, a pointer).
//! No other attribute, comment or script text is kept.
//!
//! The table follows the tree a browser builds where it matters here:
//! `<html>` and `<body>` are one root (what comes after `</body>` is in it),
//! what may not be in `<head>` leaves it for the body, and rows written
//! straight in a `<table>` get the `<tbody>` browsers make up.

use super::css::Cascaded;
use crate::tree::Element;

/// No node.
pub(crate) const NONE: u32 = u32::MAX;

/// Hidden for ARIA (Playwright's `isElementHiddenForAria`).
pub(crate) const HIDDEN: u32 = 1;
/// Has a box, as far as the markup tells (`isElementVisible`, sizes unknown).
pub(crate) const BOX: u32 = 1 << 1;
/// `display: inline`.
pub(crate) const INLINE: u32 = 1 << 2;
/// `cursor: pointer`, inherited.
pub(crate) const POINTER: u32 = 1 << 3;
/// `pointer-events: none`, inherited.
pub(crate) const NO_POINTER_EVENTS: u32 = 1 << 4;
/// Inside an `<svg>` (`ownerSVGElement`).
pub(crate) const IN_SVG: u32 = 1 << 5;
/// `aria-hidden="true"` on itself.
pub(crate) const ARIA_HIDDEN: u32 = 1 << 6;
/// `aria-label`, `aria-labelledby`, `aria-roledescription` or another global ARIA attribute is present.
pub(crate) const HAS_LABEL: u32 = 1 << 7;
pub(crate) const HAS_LABELLEDBY: u32 = 1 << 8;
pub(crate) const HAS_ROLEDESCRIPTION: u32 = 1 << 9;
pub(crate) const HAS_GLOBAL: u32 = 1 << 10;
/// A text node.
pub(crate) const TEXT: u32 = 1 << 11;
/// A `<tbody>` the markup did not write.
pub(crate) const VIRTUAL: u32 = 1 << 12;
/// Concealed by an inline style or a class meant to hide (only with `concealed`).
pub(crate) const CONCEALED: u32 = 1 << 13;
/// Laid out (takes room, seen or not): not `display: none`, not in content that is not rendered.
pub(crate) const LAID_OUT: u32 = 1 << 14;
/// A comment: kept as an empty text node, for it counts among an element's child nodes.
const COMMENT: u32 = 1 << 15;

// What an element hands down to its content.
const NONE_TREE: u32 = 1 << 16;
const ARIA_HIDDEN_TREE: u32 = 1 << 17;
const UNRENDERED_TREE: u32 = 1 << 18;
const INVISIBLE: u32 = 1 << 19;
const SKIP_CONTENT: u32 = 1 << 20;
const IN_SELECT: u32 = 1 << 21;
/// A closed `<details>` (its content but its first `<summary>` is not rendered), or `content-visibility: hidden`.
pub(crate) const HIDES_CONTENT: u32 = 1 << 22;
const CONCEALED_TREE: u32 = 1 << 23;
/// A `<details>` whose first `<summary>` has been seen.
const SUMMARY_SEEN: u32 = 1 << 24;
/// A flex or grid container: what is in it is blockified.
const FLEX: u32 = 1 << 25;
/// Inside a `<select>` drawn as a drop-down (its options are in a popup, not laid out).
const IN_DROPDOWN: u32 = 1 << 26;
/// An inline style sets its width or height to zero.
pub(crate) const ZERO_SIZE: u32 = 1 << 27;

macro_rules! tags {
    ($($id:ident = $name:literal),* $(,)?) => {
        /// The tags the accessibility tree tells apart.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Tag { Other, $($id),* }

        impl Tag {
            pub(crate) fn of(name: &str) -> Tag {
                match name { $($name => Tag::$id,)* _ => Tag::Other }
            }
        }
    };
}

tags! {
    A = "a", Address = "address", Area = "area", Article = "article", Aside = "aside", Audio = "audio",
    Base = "base", Basefont = "basefont", Blockquote = "blockquote", Body = "body", Br = "br", Button = "button",
    Canvas = "canvas", Caption = "caption", Center = "center", ClipPath = "clippath", Code = "code", Col = "col",
    Colgroup = "colgroup", Datalist = "datalist", Dd = "dd", Defs = "defs", Del = "del", Desc = "desc",
    Details = "details", Dfn = "dfn", Dialog = "dialog", Dir = "dir", Div = "div", Dl = "dl", Dt = "dt",
    Em = "em", Fieldset = "fieldset", Figcaption = "figcaption", Figure = "figure", Filter = "filter",
    Footer = "footer", Form = "form", Frame = "frame", Frameset = "frameset", H1 = "h1", H2 = "h2", H3 = "h3",
    H4 = "h4", H5 = "h5", H6 = "h6", Head = "head", Header = "header", Hgroup = "hgroup", Hr = "hr",
    Html = "html", Iframe = "iframe", Img = "img", Input = "input", Ins = "ins", Keygen = "keygen",
    Label = "label", Legend = "legend", Li = "li", LinearGradient = "lineargradient", Link = "link",
    Listing = "listing", Main = "main", Mark = "mark", Marker = "marker", Marquee = "marquee", Mask = "mask",
    Math = "math", Menu = "menu", Meta = "meta", Metadata = "metadata", Meter = "meter", Nav = "nav",
    Noembed = "noembed", Noframes = "noframes", Noscript = "noscript", Ol = "ol", Optgroup = "optgroup",
    Option = "option", Output = "output", P = "p", Param = "param", Pattern = "pattern",
    Plaintext = "plaintext", Pre = "pre", Progress = "progress", RadialGradient = "radialgradient",
    Rp = "rp", Rt = "rt", Ruby = "ruby", Script = "script", Search = "search", Section = "section",
    Select = "select", Slot = "slot", Source = "source", Strong = "strong", Style = "style", Sub = "sub",
    Summary = "summary", Sup = "sup", Svg = "svg", Symbol = "symbol", Table = "table", Tbody = "tbody",
    Td = "td", Template = "template", Textarea = "textarea", Tfoot = "tfoot", Th = "th", Thead = "thead",
    Text = "text", Time = "time", Title = "title", Tr = "tr", Track = "track", Ul = "ul", Video = "video",
    Object = "object", Xmp = "xmp",
}

impl Tag {
    /// `display: none` in browsers' own style sheet (attributes aside).
    fn hidden_by_default(self) -> bool {
        use Tag::*;
        matches!(
            self,
            Area | Base
                | Basefont
                | Datalist
                | Head
                | Link
                | Meta
                | Noembed
                | Noframes
                | Param
                | Rp
                | Script
                | Style
                | Template
                | Title
                | Source
                | Track
        )
    }

    /// Not `display: inline` in browsers' own style sheet (block, list item, table parts, inline blocks).
    fn block_by_default(self) -> bool {
        use Tag::*;
        matches!(
            self,
            Address
                | Article
                | Aside
                | Blockquote
                | Body
                | Button
                | Caption
                | Center
                | Col
                | Colgroup
                | Dd
                | Details
                | Dialog
                | Dir
                | Div
                | Dl
                | Dt
                | Fieldset
                | Figcaption
                | Figure
                | Footer
                | Form
                | Frame
                | Frameset
                | H1
                | H2
                | H3
                | H4
                | H5
                | H6
                | Header
                | Hgroup
                | Hr
                | Html
                | Input
                | Keygen
                | Legend
                | Li
                | Listing
                | Main
                | Marquee
                | Math
                | Menu
                | Meter
                | Nav
                | Ol
                | Optgroup
                | Option
                | P
                | Plaintext
                | Pre
                | Progress
                | Rt
                | Ruby
                | Search
                | Section
                | Select
                | Summary
                | Table
                | Tbody
                | Td
                | Textarea
                | Tfoot
                | Th
                | Thead
                | Tr
                | Ul
                | Xmp
        )
    }

    /// SVG elements that are never rendered, so have no box.
    fn unrendered_in_svg(self) -> bool {
        use Tag::*;
        matches!(
            self,
            Title
                | Desc
                | Metadata
                | Defs
                | Symbol
                | ClipPath
                | Mask
                | LinearGradient
                | RadialGradient
                | Pattern
                | Marker
                | Filter
                | Style
                | Script
        )
    }

    /// Content that is not part of the accessibility tree (`isElementIgnoredForAria`).
    pub(crate) fn ignored(self) -> bool {
        matches!(self, Tag::Style | Tag::Script | Tag::Noscript | Tag::Template)
    }

    /// What may stay in `<head>`; anything else starts the body.
    pub(crate) fn in_head(self) -> bool {
        use Tag::*;
        matches!(
            self,
            Title | Meta | Link | Style | Script | Base | Basefont | Noscript | Template | Noframes
        )
    }

    pub(crate) fn labelable(self) -> bool {
        use Tag::*;
        matches!(self, Button | Input | Meter | Output | Progress | Select | Textarea)
    }
}

/// The attributes ARIA reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attr {
    Role,
    AriaLabel,
    AriaLabelledby,
    AriaOwns,
    AriaChecked,
    AriaDisabled,
    AriaExpanded,
    AriaLevel,
    AriaPressed,
    AriaSelected,
    AriaInvalid,
    AriaValuetext,
    AriaValuenow,
    Id,
    Href,
    Alt,
    Title,
    Type,
    Value,
    Placeholder,
    For,
    List,
    Hidden,
    Disabled,
    Checked,
    Selected,
    Multiple,
    Size,
    Open,
    Scope,
    Tabindex,
    Summary,
    XlinkTitle,
    Class,
    AriaHidden,
    Style,
    Controls,
    Min,
    Max,
    Width,
    Height,
    Src,
    Srcset,
}

impl Attr {
    fn of(name: &str) -> Option<Attr> {
        Some(match name {
            "role" => Attr::Role,
            "aria-label" => Attr::AriaLabel,
            "aria-labelledby" => Attr::AriaLabelledby,
            "aria-owns" => Attr::AriaOwns,
            "aria-checked" => Attr::AriaChecked,
            "aria-disabled" => Attr::AriaDisabled,
            "aria-expanded" => Attr::AriaExpanded,
            "aria-level" => Attr::AriaLevel,
            "aria-pressed" => Attr::AriaPressed,
            "aria-selected" => Attr::AriaSelected,
            "aria-invalid" => Attr::AriaInvalid,
            "aria-valuetext" => Attr::AriaValuetext,
            "aria-valuenow" => Attr::AriaValuenow,
            "id" => Attr::Id,
            "href" => Attr::Href,
            "alt" => Attr::Alt,
            "title" => Attr::Title,
            "type" => Attr::Type,
            "value" => Attr::Value,
            "placeholder" => Attr::Placeholder,
            "for" => Attr::For,
            "list" => Attr::List,
            "hidden" => Attr::Hidden,
            "disabled" => Attr::Disabled,
            "checked" => Attr::Checked,
            "selected" => Attr::Selected,
            "multiple" => Attr::Multiple,
            "size" => Attr::Size,
            "open" => Attr::Open,
            "scope" => Attr::Scope,
            "tabindex" => Attr::Tabindex,
            "summary" => Attr::Summary,
            "xlink:title" => Attr::XlinkTitle,
            "class" => Attr::Class,
            "min" => Attr::Min,
            "max" => Attr::Max,
            "width" => Attr::Width,
            "src" => Attr::Src,
            "srcset" => Attr::Srcset,
            "height" => Attr::Height,
            _ => return None,
        })
    }
}

/// The global ARIA attributes besides `aria-label`, `aria-labelledby` and `aria-roledescription`
/// (Playwright's `kGlobalAriaAttributes`).
fn is_global_aria(name: &str) -> bool {
    matches!(
        name,
        "aria-atomic"
            | "aria-busy"
            | "aria-controls"
            | "aria-current"
            | "aria-describedby"
            | "aria-details"
            | "aria-dropeffect"
            | "aria-flowto"
            | "aria-grabbed"
            | "aria-hidden"
            | "aria-keyshortcuts"
            | "aria-live"
            | "aria-owns"
            | "aria-relevant"
    )
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub parent: u32,
    pub first: u32,
    pub last: u32,
    pub next: u32,
    pub tag: Tag,
    pub flags: u32,
    /// An element's name, or a text node's text, in [`Dom::strs`].
    pub span: (u32, u32),
    /// Its attributes: where they start in [`Dom::attrs`], and how many.
    pub attrs: (u32, u32),
}

/// An inline `style`, as far as rendering goes.
#[derive(Default)]
struct Style<'a> {
    display: Option<&'a str>,
    visibility: Option<&'a str>,
    cursor: Option<&'a str>,
    pointer_events: Option<&'a str>,
    content_visibility: Option<&'a str>,
    position: Option<&'a str>,
    float: Option<&'a str>,
    zero_size: bool,
    concealed: bool,
}

/// What an inline style says (the last declaration of a property wins).
fn style(s: &str, concealed: bool) -> Style<'_> {
    let mut st = Style::default();
    for decl in s.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let value = value.strip_suffix("!important").map_or(value, str::trim_end);
        let prop = prop.trim();
        if prop.eq_ignore_ascii_case("display") {
            st.display = Some(value);
        } else if prop.eq_ignore_ascii_case("visibility") {
            st.visibility = Some(value);
        } else if prop.eq_ignore_ascii_case("cursor") {
            st.cursor = Some(value);
        } else if prop.eq_ignore_ascii_case("pointer-events") {
            st.pointer_events = Some(value);
        } else if prop.eq_ignore_ascii_case("content-visibility") {
            st.content_visibility = Some(value);
        } else if prop.eq_ignore_ascii_case("position") {
            st.position = Some(value);
        } else if prop.eq_ignore_ascii_case("float") {
            st.float = Some(value);
        } else if prop.eq_ignore_ascii_case("height") || prop.eq_ignore_ascii_case("width") {
            let n = value.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
            st.zero_size = n.parse::<f64>().is_ok_and(|n| n == 0.0);
        } else if concealed && conceals(prop, value) {
            st.concealed = true;
        }
    }
    st
}

/// A declaration that hides text from sight while keeping it in the page: no size, no opacity,
/// far off screen, clipped away.
fn conceals(prop: &str, value: &str) -> bool {
    let p = prop.to_ascii_lowercase();
    let v = value.to_ascii_lowercase();
    let zero = |v: &str| {
        let n = v.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        !n.is_empty() && n.parse::<f64>().is_ok_and(|n| n == 0.0)
    };
    let far = |v: &str| {
        let n = v.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        n.parse::<f64>().is_ok_and(|n| n <= -999.0)
    };
    match p.as_str() {
        "font-size" | "max-height" | "max-width" => zero(&v),
        "opacity" => v.parse::<f64>().is_ok_and(|o| o <= 0.01),
        "left" | "top" | "right" | "margin-left" | "margin-top" | "text-indent" => far(&v),
        "clip" => v.starts_with("rect(0") && !v.contains("auto"),
        "clip-path" => v.contains("inset(50%") || v.contains("inset(100%") || v == "circle(0)",
        "transform" => v.contains("scale(0)") || v.contains("scale(0,") || v.contains("translatex(-999"),
        _ => false,
    }
}

/// Classes that hide an element in the common CSS frameworks (Tailwind, Bootstrap, Bulma,
/// Foundation, UIkit), unless a responsive class shows it again.
fn hidden_by_class(class: &str) -> bool {
    let mut hidden = false;
    for c in class.split_ascii_whitespace() {
        match c {
            "hidden" | "d-none" | "is-hidden" | "hide" | "invisible" | "uk-hidden" | "display-none" => hidden = true,
            _ => {
                // `md:block`, `d-md-block`, `lg:flex`: shown at some width.
                let shows = c.contains(':') && !c.ends_with(":hidden")
                    || c.starts_with("d-") && c != "d-none" && !c.ends_with("-none");
                if shows {
                    return false;
                }
            }
        }
    }
    hidden
}

/// The page's elements and text.
pub(crate) struct Dom {
    pub nodes: Vec<Node>,
    pub attrs: Vec<(Attr, u32, u32)>,
    pub strs: String,
    /// Also hide what inline styles and hiding classes conceal.
    concealed: bool,
    /// The page has an address (or an absolute `<base href>`): relative URLs load.
    pub based: bool,
}

/// The root: `<html>` and `<body>` as one.
pub(crate) const ROOT: u32 = 0;

impl Dom {
    pub(crate) fn new(concealed: bool) -> Dom {
        let mut d = Dom {
            nodes: Vec::with_capacity(256),
            attrs: Vec::with_capacity(512),
            strs: String::with_capacity(4096),
            concealed,
            based: false,
        };
        d.strs.push_str("body");
        d.nodes.push(Node {
            parent: NONE,
            first: NONE,
            last: NONE,
            next: NONE,
            tag: Tag::Body,
            flags: BOX | LAID_OUT,
            span: (0, 4),
            attrs: (0, 0),
        });
        d
    }

    #[inline]
    pub(crate) fn str(&self, (a, b): (u32, u32)) -> &str {
        &self.strs[a as usize..b as usize]
    }

    #[inline]
    pub(crate) fn attr(&self, n: u32, a: Attr) -> Option<&str> {
        let (start, len) = self.nodes[n as usize].attrs;
        self.attrs[start as usize..(start + len) as usize]
            .iter()
            .find(|(k, _, _)| *k == a)
            .map(|&(_, s, e)| &self.strs[s as usize..e as usize])
    }

    #[inline]
    pub(crate) fn has(&self, n: u32, a: Attr) -> bool {
        self.attr(n, a).is_some()
    }

    #[inline]
    pub(crate) fn tag(&self, n: u32) -> Tag {
        self.nodes[n as usize].tag
    }

    #[inline]
    pub(crate) fn flag(&self, n: u32, f: u32) -> bool {
        self.nodes[n as usize].flags & f != 0
    }

    #[inline]
    pub(crate) fn is_text(&self, n: u32) -> bool {
        self.flag(n, TEXT)
    }

    /// An element's name as written (lower case).
    pub(crate) fn name(&self, n: u32) -> &str {
        self.str(self.nodes[n as usize].span)
    }

    pub(crate) fn text(&self, n: u32) -> &str {
        self.str(self.nodes[n as usize].span)
    }

    /// The children of `n`, in order.
    pub(crate) fn children(&self, n: u32) -> Children<'_> {
        Children {
            dom: self,
            at: self.nodes[n as usize].first,
        }
    }

    /// The element children of `n`.
    pub(crate) fn elements(&self, n: u32) -> impl Iterator<Item = u32> + '_ {
        self.children(n).filter(|&c| !self.is_text(c))
    }

    pub(crate) fn parent(&self, n: u32) -> Option<u32> {
        let p = self.nodes[n as usize].parent;
        (p != NONE).then_some(p)
    }

    /// The elements inside `n`, in document order (not `n`).
    pub(crate) fn descendants(&self, n: u32) -> Descendants<'_> {
        Descendants {
            dom: self,
            root: n,
            at: self.nodes[n as usize].first,
        }
    }

    /// The previous element sibling.
    pub(crate) fn prev_element(&self, n: u32) -> Option<u32> {
        let p = self.parent(n)?;
        let mut prev = None;
        for c in self.elements(p) {
            if c == n {
                return prev;
            }
            prev = Some(c);
        }
        None
    }

    pub(crate) fn next_element(&self, n: u32) -> Option<u32> {
        let mut at = self.nodes[n as usize].next;
        while at != NONE {
            if !self.is_text(at) {
                return Some(at);
            }
            at = self.nodes[at as usize].next;
        }
        None
    }

    /// All text inside `n` (`textContent`).
    pub(crate) fn text_content(&self, n: u32, out: &mut String) {
        // Document order without recursion: down to the first child, else on to the next sibling
        // of the nearest ancestor below `n`.
        let mut at = self.nodes[n as usize].first;
        while at != NONE {
            let node = &self.nodes[at as usize];
            if node.flags & TEXT != 0 {
                out.push_str(self.str(node.span));
            }
            at = if node.first != NONE {
                node.first
            } else {
                let mut up = at;
                loop {
                    let u = &self.nodes[up as usize];
                    if u.next != NONE {
                        break u.next;
                    }
                    if u.parent == n || u.parent == NONE {
                        break NONE;
                    }
                    up = u.parent;
                }
            };
        }
    }

    fn append(&mut self, parent: u32, mut node: Node) -> u32 {
        let id = self.nodes.len() as u32;
        node.parent = parent;
        let p = &mut self.nodes[parent as usize];
        let last = p.last;
        p.last = id;
        if last == NONE {
            p.first = id;
        } else {
            self.nodes[last as usize].next = id;
        }
        self.nodes.push(node);
        id
    }

    /// Where an element opening in `parent` goes: into a made-up `<tbody>` when it is a row in a
    /// table.
    pub(crate) fn place(&mut self, parent: u32, tag: Tag) -> u32 {
        let p = self.tag(parent);
        if tag == Tag::Tr && p == Tag::Table && !self.flag(parent, IN_SVG) {
            let last = self.nodes[parent as usize].last;
            if last != NONE && self.flag(last, VIRTUAL) {
                return last;
            }
            let start = self.strs.len() as u32;
            self.strs.push_str("tbody");
            let flags = self.inherit(parent) | (self.nodes[parent as usize].flags & (BOX | LAID_OUT)) | VIRTUAL;
            return self.append(
                parent,
                Node {
                    parent,
                    first: NONE,
                    last: NONE,
                    next: NONE,
                    tag: Tag::Tbody,
                    flags,
                    span: (start, start + 5),
                    attrs: (self.attrs.len() as u32, 0),
                },
            );
        }
        parent
    }

    /// What `parent` hands down to an element in it.
    fn inherit(&self, parent: u32) -> u32 {
        let pf = self.nodes[parent as usize].flags;
        let mut f = pf
            & (NONE_TREE
                | ARIA_HIDDEN_TREE
                | UNRENDERED_TREE
                | INVISIBLE
                | IN_SELECT
                | POINTER
                | NO_POINTER_EVENTS
                | CONCEALED_TREE);
        if pf & IN_SVG != 0 || self.tag(parent) == Tag::Svg {
            f |= IN_SVG;
        }
        if self.tag(parent) == Tag::Select && pf & IN_SVG == 0 {
            f |= IN_SELECT;
            let listbox = self.attr(parent, Attr::Multiple).is_some()
                || self
                    .attr(parent, Attr::Size)
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .is_some_and(|s| s > 1);
            if !listbox {
                f |= IN_DROPDOWN;
            }
        }
        f |= pf & IN_DROPDOWN;
        f
    }

    /// Whether `n` is or is inside an `<svg>`.
    pub(crate) fn in_svg(&self, n: u32) -> bool {
        self.flag(n, IN_SVG) || self.tag(n) == Tag::Svg
    }

    /// Whether an element opening in `parent` gets no node with its content (that of `<script>`,
    /// `<style>`, `<template>`, `<noscript>`).
    pub(crate) fn skips(&self, parent: u32) -> bool {
        self.flag(parent, SKIP_CONTENT)
    }

    /// Adds an element; `parent` is where [`Dom::place`] put it, `sheet` what the page's style
    /// sheets give it.
    pub(crate) fn element(&mut self, parent: u32, el: &Element, sheet: &Cascaded) -> u32 {
        self.element_as(parent, el, Tag::of(el.name()), sheet)
    }

    /// [`Dom::element`] when its tag is known.
    pub(crate) fn element_as(&mut self, parent: u32, el: &Element, tag: Tag, sheet: &Cascaded) -> u32 {
        let name = el.name();
        let start = self.strs.len() as u32;
        self.strs.push_str(name);
        let span = (start, self.strs.len() as u32);
        let (attrs, global) = self.read_attrs(el);
        let inherited = self.inherit(parent);
        let first_summary = tag == Tag::Summary
            && self.tag(parent) == Tag::Details
            && !self.flag(parent, SUMMARY_SEEN)
            && !self.flag(parent, IN_SVG);
        if first_summary {
            self.nodes[parent as usize].flags |= SUMMARY_SEEN;
        }
        let hides_content = !first_summary && self.flag(parent, HIDES_CONTENT);
        let mut flags = self.render_flags(inherited | global, tag, attrs, hides_content, sheet);
        if self.flag(parent, FLEX) {
            flags &= !INLINE;
        }
        self.append(
            parent,
            Node {
                parent,
                first: NONE,
                last: NONE,
                next: NONE,
                tag,
                flags,
                span,
                attrs,
            },
        )
    }

    /// The first `<body>`'s attributes, for the root.
    pub(crate) fn body(&mut self, el: &Element, sheet: &Cascaded) {
        if self.nodes[0].attrs.1 > 0 {
            return;
        }
        let (attrs, global) = self.read_attrs(el);
        self.nodes[0].attrs = attrs;
        self.nodes[0].flags = self.render_flags(global, Tag::Body, attrs, false, sheet);
    }

    /// A run of text in `parent` (merged with a run just before it).
    pub(crate) fn add_text(&mut self, parent: u32, text: &str) {
        if text.is_empty() {
            return;
        }
        let last = self.nodes[parent as usize].last;
        if last != NONE
            && self.is_text(last)
            && !self.flag(last, COMMENT)
            && self.nodes[last as usize].span.1 as usize == self.strs.len()
        {
            self.strs.push_str(text);
            self.nodes[last as usize].span.1 = self.strs.len() as u32;
            return;
        }
        let start = self.strs.len() as u32;
        self.strs.push_str(text);
        self.append(
            parent,
            Node {
                parent,
                first: NONE,
                last: NONE,
                next: NONE,
                tag: Tag::Other,
                flags: TEXT,
                span: (start, self.strs.len() as u32),
                attrs: (0, 0),
            },
        );
    }

    /// A comment in `parent`.
    pub(crate) fn add_comment(&mut self, parent: u32) {
        let at = self.strs.len() as u32;
        self.append(
            parent,
            Node {
                parent,
                first: NONE,
                last: NONE,
                next: NONE,
                tag: Tag::Other,
                flags: TEXT | COMMENT,
                span: (at, at),
                attrs: (0, 0),
            },
        );
    }

    /// What an element's tag, attributes and inline style say of its rendering, on top of what
    /// its parent hands down (`flags`).
    fn render_flags(&self, mut flags: u32, tag: Tag, attrs: (u32, u32), hides_content: bool, sheet: &Cascaded) -> u32 {
        let list = &self.attrs[attrs.0 as usize..(attrs.0 + attrs.1) as usize];
        let get = |a: Attr| {
            list.iter()
                .find(|(k, _, _)| *k == a)
                .map(|&(_, s, e)| &self.strs[s as usize..e as usize])
        };
        let in_svg = flags & IN_SVG != 0;
        let mut st = get(Attr::Style).map(|s| style(s, self.concealed)).unwrap_or_default();
        // An important rule beats the inline style, which beats the other rules.
        let pick = |rule: &Option<(String, bool)>, inline: Option<&'_ str>| -> Option<String> {
            match (rule, inline) {
                (Some((v, true)), _) => Some(v.clone()),
                (_, Some(i)) => Some(i.to_owned()),
                (Some((v, false)), None) => Some(v.clone()),
                (None, None) => None,
            }
        };
        let display = pick(&sheet.display, st.display);
        let visibility = pick(&sheet.visibility, st.visibility);
        let content_visibility = pick(&sheet.content_visibility, st.content_visibility);
        let cursor = pick(&sheet.cursor, st.cursor);
        let pointer_events = pick(&sheet.pointer_events, st.pointer_events);
        st.display = display.as_deref();
        st.visibility = visibility.as_deref();
        st.content_visibility = content_visibility.as_deref();
        st.cursor = cursor.as_deref();
        st.pointer_events = pointer_events.as_deref();
        let hidden_attr = get(Attr::Hidden);
        let until_found = hidden_attr.is_some_and(|h| h.trim().eq_ignore_ascii_case("until-found"));

        // display
        let mut none = if in_svg {
            false
        } else {
            tag.hidden_by_default()
                || hidden_attr.is_some() && !until_found
                || tag == Tag::Dialog && get(Attr::Open).is_none()
                || tag == Tag::Audio && get(Attr::Controls).is_none()
        };
        let mut inline = if in_svg {
            tag != Tag::Text
        } else {
            !tag.block_by_default()
        };
        if let Some(d) = st.display {
            let d = d.to_ascii_lowercase();
            none = d == "none";
            inline = d == "inline";
            if matches!(d.as_str(), "flex" | "inline-flex" | "grid" | "inline-grid") {
                flags |= FLEX;
            }
        }
        // Absolutely positioned and floated boxes are blocks.
        let out_of_flow = st
            .position
            .is_some_and(|p| p.eq_ignore_ascii_case("absolute") || p.eq_ignore_ascii_case("fixed"))
            || st.float.is_some_and(|f| !f.eq_ignore_ascii_case("none"));
        if out_of_flow {
            inline = false;
        }
        if st.zero_size && !inline {
            flags |= ZERO_SIZE;
        }
        if tag == Tag::Input && get(Attr::Type).is_some_and(|t| t.trim().eq_ignore_ascii_case("hidden")) {
            none = true;
        }
        if none {
            flags |= NONE_TREE;
            inline = false;
        }
        if inline {
            flags |= INLINE;
        }

        // visibility (inherited)
        if let Some(v) = st.visibility {
            let v = v.to_ascii_lowercase();
            if v == "hidden" || v == "collapse" {
                flags |= INVISIBLE;
            } else if v == "visible" {
                flags &= !INVISIBLE;
            }
        }

        // aria-hidden
        let aria_hidden = list
            .iter()
            .any(|&(k, s, e)| k == Attr::AriaHidden && self.strs[s as usize..e as usize].eq_ignore_ascii_case("true"));
        if aria_hidden {
            flags |= ARIA_HIDDEN | ARIA_HIDDEN_TREE;
        }

        // no box: unrendered SVG, content a closed <details> or `content-visibility: hidden` holds
        // What a drop-down <select> holds is in its popup, not laid out.
        let unrendered = hides_content || in_svg && tag.unrendered_in_svg() || flags & IN_DROPDOWN != 0;
        if unrendered {
            flags |= UNRENDERED_TREE;
        }
        let content_hidden = tag == Tag::Details && get(Attr::Open).is_none() && !in_svg
            || until_found
            || st.content_visibility.is_some_and(|c| c.eq_ignore_ascii_case("hidden"));
        if content_hidden {
            flags |= HIDES_CONTENT;
        }

        // cursor and pointer events (inherited)
        match st.cursor {
            Some(c) => {
                if c.eq_ignore_ascii_case("pointer") {
                    flags |= POINTER;
                } else {
                    flags &= !POINTER;
                }
            }
            None if matches!(tag, Tag::A | Tag::Area) && get(Attr::Href).is_some() => flags |= POINTER,
            None if tag == Tag::Input
                && !in_svg
                && get(Attr::Type).is_some_and(|t| t.eq_ignore_ascii_case("image")) =>
            {
                flags |= POINTER
            }
            // Browsers' own style sheet sets the cursor of form controls and labels.
            None if !in_svg
                && matches!(
                    tag,
                    Tag::Label | Tag::Button | Tag::Select | Tag::Option | Tag::Optgroup | Tag::Input | Tag::Textarea
                ) =>
            {
                flags &= !POINTER
            }
            None => {}
        }
        if let Some(p) = st.pointer_events {
            if p.eq_ignore_ascii_case("none") {
                flags |= NO_POINTER_EVENTS;
            } else {
                flags &= !NO_POINTER_EVENTS;
            }
        }

        if self.concealed && (st.concealed || get(Attr::Class).is_some_and(hidden_by_class)) {
            flags |= CONCEALED_TREE;
        }

        if tag.ignored() {
            flags |= SKIP_CONTENT;
        }

        let option_in_select = tag == Tag::Option && flags & IN_SELECT != 0;
        let unrendered = flags & (NONE_TREE | UNRENDERED_TREE) != 0;
        let invisible = flags & INVISIBLE != 0;
        if tag.ignored()
            || flags & (NONE_TREE | ARIA_HIDDEN_TREE | CONCEALED_TREE) != 0
            || !option_in_select && (unrendered || invisible)
        {
            flags |= HIDDEN;
        }
        if !unrendered && !invisible && flags & CONCEALED_TREE == 0 {
            flags |= BOX;
        }
        if !unrendered {
            flags |= LAID_OUT;
        }
        if flags & CONCEALED_TREE != 0 {
            flags |= CONCEALED;
        }
        flags
    }

    fn read_attrs(&mut self, el: &Element) -> ((u32, u32), u32) {
        let first = self.attrs.len() as u32;
        let mut global = 0;
        for (n, v) in el.attrs() {
            let a = match n {
                "aria-label" => {
                    global |= HAS_LABEL;
                    Some(Attr::AriaLabel)
                }
                "aria-labelledby" => {
                    global |= HAS_LABELLEDBY;
                    Some(Attr::AriaLabelledby)
                }
                "aria-roledescription" => {
                    global |= HAS_ROLEDESCRIPTION;
                    None
                }
                "style" => Some(Attr::Style),
                "aria-hidden" => {
                    global |= HAS_GLOBAL;
                    Some(Attr::AriaHidden)
                }
                "controls" => Some(Attr::Controls),
                _ => {
                    if is_global_aria(n) {
                        global |= HAS_GLOBAL;
                    }
                    Attr::of(n)
                }
            };
            if let Some(a) = a {
                let s = self.strs.len() as u32;
                self.strs.push_str(v);
                self.attrs.push((a, s, self.strs.len() as u32));
            }
        }
        ((first, self.attrs.len() as u32 - first), global)
    }
}

/// The children of a node.
pub(crate) struct Children<'a> {
    dom: &'a Dom,
    at: u32,
}

impl Iterator for Children<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.at == NONE {
            return None;
        }
        let n = self.at;
        self.at = self.dom.nodes[n as usize].next;
        Some(n)
    }
}

/// The elements inside a node, in document order.
pub(crate) struct Descendants<'a> {
    dom: &'a Dom,
    root: u32,
    at: u32,
}

impl Iterator for Descendants<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        loop {
            if self.at == NONE {
                return None;
            }
            let n = self.at;
            let node = &self.dom.nodes[n as usize];
            // Next in document order: first child, else the next sibling of the nearest ancestor below the root.
            self.at = if node.first != NONE {
                node.first
            } else {
                let mut up = n;
                loop {
                    let u = &self.dom.nodes[up as usize];
                    if u.next != NONE {
                        break u.next;
                    }
                    if u.parent == self.root || u.parent == NONE {
                        break NONE;
                    }
                    up = u.parent;
                }
            };
            if !self.dom.is_text(n) {
                return Some(n);
            }
        }
    }
}
