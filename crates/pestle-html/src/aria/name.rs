//! Accessible names, as Playwright computes them (`getTextAlternativeInternal`,
//! after the W3C accessible name computation): `aria-labelledby`, then
//! embedded control values, `aria-label`, the native text alternatives
//! (labels, `alt`, captions, legends, an SVG's `<title>`), the content for
//! roles named by it, and `title` last. CSS generated content is unknown
//! here and counts as none.

use super::dom::{Attr, Tag, HIDDEN, INLINE, IN_SVG};
use super::role::{js_space, js_trim, Ctx, MAX_DEPTH, NAME_BUDGET, TOTAL_BUDGET};

/// Where the computation is, relative to the element being named.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Outside,
    Itself,
    Descendant,
}

/// Which reference the computation follows, and whether what it refers to is hidden.
#[derive(Clone, Copy)]
struct Walk {
    labelled_by: Option<bool>,
    label: Option<bool>,
    native: Option<bool>,
    target: Target,
    collect: bool,
}

/// A name, and the elements it came from (only when collected).
#[derive(Default)]
pub(crate) struct Name {
    pub text: String,
    pub elements: Vec<u32>,
}

impl Name {
    fn of(text: &str, element: u32, collect: bool) -> Name {
        Name {
            text: text.to_owned(),
            elements: if collect && !text.is_empty() {
                vec![element]
            } else {
                Vec::new()
            },
        }
    }

    fn join(parts: Vec<Name>, sep: &str) -> Name {
        let mut out = Name::default();
        for (i, p) in parts.into_iter().enumerate() {
            if i > 0 {
                out.text.push_str(sep);
            }
            out.text.push_str(&p.text);
            for e in p.elements {
                if !out.elements.contains(&e) {
                    out.elements.push(e);
                }
            }
        }
        out
    }
}

/// Roles that may not be named at all.
const NOT_NAMED: &[&str] = &[
    "caption",
    "code",
    "definition",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "mark",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "suggestion",
    "superscript",
    "term",
    "time",
];

fn named_from_content(role: &str, descendant: bool) -> bool {
    matches!(
        role,
        "button"
            | "cell"
            | "checkbox"
            | "columnheader"
            | "gridcell"
            | "heading"
            | "link"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "option"
            | "radio"
            | "row"
            | "rowheader"
            | "switch"
            | "tab"
            | "tooltip"
            | "treeitem"
    ) || descendant
        && matches!(
            role,
            "" | "caption"
                | "code"
                | "contentinfo"
                | "definition"
                | "deletion"
                | "emphasis"
                | "insertion"
                | "list"
                | "listitem"
                | "mark"
                | "none"
                | "paragraph"
                | "presentation"
                | "region"
                | "row"
                | "rowgroup"
                | "section"
                | "strong"
                | "subscript"
                | "superscript"
                | "table"
                | "term"
                | "time"
        )
}

/// Whether text is plain ASCII with single spaces only between words: nothing to normalize.
fn plain(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() == Some(&b' ') || b.last() == Some(&b' ') {
        return false;
    }
    let mut prev = 0u8;
    for &c in b {
        if !(b' '..0x80).contains(&c) || c == b' ' && prev == b' ' {
            return false;
        }
        prev = c;
    }
    true
}

/// [`flat`] of an owned string, which it returns as it is when there is nothing to do.
pub(crate) fn flat_owned(s: String) -> String {
    if plain(&s) {
        s
    } else {
        flat(&s)
    }
}

/// [`normalize`] of an owned string, which it returns as it is when there is nothing to do.
pub(crate) fn normalize_owned(s: String) -> String {
    if plain(&s) {
        s
    } else {
        normalize(&s)
    }
}

/// Text flattened as names are (`asFlatString`): whitespace collapsed but no-break spaces kept,
/// soft hyphens and zero-width spaces out, trimmed.
pub(crate) fn flat(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        match c {
            '\u{200b}' | '\u{ad}' => {}
            '\u{a0}' => {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
            c if js_space(c) => space = true,
            c => {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
        }
    }
    if space {
        out.push(' ');
    }
    js_trim(&out).to_owned()
}

/// Whitespace as a snapshot shows it (`normalizeWhiteSpace`): soft hyphens and zero-width spaces
/// out, every run of whitespace one space, trimmed.
pub(crate) fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        match c {
            '\u{200b}' | '\u{ad}' => {}
            c if js_space(c) => space = !out.is_empty(),
            c => {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
        }
    }
    out
}

impl Ctx<'_> {
    fn hidden(&self, n: u32) -> bool {
        self.dom.flag(n, HIDDEN)
    }

    fn visit(&mut self, n: u32) {
        self.visited[n as usize] = self.pass;
    }

    fn html(&self, n: u32, tag: Tag) -> bool {
        self.dom.tag(n) == tag && !self.dom.flag(n, IN_SVG)
    }

    /// An element's accessible name, with the elements it came from when `collect`.
    pub(crate) fn accessible_name(&mut self, n: u32, collect: bool) -> Name {
        if self.role(n).is_some_and(|r| NOT_NAMED.contains(&r)) {
            return Name::default();
        }
        self.pass += 1;
        // Past the page's budget, names get only a little.
        self.budget = if self.total > TOTAL_BUDGET { 64 } else { NAME_BUDGET };
        let mut name = self.alternative(
            n,
            Walk {
                labelled_by: None,
                label: None,
                native: None,
                target: Target::Itself,
                collect,
            },
        );
        name.text = flat_owned(std::mem::take(&mut name.text));
        name
    }

    fn labels_name(&mut self, labels: Vec<u32>, w: Walk) -> Name {
        let parts = labels
            .into_iter()
            .map(|l| {
                let hidden = self.hidden(l);
                self.alternative(
                    l,
                    Walk {
                        label: Some(hidden),
                        native: None,
                        labelled_by: None,
                        target: Target::Outside,
                        ..w
                    },
                )
            })
            .filter(|n| !n.text.is_empty())
            .collect();
        Name::join(parts, " ")
    }

    fn labels_of(&self, n: u32) -> Vec<u32> {
        self.labels.get(&n).cloned().unwrap_or_default()
    }

    /// Elements in `n` and in what it owns (`queryInAriaOwned(n, "*")`).
    fn in_owned(&self, n: u32) -> Vec<u32> {
        let d = self.dom;
        let mut out: Vec<u32> = d.descendants(n).collect();
        for o in self.id_refs(d.attr(n, Attr::AriaOwns)) {
            out.push(o);
            out.extend(d.descendants(o));
        }
        out
    }

    fn alternative(&mut self, n: u32, w: Walk) -> Name {
        if self.visited[n as usize] == self.pass || self.budget == 0 || self.depth >= MAX_DEPTH {
            return Name::default();
        }
        self.budget -= 1;
        self.total += 1;
        self.depth += 1;
        let name = super::grow(|| self.alternative_in(n, w));
        self.depth -= 1;
        name
    }

    fn alternative_in(&mut self, n: u32, w: Walk) -> Name {
        let d = self.dom;
        let collect = w.collect;
        let child = Walk {
            target: if w.target == Target::Itself {
                Target::Descendant
            } else {
                w.target
            },
            ..w
        };
        let in_hidden_reference = w.labelled_by == Some(true) || w.label == Some(true) || w.native == Some(true);
        if d.tag(n).ignored() && !d.flag(n, IN_SVG) || !in_hidden_reference && self.hidden(n) {
            self.visit(n);
            return Name::default();
        }

        let labelled_by = d
            .attr(n, Attr::AriaLabelledby)
            .map(|r| self.id_refs(Some(r)))
            .filter(|r| !r.is_empty());
        if w.labelled_by.is_none() {
            if let Some(refs) = &labelled_by {
                let parts = refs
                    .iter()
                    .map(|&r| {
                        let hidden = self.hidden(r);
                        self.alternative(
                            r,
                            Walk {
                                labelled_by: Some(hidden),
                                label: None,
                                native: None,
                                target: Target::Outside,
                                collect,
                            },
                        )
                    })
                    .collect();
                let name = Name::join(parts, " ");
                if !name.text.is_empty() {
                    return name;
                }
            }
        }

        let role = self.role(n).unwrap_or("");
        let tag = d.tag(n);
        let svg = d.flag(n, IN_SVG);
        if w.label.is_some() || w.labelled_by.is_some() || w.target == Target::Descendant {
            let own = labelled_by.as_ref().is_some_and(|r| r.contains(&n));
            if !own {
                match role {
                    "textbox" | "searchbox" => {
                        self.visit(n);
                        if !svg && matches!(tag, Tag::Input | Tag::Textarea) {
                            return Name::of(&self.value(n), n, collect);
                        }
                        let mut t = String::new();
                        d.text_content(n, &mut t);
                        return Name::of(&t, n, collect);
                    }
                    "combobox" | "listbox" => {
                        self.visit(n);
                        let selected: Vec<u32> = if self.html(n, Tag::Select) {
                            let options = self.options(n);
                            let mut s: Vec<u32> =
                                options.iter().copied().filter(|&o| self.option_selected(o)).collect();
                            if s.is_empty() {
                                s.extend(options.first());
                            }
                            s
                        } else {
                            let listbox = if role == "combobox" {
                                let mut found = None;
                                for e in self.in_owned(n) {
                                    if self.role(e) == Some("listbox") {
                                        found = Some(e);
                                        break;
                                    }
                                }
                                found
                            } else {
                                Some(n)
                            };
                            match listbox {
                                Some(l) => {
                                    let mut s = Vec::new();
                                    for e in self.in_owned(l) {
                                        if d.attr(e, Attr::AriaSelected) == Some("true")
                                            && self.role(e) == Some("option")
                                        {
                                            s.push(e);
                                        }
                                    }
                                    s
                                }
                                None => Vec::new(),
                            }
                        };
                        if selected.is_empty() && self.html(n, Tag::Input) {
                            return Name::of(&self.value(n), n, collect);
                        }
                        let parts = selected.into_iter().map(|o| self.alternative(o, child)).collect();
                        return Name::join(parts, " ");
                    }
                    "progressbar" | "scrollbar" | "slider" | "spinbutton" | "meter" => {
                        self.visit(n);
                        let v = d
                            .attr(n, Attr::AriaValuetext)
                            .or_else(|| d.attr(n, Attr::AriaValuenow))
                            .or_else(|| d.attr(n, Attr::Value))
                            .unwrap_or("");
                        return Name::of(v, n, collect);
                    }
                    "menu" => {
                        self.visit(n);
                        return Name::default();
                    }
                    _ => {}
                }
            }
        }

        let aria_label = d.attr(n, Attr::AriaLabel).unwrap_or("");
        if !js_trim(aria_label).is_empty() {
            self.visit(n);
            return Name::of(aria_label, n, collect);
        }

        let presentational = matches!(role, "presentation" | "none");
        if !presentational {
            if let Some(name) = self.native_alternative(n, w, child, labelled_by.is_some()) {
                return name;
            }
        }

        let summary = self.html(n, Tag::Summary) && !presentational;
        if named_from_content(role, w.target == Target::Descendant)
            || summary
            || w.labelled_by.is_some()
            || w.label.is_some()
            || w.native.is_some()
        {
            self.visit(n);
            let mut name = self.content(n, child);
            let empty = if w.target == Target::Itself {
                js_trim(&name.text).is_empty()
            } else {
                name.text.is_empty()
            };
            if !empty {
                if collect && !name.elements.contains(&n) {
                    name.elements.push(n);
                }
                return name;
            }
        }

        if !presentational || self.html(n, Tag::Iframe) || self.html(n, Tag::Frame) {
            self.visit(n);
            let title = d.attr(n, Attr::Title).unwrap_or("");
            if !js_trim(title).is_empty() {
                return Name::of(title, n, collect);
            }
        }
        self.visit(n);
        Name::default()
    }

    /// The text alternatives HTML gives some elements; `None` to go on.
    fn native_alternative(&mut self, n: u32, w: Walk, child: Walk, labelled_by: bool) -> Option<Name> {
        let d = self.dom;
        let collect = w.collect;
        let title = || d.attr(n, Attr::Title).unwrap_or("");
        let first_child = |tag: Tag| d.elements(n).find(|&c| d.tag(c) == tag);
        if d.flag(n, IN_SVG) || d.tag(n) == Tag::Svg {
            self.visit(n);
            if let Some(t) = d
                .elements(n)
                .find(|&c| d.tag(c) == Tag::Title && (d.flag(c, IN_SVG) || d.tag(n) == Tag::Svg))
            {
                let hidden = self.hidden(t);
                return Some(self.alternative(
                    t,
                    Walk {
                        labelled_by: Some(hidden),
                        ..child
                    },
                ));
            }
            if d.tag(n) == Tag::A && d.flag(n, IN_SVG) {
                let t = d.attr(n, Attr::XlinkTitle).unwrap_or("");
                if !js_trim(t).is_empty() {
                    return Some(Name::of(t, n, collect));
                }
            }
            return None;
        }
        match d.tag(n) {
            Tag::Input => {
                let ty = self.input_type(n);
                match ty {
                    "button" | "submit" | "reset" => {
                        self.visit(n);
                        let v = self.value(n);
                        if !js_trim(&v).is_empty() {
                            return Some(Name::of(&v, n, collect));
                        }
                        return Some(match ty {
                            "submit" => Name::of("Submit", n, collect),
                            "reset" => Name::of("Reset", n, collect),
                            _ => Name::of(title(), n, collect),
                        });
                    }
                    "file" => {
                        self.visit(n);
                        let labels = self.labels_of(n);
                        if !labels.is_empty() && w.labelled_by.is_none() {
                            return Some(self.labels_name(labels, w));
                        }
                        return Some(Name::of("Choose File", n, collect));
                    }
                    "image" => {
                        self.visit(n);
                        let labels = self.labels_of(n);
                        if !labels.is_empty() && w.labelled_by.is_none() {
                            return Some(self.labels_name(labels, w));
                        }
                        let alt = d.attr(n, Attr::Alt).unwrap_or("");
                        if !js_trim(alt).is_empty() {
                            return Some(Name::of(alt, n, collect));
                        }
                        if !js_trim(title()).is_empty() {
                            return Some(Name::of(title(), n, collect));
                        }
                        return Some(Name::of("Submit", n, collect));
                    }
                    _ => {}
                }
            }
            Tag::Button if !labelled_by => {
                self.visit(n);
                let labels = self.labels_of(n);
                if !labels.is_empty() {
                    return Some(self.labels_name(labels, w));
                }
            }
            Tag::Output if !labelled_by => {
                self.visit(n);
                let labels = self.labels_of(n);
                if !labels.is_empty() {
                    return Some(self.labels_name(labels, w));
                }
                return Some(Name::of(title(), n, collect));
            }
            _ => {}
        }
        match d.tag(n) {
            Tag::Textarea | Tag::Select | Tag::Input | Tag::Meter | Tag::Progress if !labelled_by => {
                self.visit(n);
                let labels = self.labels_of(n);
                if !labels.is_empty() {
                    return Some(self.labels_name(labels, w));
                }
                let placeholder_ok = d.tag(n) == Tag::Input
                    && matches!(
                        self.input_type(n),
                        "text" | "password" | "number" | "search" | "tel" | "email" | "url"
                    )
                    || d.tag(n) == Tag::Textarea;
                let t = title();
                if !placeholder_ok || !t.is_empty() {
                    return Some(Name::of(t, n, collect));
                }
                return Some(Name::of(d.attr(n, Attr::Placeholder).unwrap_or(""), n, collect));
            }
            tag @ (Tag::Fieldset | Tag::Figure) if !labelled_by => {
                self.visit(n);
                let caption = if tag == Tag::Fieldset {
                    Tag::Legend
                } else {
                    Tag::Figcaption
                };
                if let Some(c) = first_child(caption) {
                    let hidden = self.hidden(c);
                    return Some(self.alternative(
                        c,
                        Walk {
                            native: Some(hidden),
                            ..child
                        },
                    ));
                }
                return Some(Name::of(title(), n, collect));
            }
            Tag::Img | Tag::Area => {
                self.visit(n);
                let alt = d.attr(n, Attr::Alt).unwrap_or("");
                if !js_trim(alt).is_empty() {
                    return Some(Name::of(alt, n, collect));
                }
                return Some(Name::of(title(), n, collect));
            }
            Tag::Table => {
                self.visit(n);
                if let Some(c) = first_child(Tag::Caption) {
                    let hidden = self.hidden(c);
                    return Some(self.alternative(
                        c,
                        Walk {
                            native: Some(hidden),
                            ..child
                        },
                    ));
                }
                let summary = d.attr(n, Attr::Summary).unwrap_or("");
                if !summary.is_empty() {
                    return Some(Name::of(summary, n, collect));
                }
            }
            _ => {}
        }
        None
    }

    /// The text of an element's content (`innerAccumulatedElementText`): block elements and `<br>`
    /// apart by spaces.
    fn content(&mut self, n: u32, w: Walk) -> Name {
        let d = self.dom;
        let mut out = Name::default();
        let add = |out: &mut Name, part: Name, block: bool| {
            if block {
                out.text.push(' ');
            }
            out.text.push_str(&part.text);
            if block {
                out.text.push(' ');
            }
            for e in part.elements {
                if !out.elements.contains(&e) {
                    out.elements.push(e);
                }
            }
        };
        let mut c = d.nodes[n as usize].first;
        while c != super::dom::NONE {
            if d.is_text(c) {
                out.text.push_str(d.text(c));
            } else {
                let part = self.alternative(c, w);
                let block = !d.flag(c, INLINE) || d.tag(c) == Tag::Br;
                add(&mut out, part, block);
            }
            c = d.nodes[c as usize].next;
        }
        for o in self.id_refs(d.attr(n, Attr::AriaOwns)) {
            let part = self.alternative(o, w);
            let block = !d.flag(o, INLINE) || d.tag(o) == Tag::Br;
            add(&mut out, part, block);
        }
        out
    }
}
