//! The snapshot: Playwright's `generateAriaTree`, its distilling passes
//! and its YAML (`renderAriaSnapshotAsYaml`), on the table [`super::dom`]
//! keeps.

use std::collections::HashSet;
use std::fmt::Write;

use super::dom::{
    Attr, Dom, Tag, ARIA_HIDDEN, BOX, HIDDEN, HIDES_CONTENT, INLINE, IN_SVG, LAID_OUT, NONE, NO_POINTER_EVENTS,
    POINTER, ROOT, ZERO_SIZE,
};
use super::name::normalize_owned;
use super::role::{
    js_number, js_space, Ctx, Invalid, Tri, CHECKED_ROLES, DISABLED_ROLES, EXPANDED_ROLES, INVALID_ROLES, LEVEL_ROLES,
    PRESSED_ROLES, SELECTED_ROLES,
};
use super::{AriaMode, AriaOptions};

enum Child {
    Text(String),
    Node(usize),
}

struct ANode {
    role: &'static str,
    name: String,
    children: Vec<Child>,
    url: Option<String>,
    placeholder: Option<String>,
    aria_hidden: bool,
    checked: Tri,
    disabled: bool,
    expanded: bool,
    invalid: Invalid,
    level: u32,
    pressed: Tri,
    selected: bool,
    /// The focused element (the body, on a page just loaded).
    active: bool,
    /// Its ref number (`e3`), in `ai` mode.
    r#ref: u32,
    el: u32,
    pointer: bool,
    /// The refs of the elements its name came from, but its own.
    name_refs: Vec<u32>,
}

impl ANode {
    fn new(role: &'static str, el: u32) -> ANode {
        ANode {
            role,
            name: String::new(),
            children: Vec::new(),
            url: None,
            placeholder: None,
            aria_hidden: false,
            checked: Tri::False,
            disabled: false,
            expanded: false,
            invalid: Invalid::False,
            level: 0,
            pressed: Tri::False,
            selected: false,
            active: false,
            r#ref: 0,
            el,
            pointer: false,
            name_refs: Vec::new(),
        }
    }

    fn has_props(&self) -> bool {
        self.url.is_some() || self.placeholder.is_some() || self.aria_hidden
    }

    fn push_text(&mut self, t: &str) {
        if let Some(Child::Text(last)) = self.children.last_mut() {
            last.push_str(t);
        } else {
            self.children.push(Child::Text(t.to_owned()));
        }
    }
}

/// What a snapshot gives: the YAML and, in `ai` mode, where each ref is.
pub(crate) struct Built {
    pub yaml: String,
    pub refs: Vec<(String, u32)>,
}

struct Gen<'c, 'd> {
    ctx: &'c mut Ctx<'d>,
    ai: bool,
    nodes: Vec<ANode>,
    seen: Vec<bool>,
    next_ref: u32,
    /// The ref each element got.
    el_ref: Vec<u32>,
    /// Whether each node would have a size.
    sized: Vec<bool>,
    /// How deep the walk is (owned elements nest in their owners).
    depth: u32,
}

pub(crate) fn build(dom: &Dom, opts: &AriaOptions) -> Built {
    let mut ctx = Ctx::new(dom);
    let n = dom.nodes.len();
    let mut g = Gen {
        ctx: &mut ctx,
        ai: opts.mode == AriaMode::Ai,
        nodes: vec![ANode::new("fragment", ROOT)],
        seen: vec![false; n],
        next_ref: 0,
        el_ref: vec![0; n],
        sized: if opts.mode == AriaMode::Ai {
            sizes(dom)
        } else {
            Vec::new()
        },
        depth: 0,
    };
    g.visit(0, ROOT, true);
    let mut d = Distill {
        nodes: &mut g.nodes,
        ai: g.ai,
        ancestors: Vec::new(),
        pending: HashSet::new(),
        max_depth: opts.depth,
        depth: -1,
    };
    d.run();
    let mut yaml = String::new();
    let mut refs = Vec::new();
    let r = Render {
        nodes: &g.nodes,
        depth: opts.depth,
        cursor: g.ai,
    };
    for c in &g.nodes[0].children {
        match c {
            Child::Text(t) => r.text(&mut yaml, t, 0),
            Child::Node(i) => r.node(&mut yaml, *i, 0, g.ai),
        }
    }
    if yaml.ends_with('\n') {
        yaml.pop();
    }
    if g.ai {
        for node in &g.nodes {
            if node.r#ref != 0 {
                refs.push((format!("e{}", node.r#ref), node.el));
            }
        }
        refs.sort_by_key(|(_, el)| *el);
        refs.dedup();
    }
    Built { yaml, refs }
}

impl Gen<'_, '_> {
    fn visit(&mut self, parent: usize, n: u32, parent_visible: bool) {
        if self.seen[n as usize] || self.depth >= super::role::MAX_DEPTH {
            return;
        }
        self.seen[n as usize] = true;
        let dom = self.ctx.dom;
        if dom.is_text(n) {
            if parent_visible && self.nodes[parent].role != "textbox" {
                self.nodes[parent].push_text(dom.text(n));
            }
            return;
        }
        let aria_visible = !dom.flag(n, HIDDEN);
        let visible = aria_visible || self.ai && self.boxed(n);
        if !visible && !self.ai {
            return;
        }
        let owned: Vec<u32> = match dom.attr(n, Attr::AriaOwns) {
            Some(list) => list
                .split(js_space)
                .filter_map(|id| self.ctx.ids.get(id).copied())
                .collect(),
            None => Vec::new(),
        };
        let child = if visible { self.node(n) } else { None };
        let target = match child {
            Some(c) => {
                if dom.flag(n, ARIA_HIDDEN) {
                    self.nodes[c].aria_hidden = true;
                }
                self.nodes[parent].children.push(Child::Node(c));
                c
            }
            None => parent,
        };
        self.depth += 1;
        super::grow(|| self.process(target, n, &owned, visible));
        self.depth -= 1;
        if let Some(c) = child {
            if self.nodes[c].r#ref != 0 {
                let own = self.nodes[c].r#ref;
                let contributors = std::mem::take(&mut self.nodes[c].name_refs);
                let refs = contributors
                    .into_iter()
                    .map(|e| self.el_ref[e as usize])
                    .filter(|&r| r != 0 && r != own)
                    .collect();
                self.nodes[c].name_refs = refs;
            }
        }
    }

    fn process(&mut self, target: usize, n: u32, owned: &[u32], parent_visible: bool) {
        let dom = self.ctx.dom;
        let block = !dom.flag(n, INLINE) || dom.tag(n) == Tag::Br;
        if block {
            self.nodes[target].push_text(" ");
        }
        let mut c = dom.nodes[n as usize].first;
        while c != NONE {
            self.visit(target, c, parent_visible);
            c = dom.nodes[c as usize].next;
        }
        for &o in owned {
            self.visit(target, o, parent_visible);
        }
        if block {
            self.nodes[target].push_text(" ");
        }
        let t = &mut self.nodes[target];
        if t.role == "link" {
            if let Some(href) = dom.attr(n, Attr::Href) {
                t.url = Some(truncate_data_url(href));
            }
        }
        if t.role == "textbox" {
            if let Some(p) = dom.attr(n, Attr::Placeholder) {
                if p != t.name {
                    t.placeholder = Some(p.to_owned());
                }
            }
        }
    }

    /// Seen with a size (`isElementVisible`).
    fn boxed(&self, n: u32) -> bool {
        self.ctx.dom.flag(n, BOX) && self.sized[n as usize]
    }

    fn assign_ref(&mut self, node: usize, n: u32) {
        let dom = self.ctx.dom;
        if self.ai && self.boxed(n) && !dom.flag(n, NO_POINTER_EVENTS) {
            self.next_ref += 1;
            self.nodes[node].r#ref = self.next_ref;
            self.el_ref[n as usize] = self.next_ref;
        }
    }

    fn node(&mut self, n: u32) -> Option<usize> {
        let dom = self.ctx.dom;
        let html = !dom.flag(n, IN_SVG);
        if html && matches!(dom.tag(n), Tag::Iframe | Tag::Frame) {
            let i = self.nodes.len();
            self.nodes.push(ANode::new("iframe", n));
            self.nodes[i].pointer = dom.flag(n, POINTER);
            self.assign_ref(i, n);
            return Some(i);
        }
        let role = self.ctx.role(n).or(if self.ai { Some("generic") } else { None })?;
        if role == "presentation" || role == "none" {
            return None;
        }
        let name = self.ctx.accessible_name(n, self.ai);
        let inline = dom.flag(n, INLINE) && dom.flag(n, BOX);
        if role == "generic" && inline {
            let mut kids = dom.children(n);
            if let (Some(only), None) = (kids.next(), kids.next()) {
                if dom.is_text(only) {
                    return None;
                }
            }
        }
        let i = self.nodes.len();
        let mut a = ANode::new(role, n);
        a.name = normalize_owned(name.text);
        a.name_refs = name.elements;
        a.pointer = dom.flag(n, POINTER);
        a.active = self.ai && n == ROOT;
        self.nodes.push(a);
        self.assign_ref(i, n);
        let c = &mut *self.ctx;
        let a = &mut self.nodes[i];
        if CHECKED_ROLES.contains(&role) {
            a.checked = c.checked(n);
        }
        if DISABLED_ROLES.contains(&role) {
            a.disabled = c.disabled(n);
        }
        if EXPANDED_ROLES.contains(&role) {
            a.expanded = c.expanded(n);
        }
        if INVALID_ROLES.contains(&role) {
            a.invalid = c.invalid(n);
        }
        if LEVEL_ROLES.contains(&role) {
            a.level = c.level(n);
        }
        if PRESSED_ROLES.contains(&role) {
            a.pressed = c.pressed(n);
        }
        if SELECTED_ROLES.contains(&role) {
            a.selected = c.selected(n);
        }
        let field = html
            && (dom.tag(n) == Tag::Textarea
                || dom.tag(n) == Tag::Input && !matches!(c.input_type(n), "checkbox" | "radio" | "file"));
        if field {
            a.children = vec![Child::Text(c.value(n))];
        }
        Some(i)
    }
}

/// Whether each node would get a size: text that is not all white space, replaced elements
/// (controls, frames, media, an image with its `alt` text or its size), SVG shapes, and what holds
/// any of them. Layout itself is unknown: style sheets may size what is empty here.
fn sizes(dom: &Dom) -> Vec<bool> {
    let n = dom.nodes.len();
    let mut sized = vec![false; n];
    let mut holds = vec![false; n];
    let mut no_size = vec![false; n];
    let mut shrunk = vec![false; n];
    // A length attribute: a number, maybe with a unit (`300`, `1em`, `50%`).
    let number = |v: &str| {
        v.trim()
            .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%')
            .parse::<f64>()
            .ok()
    };
    let positive = |v: Option<&str>| v.and_then(number).is_some_and(|v| v > 0.0);
    let zero = |v: Option<&str>| v.and_then(number).is_some_and(|v| v <= 0.0);
    for i in (0..n as u32).rev() {
        let node = &dom.nodes[i as usize];
        let own = if dom.is_text(i) {
            let svg = dom
                .parent(i)
                .is_some_and(|p| dom.flag(p, IN_SVG) || dom.tag(p) == Tag::Svg);
            let shown = !svg
                || dom
                    .parent(i)
                    .is_some_and(|p| dom.tag(p) == Tag::Text || dom.name(p) == "tspan");
            shown
                && dom
                    .text(i)
                    .chars()
                    .any(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
        } else if dom.flag(i, IN_SVG) {
            matches!(
                dom.name(i),
                "circle"
                    | "rect"
                    | "path"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "ellipse"
                    | "image"
                    | "use"
                    | "foreignobject"
            )
        } else {
            match dom.tag(i) {
                Tag::Img => {
                    let loads = |a: Attr| {
                        dom.attr(i, a).map(str::trim).is_some_and(|s| {
                            !s.is_empty()
                                && (dom.based
                                    || url::Url::parse(s.split_ascii_whitespace().next().unwrap_or(s)).is_ok())
                        })
                    };
                    loads(Attr::Src)
                        || loads(Attr::Srcset)
                        || dom.attr(i, Attr::Alt).is_some_and(|a| !a.trim().is_empty())
                        || positive(dom.attr(i, Attr::Width)) && positive(dom.attr(i, Attr::Height))
                }
                Tag::Svg => {
                    let empty = zero(dom.attr(i, Attr::Width)) || zero(dom.attr(i, Attr::Height));
                    no_size[i as usize] = empty;
                    // Without a width, an SVG alone in a button (a box as wide as its content) is as
                    // wide as nothing; what it draws keeps its size.
                    shrunk[i as usize] = dom.attr(i, Attr::Width).is_none() && alone_in_button(dom, i);
                    !empty
                }
                Tag::Input
                | Tag::Button
                | Tag::Select
                | Tag::Textarea
                | Tag::Meter
                | Tag::Progress
                | Tag::Marquee
                | Tag::Iframe
                | Tag::Frame
                | Tag::Video
                | Tag::Object
                | Tag::Hr
                | Tag::Fieldset
                | Tag::Audio => true,
                // Cells have padding; list items and summaries draw a marker.
                Tag::Td | Tag::Th | Tag::Summary => true,
                Tag::Li => dom.flag(i, LAID_OUT) && !dom.flag(i, INLINE),
                _ => false,
            }
        };
        let s = (own || holds[i as usize])
            && !no_size[i as usize]
            && !shrunk[i as usize]
            && (dom.is_text(i) || !dom.flag(i, ZERO_SIZE));
        sized[i as usize] = s;
        let p = node.parent;
        let counts = if dom.is_text(i) {
            p != NONE && dom.flag(p, LAID_OUT) && !dom.flag(p, HIDES_CONTENT)
        } else {
            dom.flag(i, LAID_OUT)
        };
        // Content hidden until found takes no room (a closed <details> still shows its summary).
        let holds_none = p != NONE && dom.flag(p, HIDES_CONTENT) && dom.tag(p) != Tag::Details;
        if s && p != NONE && counts && !holds_none && !replaced_parent(dom, p) {
            holds[node.parent as usize] = true;
        }
    }
    // What an SVG of no size draws has no size either.
    for i in 1..n {
        let p = dom.nodes[i].parent;
        if p != NONE && no_size[p as usize] {
            no_size[i] = true;
            sized[i] = false;
        }
    }
    sized
}

/// Whether an element is in a `<button>` (through inline elements only) that has no text.
fn alone_in_button(dom: &Dom, n: u32) -> bool {
    let mut at = n;
    while let Some(p) = dom.parent(at) {
        if dom.tag(p) == Tag::Button && !dom.flag(p, IN_SVG) {
            return !rendered_text(dom, p);
        }
        if !dom.flag(p, INLINE) {
            return false;
        }
        at = p;
    }
    false
}

/// Whether an element shows any text (not white space, not in what is not rendered, not an SVG's
/// text outside `<text>`).
fn rendered_text(dom: &Dom, n: u32) -> bool {
    dom.children(n).any(|c| {
        if dom.is_text(c) {
            let svg = dom.flag(n, IN_SVG) || dom.tag(n) == Tag::Svg;
            (!svg || dom.tag(n) == Tag::Text) && !dom.text(c).trim().is_empty()
        } else {
            dom.flag(c, LAID_OUT) && super::grow(|| rendered_text(dom, c))
        }
    })
}

/// Elements whose content is not laid out as content.
fn replaced_parent(dom: &Dom, p: u32) -> bool {
    !dom.flag(p, IN_SVG)
        && matches!(
            dom.tag(p),
            Tag::Iframe | Tag::Frame | Tag::Textarea | Tag::Select | Tag::Video | Tag::Object
        )
}

/// `data:` URLs cut after their comma.
fn truncate_data_url(url: &str) -> String {
    match url.strip_prefix("data:").and_then(|_| url.find(',')) {
        Some(comma) => format!("{}\u{2026}", &url[..=comma]),
        None => url.to_owned(),
    }
}

enum Verdict {
    Keep,
    Remove,
    Unwrap,
}

/// Playwright's distilling passes (`ariaSnapshotDistiller.ts`).
struct Distill<'a> {
    nodes: &'a mut Vec<ANode>,
    ai: bool,
    ancestors: Vec<usize>,
    pending: HashSet<u32>,
    max_depth: Option<usize>,
    depth: i64,
}

impl Distill<'_> {
    fn run(&mut self) {
        self.traverse(0, -1);
        self.depth = -1;
        self.exit(0);
    }

    fn traverse(&mut self, node: usize, depth: i64) {
        let children = std::mem::take(&mut self.nodes[node].children);
        let mut out = Vec::with_capacity(children.len());
        self.ancestors.push(node);
        for c in children {
            match c {
                Child::Text(t) => out.push(Child::Text(t)),
                Child::Node(i) => {
                    self.depth = depth + 1;
                    self.enter(i);
                    super::grow(|| self.traverse(i, depth + 1));
                    self.depth = depth + 1;
                    match self.exit(i) {
                        Verdict::Keep => out.push(Child::Node(i)),
                        Verdict::Remove => {}
                        Verdict::Unwrap => out.append(&mut self.nodes[i].children),
                    }
                }
            }
        }
        self.ancestors.pop();
        self.nodes[node].children = out;
    }

    fn leaf_generic(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        n.role == "generic" && n.children.iter().all(|c| matches!(c, Child::Text(_)))
    }

    fn click_target_root(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        n.r#ref != 0
            && n.pointer
            && !self
                .ancestors
                .iter()
                .any(|&a| self.nodes[a].r#ref != 0 && self.nodes[a].pointer)
    }

    fn enter(&mut self, i: usize) {
        if !self.ai || self.nodes[i].r#ref == 0 {
            return;
        }
        // removeRedundantNames
        for &r in &self.nodes[i].name_refs {
            self.pending.insert(r);
        }
        let beyond = self.max_depth.is_some_and(|m| m > 0 && self.depth > m as i64);
        if !beyond && !self.leaf_generic(i) {
            let r = self.nodes[i].r#ref;
            self.pending.remove(&r);
        }
    }

    fn exit(&mut self, i: usize) -> Verdict {
        self.merge_strings(i);
        if self.ai {
            // removeNamelessImages
            let n = &self.nodes[i];
            if n.role == "img" && n.name.is_empty() && n.children.is_empty() && !self.click_target_root(i) {
                return Verdict::Remove;
            }
            // removeRedundantNames
            if self.nodes[i].r#ref != 0 && !self.nodes[i].name_refs.is_empty() {
                if self.nodes[i].name_refs.iter().all(|r| !self.pending.contains(r)) {
                    self.nodes[i].name.clear();
                } else {
                    for r in std::mem::take(&mut self.nodes[i].name_refs) {
                        self.pending.remove(&r);
                    }
                }
            }
            // inlineTextIntoGeneric
            let n = &self.nodes[i];
            if n.role == "generic" && !n.has_props() && n.children.len() == 1 {
                if let Child::Node(c) = n.children[0] {
                    let child = &self.nodes[c];
                    if child.role == "generic" && child.name.is_empty() && !child.active && !child.has_props() {
                        if let [Child::Text(t)] = child.children.as_slice() {
                            let t = t.clone();
                            self.nodes[i].children = vec![Child::Text(t)];
                        }
                    }
                }
            }
            // removeNameRepeatingChild
            if let Some(&p) = self.ancestors.last() {
                let n = &self.nodes[i];
                let parent = &self.nodes[p].name;
                if !parent.is_empty() && n.role == "generic" && !n.active && !n.has_props() {
                    let text = if !n.name.is_empty() {
                        n.children.is_empty().then_some(n.name.as_str())
                    } else if let [Child::Text(t)] = n.children.as_slice() {
                        Some(t.as_str())
                    } else {
                        None
                    };
                    if text == Some(parent.as_str()) {
                        if n.r#ref != 0 {
                            let r = n.r#ref;
                            self.pending.insert(r);
                        }
                        return Verdict::Remove;
                    }
                }
            }
        }
        // unwrapSingleChildGenerics
        let n = &self.nodes[i];
        if n.role == "generic"
            && n.name.is_empty()
            && n.children.len() <= 1
            && n.children
                .iter()
                .all(|c| matches!(c, Child::Node(k) if self.nodes[*k].r#ref != 0))
        {
            if n.children.is_empty() && self.click_target_root(i) {
                return Verdict::Keep;
            }
            return Verdict::Unwrap;
        }
        Verdict::Keep
    }

    fn merge_strings(&mut self, i: usize) {
        let children = std::mem::take(&mut self.nodes[i].children);
        let mut out = Vec::with_capacity(children.len());
        let mut buf: Option<String> = None;
        let flush = |buf: &mut Option<String>, out: &mut Vec<Child>| {
            if let Some(b) = buf.take() {
                let t = normalize_owned(b);
                if !t.is_empty() {
                    out.push(Child::Text(t));
                }
            }
        };
        for c in children {
            match c {
                Child::Text(t) => match &mut buf {
                    Some(b) => b.push_str(&t),
                    None => buf = Some(t),
                },
                node => {
                    flush(&mut buf, &mut out);
                    out.push(node);
                }
            }
        }
        flush(&mut buf, &mut out);
        let n = &mut self.nodes[i];
        if let [Child::Text(t)] = out.as_slice() {
            if *t == n.name {
                out.clear();
            }
        }
        n.children = out;
    }
}

struct Render<'a> {
    nodes: &'a [ANode],
    depth: Option<usize>,
    cursor: bool,
}

impl Render<'_> {
    fn text(&self, out: &mut String, t: &str, depth: usize) {
        indent(out, depth);
        out.push_str("- text: ");
        yaml_value(out, t);
        out.push('\n');
    }

    fn node(&self, out: &mut String, i: usize, depth: usize, cursor: bool) {
        let n = &self.nodes[i];
        let mut key = String::with_capacity(n.role.len() + n.name.len() + 8);
        key.push_str(n.role);
        if !n.name.is_empty() && n.name.encode_utf16().count() <= 900 {
            key.push(' ');
            json_string(&mut key, &n.name);
        }
        match n.checked {
            Tri::Mixed => key.push_str(" [checked=mixed]"),
            Tri::True => key.push_str(" [checked]"),
            Tri::False => {}
        }
        if n.disabled {
            key.push_str(" [disabled]");
        }
        if n.expanded {
            key.push_str(" [expanded]");
        }
        if n.active {
            key.push_str(" [active]");
        }
        match n.invalid {
            Invalid::Grammar => key.push_str(" [invalid=grammar]"),
            Invalid::Spelling => key.push_str(" [invalid=spelling]"),
            Invalid::True => key.push_str(" [invalid]"),
            Invalid::False => {}
        }
        if n.level != 0 {
            let _ = write!(key, " [level={}]", n.level);
        }
        match n.pressed {
            Tri::Mixed => key.push_str(" [pressed=mixed]"),
            Tri::True => key.push_str(" [pressed]"),
            Tri::False => {}
        }
        if n.selected {
            key.push_str(" [selected]");
        }
        if n.aria_hidden {
            key.push_str(" [aria-hidden]");
        }
        let pointer = n.r#ref != 0 && cursor && n.pointer;
        if n.r#ref != 0 {
            let _ = write!(key, " [ref=e{}]", n.r#ref);
            if pointer {
                key.push_str(" [cursor=pointer]");
            }
        }

        let single = match n.children.as_slice() {
            [Child::Text(t)] => Some(t.as_str()),
            _ => None,
        };
        let at_limit = self.depth.is_some_and(|d| d > 0 && depth == d);
        let children = single.is_none() && !at_limit && !n.children.is_empty();
        let props = n.url.is_some() || n.placeholder.is_some();

        indent(out, depth);
        out.push_str("- ");
        yaml_key(out, &key);
        if single.is_none() && !props && !children {
            out.push('\n');
        } else if let (Some(t), false) = (single, props) {
            out.push_str(": ");
            yaml_value(out, t);
            out.push('\n');
        } else {
            out.push_str(":\n");
            for (k, v) in [("url", &n.url), ("placeholder", &n.placeholder)] {
                if let Some(v) = v {
                    indent(out, depth + 1);
                    out.push_str("- /");
                    out.push_str(k);
                    out.push_str(": ");
                    yaml_value(out, v);
                    out.push('\n');
                }
            }
            if let Some(t) = single {
                self.text(out, t, depth + 1);
            } else if children {
                for c in &n.children {
                    match c {
                        Child::Text(t) => self.text(out, t, depth + 1),
                        Child::Node(k) => {
                            super::grow(|| self.node(out, *k, depth + 1, cursor && !pointer && self.cursor))
                        }
                    }
                }
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// `JSON.stringify` of a string.
fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Whether YAML would read the string as something else (`yamlStringNeedsQuotes`).
fn needs_quotes(s: &str) -> bool {
    let b = s.as_bytes();
    let (Some(&first), Some(&last)) = (b.first(), b.last()) else {
        return true;
    };
    // The first and last characters: white space, an indicator.
    if matches!(
        first,
        b'-' | b'&' | b'*' | b']' | b',' | b'?' | b'!' | b'>' | b'|' | b'@' | b'"' | b'\'' | b'#' | b'%' | b'['
    ) {
        return true;
    }
    let edge_space = |c: Option<char>| c.is_some_and(js_space);
    if first.is_ascii_whitespace() || last.is_ascii_whitespace() {
        return true;
    }
    if (first >= 0x80 || last >= 0x80) && (edge_space(s.chars().next()) || edge_space(s.chars().next_back())) {
        return true;
    }
    let mut prev_space = false;
    let mut ascii = true;
    for (i, &c) in b.iter().enumerate() {
        match c {
            // /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/, /[\n\r]/ (and tab: only quoted when it ends a `:`)
            0..=8 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f | b'\n' | b'\r' => return true,
            b'{' | b'}' | b'`' => return true,
            // /[\n:](\s|$)/
            b':' => match b.get(i + 1) {
                None => return true,
                Some(&n) if n.is_ascii_whitespace() => return true,
                Some(&n) if n >= 0x80 && s[i + 1..].chars().next().is_some_and(js_space) => return true,
                _ => {}
            },
            // /\s#/
            b'#' if prev_space => return true,
            _ => {}
        }
        if c >= 0x80 {
            ascii = false;
        }
        prev_space = c == b' ' || c == b'\t' || c == 0x0b || c == 0x0c;
    }
    if !ascii {
        // C1 controls, and white space before `#` beyond ASCII.
        let mut prev = ' ';
        for c in s.chars() {
            if ('\u{80}'..='\u{9f}').contains(&c) || c == '#' && js_space(prev) {
                return true;
            }
            prev = c;
        }
    }
    if b.len() <= 5
        && ["y", "n", "yes", "no", "true", "false", "on", "off", "null"]
            .iter()
            .any(|w| w.eq_ignore_ascii_case(s))
    {
        return true;
    }
    // Numbers (`12`, `3.5`, `0x1F`, `Infinity`): only strings of number characters are worth parsing.
    b.iter()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        && js_number(s).is_some()
}

fn yaml_key(out: &mut String, s: &str) {
    if !needs_quotes(s) {
        out.push_str(s);
        return;
    }
    out.push('\'');
    out.push_str(&s.replace('\'', "''"));
    out.push('\'');
}

fn yaml_value(out: &mut String, s: &str) {
    if !needs_quotes(s) {
        out.push_str(s);
        return;
    }
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
