//! Roles and states, as Playwright computes them (`roleUtils.ts`): the
//! explicit `role`, else the implicit role of the tag in its context,
//! presentational roles inherited and resolved, and the states a snapshot
//! shows (checked, disabled, expanded, invalid, level, pressed, selected).

use std::collections::HashMap;

use super::dom::{Attr, Dom, Tag, HAS_GLOBAL, HAS_LABEL, HAS_LABELLEDBY, HAS_ROLEDESCRIPTION};

/// The roles a `role` attribute may name (Playwright's `validRoles`).
const VALID_ROLES: &[&str] = &[
    "alert",
    "alertdialog",
    "application",
    "article",
    "banner",
    "blockquote",
    "button",
    "caption",
    "cell",
    "checkbox",
    "code",
    "columnheader",
    "combobox",
    "complementary",
    "contentinfo",
    "definition",
    "deletion",
    "dialog",
    "directory",
    "document",
    "emphasis",
    "feed",
    "figure",
    "form",
    "generic",
    "grid",
    "gridcell",
    "group",
    "heading",
    "img",
    "insertion",
    "link",
    "list",
    "listbox",
    "listitem",
    "log",
    "main",
    "mark",
    "marquee",
    "math",
    "meter",
    "menu",
    "menubar",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "navigation",
    "none",
    "note",
    "option",
    "paragraph",
    "presentation",
    "progressbar",
    "radio",
    "radiogroup",
    "region",
    "row",
    "rowgroup",
    "rowheader",
    "scrollbar",
    "search",
    "searchbox",
    "separator",
    "slider",
    "spinbutton",
    "status",
    "strong",
    "subscript",
    "superscript",
    "switch",
    "tab",
    "table",
    "tablist",
    "tabpanel",
    "term",
    "textbox",
    "time",
    "timer",
    "toolbar",
    "tooltip",
    "tree",
    "treegrid",
    "treeitem",
];

/// Roles that may not be named by `aria-label`/`aria-labelledby`.
const NO_LABEL_ROLES: &[&str] = &[
    "caption",
    "code",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "superscript",
];

pub(crate) const CHECKED_ROLES: &[&str] = &[
    "checkbox",
    "menuitemcheckbox",
    "option",
    "radio",
    "switch",
    "menuitemradio",
    "treeitem",
];
pub(crate) const DISABLED_ROLES: &[&str] = &[
    "application",
    "button",
    "composite",
    "gridcell",
    "group",
    "input",
    "link",
    "menuitem",
    "scrollbar",
    "separator",
    "tab",
    "checkbox",
    "columnheader",
    "combobox",
    "grid",
    "listbox",
    "menu",
    "menubar",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "radiogroup",
    "row",
    "rowheader",
    "searchbox",
    "select",
    "slider",
    "spinbutton",
    "switch",
    "tablist",
    "textbox",
    "toolbar",
    "tree",
    "treegrid",
    "treeitem",
];
pub(crate) const EXPANDED_ROLES: &[&str] = &[
    "application",
    "button",
    "checkbox",
    "combobox",
    "gridcell",
    "link",
    "listbox",
    "menuitem",
    "row",
    "rowheader",
    "tab",
    "treeitem",
    "columnheader",
    "menuitemcheckbox",
    "menuitemradio",
    "switch",
];
pub(crate) const INVALID_ROLES: &[&str] = &[
    "application",
    "checkbox",
    "columnheader",
    "combobox",
    "gridcell",
    "listbox",
    "radiogroup",
    "rowheader",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "textbox",
    "tree",
];
pub(crate) const LEVEL_ROLES: &[&str] = &["heading", "listitem", "row", "treeitem"];
pub(crate) const PRESSED_ROLES: &[&str] = &["button"];
pub(crate) const SELECTED_ROLES: &[&str] = &[
    "gridcell",
    "option",
    "row",
    "tab",
    "rowheader",
    "columnheader",
    "treeitem",
];

/// A tri-state: `aria-checked="mixed"`, `aria-pressed="mixed"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Tri {
    #[default]
    False,
    True,
    Mixed,
}

/// `aria-invalid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Invalid {
    #[default]
    False,
    True,
    Grammar,
    Spelling,
}

/// The types an `<input>` may have; any other is `text`.
const INPUT_TYPES: &[&str] = &[
    "hidden",
    "text",
    "search",
    "tel",
    "url",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
    "range",
    "color",
    "checkbox",
    "radio",
    "file",
    "submit",
    "image",
    "reset",
    "button",
];

/// What JavaScript's `\s` matches.
pub(crate) fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// JavaScript's `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// JavaScript's `Number(s)`, `None` for `NaN`.
pub(crate) fn js_number(s: &str) -> Option<f64> {
    let s = js_trim(s);
    if s.is_empty() {
        return Some(0.0);
    }
    match s {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let b = s.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return u128::from_str_radix(&s[2..], radix).ok().map(|v| v as f64);
        }
    }
    // [+-]? (digits (. digits?)? | . digits) ([eE] [+-]? digits)?
    let mut i = 0;
    if matches!(b[0], b'+' | b'-') {
        i += 1;
    }
    let int = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    i += int;
    let mut frac = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        frac = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += frac;
    }
    if int + frac == 0 {
        return None;
    }
    if i < b.len() && matches!(b[i], b'e' | b'E') {
        i += 1;
        if i < b.len() && matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        let exp = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if exp == 0 {
            return None;
        }
        i += exp;
    }
    if i != b.len() {
        return None;
    }
    s.parse::<f64>().ok()
}

/// What the role and name rules ask of the page, with what they compute once.
pub(crate) struct Ctx<'d> {
    pub dom: &'d Dom,
    /// The first element of each id.
    pub ids: HashMap<&'d str, u32>,
    /// The `<label>`s of each control, in document order.
    pub labels: HashMap<u32, Vec<u32>>,
    roles: Vec<Option<Option<&'static str>>>,
    /// Options selected, by node.
    selected: Vec<bool>,
    /// Per node: the name computation that last visited it.
    pub(crate) visited: Vec<u32>,
    pub(crate) pass: u32,
    /// How deep the name computation is (references nest: `aria-owns` chains).
    pub(crate) depth: u32,
    /// Nodes the current name may still visit, and all names together.
    pub(crate) budget: u32,
    pub(crate) total: u64,
}

/// The most nodes one name visits, and all names of a page together: a name from a huge subtree,
/// or thousands of references to one, stays bounded.
pub(crate) const NAME_BUDGET: u32 = 20_000;
pub(crate) const TOTAL_BUDGET: u64 = 20_000_000;
/// How deep references nest at most.
pub(crate) const MAX_DEPTH: u32 = 1200;

impl<'d> Ctx<'d> {
    pub(crate) fn new(dom: &'d Dom) -> Ctx<'d> {
        let n = dom.nodes.len();
        let mut ids = HashMap::new();
        for i in 0..n as u32 {
            if dom.is_text(i) {
                continue;
            }
            if let Some(id) = dom.attr(i, Attr::Id) {
                ids.entry(id).or_insert(i);
            }
        }
        let mut c = Ctx {
            dom,
            ids,
            labels: HashMap::new(),
            roles: vec![None; n],
            selected: vec![false; n],
            visited: vec![0; n],
            pass: 0,
            depth: 0,
            budget: 0,
            total: 0,
        };
        c.find_labels();
        c.find_selected();
        c
    }

    fn labelable(&self, n: u32) -> bool {
        let t = self.dom.tag(n);
        t.labelable() && !(t == Tag::Input && self.input_type(n) == "hidden")
    }

    /// Each `<label>`'s control: the element its `for` names, else the first labelable element in it.
    fn find_labels(&mut self) {
        let dom = self.dom;
        for i in 0..dom.nodes.len() as u32 {
            if dom.is_text(i) || dom.tag(i) != Tag::Label {
                continue;
            }
            let control = match dom.attr(i, Attr::For) {
                Some(id) => self.ids.get(id).copied().filter(|&c| self.labelable(c)),
                None => dom.descendants(i).find(|&c| self.labelable(c)),
            };
            if let Some(c) = control {
                self.labels.entry(c).or_default().push(i);
            }
        }
    }

    /// The options of a `<select>`: its own, and those of its `<optgroup>`s.
    pub(crate) fn options(&self, select: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for c in self.dom.elements(select) {
            match self.dom.tag(c) {
                Tag::Option => out.push(c),
                Tag::Optgroup => out.extend(self.dom.elements(c).filter(|&o| self.dom.tag(o) == Tag::Option)),
                _ => {}
            }
        }
        out
    }

    pub(crate) fn is_listbox_select(&self, n: u32) -> bool {
        self.dom.has(n, Attr::Multiple) || self.dom.attr(n, Attr::Size).and_then(parse_uint).is_some_and(|s| s > 1)
    }

    /// Which options are selected: those marked, but a single-choice select keeps the last one
    /// marked, else selects its first option that is not disabled.
    fn find_selected(&mut self) {
        let dom = self.dom;
        for i in 0..dom.nodes.len() as u32 {
            if dom.is_text(i) {
                continue;
            }
            match dom.tag(i) {
                // An option in a select is the select's to settle (it comes first).
                Tag::Option if !self.in_select(i) => self.selected[i as usize] = dom.has(i, Attr::Selected),
                Tag::Select => {
                    let options = self.options(i);
                    if dom.has(i, Attr::Multiple) {
                        for &o in &options {
                            self.selected[o as usize] = dom.has(o, Attr::Selected);
                        }
                        continue;
                    }
                    let marked: Vec<u32> = options
                        .iter()
                        .copied()
                        .filter(|&o| dom.has(o, Attr::Selected))
                        .collect();
                    for &o in &options {
                        self.selected[o as usize] = false;
                    }
                    if let Some(&last) = marked.last() {
                        self.selected[last as usize] = true;
                    } else if !self.is_listbox_select(i) {
                        if let Some(&first) = options.iter().find(|&&o| !self.option_disabled(o)) {
                            self.selected[first as usize] = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Whether an option is one of a `<select>`'s options.
    fn in_select(&self, o: u32) -> bool {
        let d = self.dom;
        match d.parent(o) {
            Some(p) if d.tag(p) == Tag::Select => true,
            Some(p) if d.tag(p) == Tag::Optgroup => d.parent(p).is_some_and(|g| d.tag(g) == Tag::Select),
            _ => false,
        }
    }

    fn option_disabled(&self, o: u32) -> bool {
        self.dom.has(o, Attr::Disabled)
            || self
                .dom
                .parent(o)
                .is_some_and(|p| self.dom.tag(p) == Tag::Optgroup && self.dom.has(p, Attr::Disabled))
    }

    /// An option's selectedness (`option.selected`).
    pub(crate) fn option_selected(&self, o: u32) -> bool {
        self.selected[o as usize]
    }

    /// `input.type`: lower case, `text` when unknown.
    pub(crate) fn input_type(&self, n: u32) -> &'static str {
        let t = self.dom.attr(n, Attr::Type).unwrap_or("");
        INPUT_TYPES
            .iter()
            .find(|k| k.eq_ignore_ascii_case(t))
            .copied()
            .unwrap_or("text")
    }

    /// The elements an id list names (`aria-labelledby`, `aria-owns`), each once.
    pub(crate) fn id_refs(&self, list: Option<&str>) -> Vec<u32> {
        let mut out = Vec::new();
        for id in list.unwrap_or("").split(' ').filter(|s| !s.is_empty()) {
            if let Some(&e) = self.ids.get(id) {
                if !out.contains(&e) {
                    out.push(e);
                }
            }
        }
        out
    }

    pub(crate) fn explicit_role(&self, n: u32) -> Option<&'static str> {
        let role = self.dom.attr(n, Attr::Role)?;
        role.split(' ')
            .map(str::trim)
            .find_map(|r| VALID_ROLES.iter().find(|v| **v == r).copied())
    }

    fn has_explicit_name(&self, n: u32) -> bool {
        self.dom.flag(n, HAS_LABEL | HAS_LABELLEDBY)
    }

    /// `hasGlobalAriaAttribute`: one of the global ARIA attributes the role allows.
    pub(crate) fn has_global_aria(&self, n: u32, role: Option<&str>) -> bool {
        let r = role.unwrap_or("");
        self.dom.flag(n, HAS_GLOBAL)
            || self.dom.flag(n, HAS_LABEL | HAS_LABELLEDBY) && !NO_LABEL_ROLES.contains(&r)
            || self.dom.flag(n, HAS_ROLEDESCRIPTION) && r != "generic"
    }

    pub(crate) fn has_tabindex(&self, n: u32) -> bool {
        // `Number(String(getAttribute("tabindex")))`: no attribute is "null", NaN.
        self.dom.attr(n, Attr::Tabindex).is_some_and(|t| js_number(t).is_some())
    }

    fn natively_focusable(&self, n: u32) -> bool {
        match self.dom.tag(n) {
            Tag::Button | Tag::Details | Tag::Select | Tag::Textarea => true,
            Tag::A | Tag::Area => self.dom.has(n, Attr::Href),
            Tag::Input => !self.dom.has(n, Attr::Hidden),
            _ => false,
        }
    }

    pub(crate) fn is_focusable(&self, n: u32) -> bool {
        !self.natively_disabled(n) && (self.natively_focusable(n) || self.has_tabindex(n))
    }

    fn presentation_conflict(&self, n: u32, role: Option<&str>) -> bool {
        self.has_global_aria(n, role) || self.is_focusable(n)
    }

    /// The element's role, `None` for none (Playwright's `getAriaRole`).
    pub(crate) fn role(&mut self, n: u32) -> Option<&'static str> {
        if let Some(r) = self.roles[n as usize] {
            return r;
        }
        let r = self.compute_role(n);
        self.roles[n as usize] = Some(r);
        r
    }

    fn compute_role(&mut self, n: u32) -> Option<&'static str> {
        let Some(explicit) = self.explicit_role(n) else {
            return self.implicit_role(n);
        };
        if explicit == "none" || explicit == "presentation" {
            let implicit = self.implicit_role(n);
            if self.presentation_conflict(n, implicit) {
                return implicit;
            }
        }
        Some(explicit)
    }

    /// Whether a landmark ancestor (or the element itself) keeps a `<header>` or `<footer>` from being one.
    fn in_sectioning(&self, n: u32) -> bool {
        let mut at = Some(n);
        while let Some(e) = at {
            let role = self.dom.attr(e, Attr::Role);
            let sectioning = matches!(
                self.dom.tag(e),
                Tag::Article | Tag::Aside | Tag::Main | Tag::Nav | Tag::Section
            ) && role.is_none()
                || role.is_some_and(|r| matches!(r, "article" | "complementary" | "main" | "navigation" | "region"));
            if sectioning {
                return true;
            }
            at = self.dom.parent(e);
        }
        false
    }

    fn closest(&self, n: u32, tag: Tag) -> Option<u32> {
        let mut at = Some(n);
        while let Some(e) = at {
            if self.dom.tag(e) == tag {
                return Some(e);
            }
            at = self.dom.parent(e);
        }
        None
    }

    fn implicit_tag_role(&self, n: u32) -> Option<&'static str> {
        let d = self.dom;
        if d.flag(n, super::dom::IN_SVG) {
            // Inside SVG only `<a href>` has a role here (tag names are SVG's).
            return match d.tag(n) {
                Tag::A if d.has(n, Attr::Href) => Some("link"),
                _ => None,
            };
        }
        Some(match d.tag(n) {
            Tag::A | Tag::Area => return d.has(n, Attr::Href).then_some("link"),
            Tag::Article => "article",
            Tag::Aside => "complementary",
            Tag::Blockquote => "blockquote",
            Tag::Button => "button",
            Tag::Caption => "caption",
            Tag::Code => "code",
            Tag::Datalist => "listbox",
            Tag::Dd => "definition",
            Tag::Del => "deletion",
            Tag::Details => "group",
            Tag::Dfn => "term",
            Tag::Dialog => "dialog",
            Tag::Dt => "term",
            Tag::Em => "emphasis",
            Tag::Fieldset => "group",
            Tag::Figure => "figure",
            Tag::Footer => return (!self.in_sectioning(n)).then_some("contentinfo"),
            Tag::Form => return self.has_explicit_name(n).then_some("form"),
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => "heading",
            Tag::Header => return (!self.in_sectioning(n)).then_some("banner"),
            Tag::Hr => "separator",
            Tag::Html => "document",
            Tag::Img => {
                let presentational = d.attr(n, Attr::Alt) == Some("")
                    && d.attr(n, Attr::Title).is_none_or(str::is_empty)
                    && !self.has_global_aria(n, None)
                    && !self.has_tabindex(n);
                if presentational {
                    "presentation"
                } else {
                    "img"
                }
            }
            Tag::Input => {
                let t = self.input_type(n);
                match t {
                    "email" | "search" | "tel" | "text" | "url" => {
                        let list = self.id_refs(d.attr(n, Attr::List)).first().copied();
                        if list.is_some_and(|l| d.tag(l) == Tag::Datalist) {
                            "combobox"
                        } else if t == "search" {
                            "searchbox"
                        } else {
                            "textbox"
                        }
                    }
                    "hidden" => return None,
                    "file" | "button" | "image" | "reset" | "submit" => "button",
                    "checkbox" => "checkbox",
                    "number" => "spinbutton",
                    "radio" => "radio",
                    "range" => "slider",
                    _ => "textbox",
                }
            }
            Tag::Ins => "insertion",
            Tag::Li => "listitem",
            Tag::Main => "main",
            Tag::Mark => "mark",
            Tag::Math => "math",
            Tag::Menu => "list",
            Tag::Meter => "meter",
            Tag::Nav => "navigation",
            Tag::Ol => "list",
            Tag::Optgroup => "group",
            Tag::Option => "option",
            Tag::Output => "status",
            Tag::P => "paragraph",
            Tag::Progress => "progressbar",
            Tag::Search => "search",
            Tag::Section => return self.has_explicit_name(n).then_some("region"),
            Tag::Select => {
                if self.is_listbox_select(n) {
                    "listbox"
                } else {
                    "combobox"
                }
            }
            Tag::Strong => "strong",
            Tag::Sub => "subscript",
            Tag::Sup => "superscript",
            Tag::Svg => "img",
            Tag::Table => "table",
            Tag::Tbody | Tag::Thead | Tag::Tfoot => "rowgroup",
            Tag::Td => {
                let role = self.closest(n, Tag::Table).and_then(|t| self.explicit_role(t));
                if matches!(role, Some("grid" | "treegrid")) {
                    "gridcell"
                } else {
                    "cell"
                }
            }
            Tag::Textarea => "textbox",
            Tag::Th => return self.header_cell_role(n),
            Tag::Time => "time",
            Tag::Tr => "row",
            Tag::Ul => "list",
            _ => return None,
        })
    }

    fn header_cell_role(&self, n: u32) -> Option<&'static str> {
        let d = self.dom;
        match d.attr(n, Attr::Scope) {
            Some("col" | "colgroup") => return Some("columnheader"),
            Some("row" | "rowgroup") => return Some("rowheader"),
            _ => {}
        }
        let next = d.next_element(n);
        let prev = d.prev_element(n);
        let row = d.parent(n).filter(|&p| d.tag(p) == Tag::Tr);
        if next.is_none() && prev.is_none() {
            if let Some(row) = row {
                if let Some(table) = self.closest(row, Tag::Table) {
                    if self.rows(table) <= 1 {
                        return None;
                    }
                }
            }
            return Some("columnheader");
        }
        let header = |e: Option<u32>| e.is_some_and(|e| d.tag(e) == Tag::Th);
        let data = |e: Option<u32>| {
            e.is_some_and(|e| {
                if d.tag(e) != Tag::Td {
                    return false;
                }
                let mut t = String::new();
                d.text_content(e, &mut t);
                !js_trim(&t).is_empty() || d.elements(e).next().is_some()
            })
        };
        if header(next) && header(prev) {
            return Some("columnheader");
        }
        if data(next) || data(prev) {
            return Some("rowheader");
        }
        Some("columnheader")
    }

    /// `table.rows.length`.
    fn rows(&self, table: u32) -> usize {
        let d = self.dom;
        let mut n = 0;
        for c in d.elements(table) {
            match d.tag(c) {
                Tag::Tr => n += 1,
                Tag::Thead | Tag::Tbody | Tag::Tfoot => n += d.elements(c).filter(|&r| d.tag(r) == Tag::Tr).count(),
                _ => {}
            }
        }
        n
    }

    fn implicit_role(&mut self, n: u32) -> Option<&'static str> {
        let implicit = self.implicit_tag_role(n)?;
        // A table part, list item or term in a presentational container is presentational too.
        let d = self.dom;
        let mut at = n;
        while let Some(parent) = d.parent(at) {
            let parents: &[Tag] = match d.tag(at) {
                Tag::Dd | Tag::Dt => &[Tag::Dl, Tag::Div],
                Tag::Div => &[Tag::Dl],
                Tag::Li => &[Tag::Ol, Tag::Ul],
                Tag::Tbody | Tag::Tfoot | Tag::Thead => &[Tag::Table],
                Tag::Td | Tag::Th => &[Tag::Tr],
                Tag::Tr => &[Tag::Thead, Tag::Tbody, Tag::Tfoot, Tag::Table],
                _ => break,
            };
            if d.flag(at, super::dom::IN_SVG) || !parents.contains(&d.tag(parent)) {
                break;
            }
            let explicit = self.explicit_role(parent);
            if let Some(r @ ("none" | "presentation")) = explicit {
                if !self.presentation_conflict(parent, Some(r)) {
                    return Some(r);
                }
            }
            at = parent;
        }
        Some(implicit)
    }

    pub(crate) fn natively_disabled(&self, n: u32) -> bool {
        let d = self.dom;
        let control = matches!(
            d.tag(n),
            Tag::Button | Tag::Input | Tag::Select | Tag::Textarea | Tag::Option | Tag::Optgroup
        ) && !d.flag(n, super::dom::IN_SVG);
        if !control {
            return false;
        }
        if d.has(n, Attr::Disabled) {
            return true;
        }
        if d.tag(n) == Tag::Option {
            let mut at = d.parent(n);
            while let Some(e) = at {
                if d.tag(e) == Tag::Optgroup && d.has(e, Attr::Disabled) {
                    return true;
                }
                at = d.parent(e);
            }
        }
        // In a disabled <fieldset>, unless in its first <legend>.
        let mut child = n;
        let mut at = d.parent(n);
        while let Some(e) = at {
            if d.tag(e) == Tag::Fieldset && d.has(e, Attr::Disabled) {
                let legend = d.elements(e).find(|&c| d.tag(c) == Tag::Legend);
                return legend != Some(child);
            }
            child = e;
            at = d.parent(e);
        }
        false
    }

    pub(crate) fn checked(&mut self, n: u32) -> Tri {
        let d = self.dom;
        if d.tag(n) == Tag::Input && matches!(self.input_type(n), "checkbox" | "radio") {
            return if d.has(n, Attr::Checked) { Tri::True } else { Tri::False };
        }
        if self.role(n).is_some_and(|r| CHECKED_ROLES.contains(&r)) {
            return match d.attr(n, Attr::AriaChecked) {
                Some("true") => Tri::True,
                Some("mixed") => Tri::Mixed,
                _ => Tri::False,
            };
        }
        Tri::False
    }

    pub(crate) fn disabled(&mut self, n: u32) -> bool {
        if self.natively_disabled(n) {
            return true;
        }
        if !self.role(n).is_some_and(|r| DISABLED_ROLES.contains(&r)) {
            return false;
        }
        let mut at = Some(n);
        while let Some(e) = at {
            match self
                .dom
                .attr(e, Attr::AriaDisabled)
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("true") => return true,
                Some("false") => return false,
                _ => at = self.dom.parent(e),
            }
        }
        false
    }

    pub(crate) fn expanded(&mut self, n: u32) -> bool {
        if self.dom.tag(n) == Tag::Details {
            return self.dom.has(n, Attr::Open);
        }
        self.role(n).is_some_and(|r| EXPANDED_ROLES.contains(&r))
            && self.dom.attr(n, Attr::AriaExpanded) == Some("true")
    }

    pub(crate) fn invalid(&self, n: u32) -> Invalid {
        match self.dom.attr(n, Attr::AriaInvalid) {
            None => Invalid::False,
            Some(v) if v.trim().is_empty() || v.eq_ignore_ascii_case("false") => Invalid::False,
            Some("grammar") => Invalid::Grammar,
            Some("spelling") => Invalid::Spelling,
            Some(_) => Invalid::True,
        }
    }

    pub(crate) fn level(&mut self, n: u32) -> u32 {
        let native = match self.dom.tag(n) {
            Tag::H1 => 1,
            Tag::H2 => 2,
            Tag::H3 => 3,
            Tag::H4 => 4,
            Tag::H5 => 5,
            Tag::H6 => 6,
            _ => 0,
        };
        if native != 0 && !self.dom.flag(n, super::dom::IN_SVG) {
            return native;
        }
        if self.role(n).is_some_and(|r| LEVEL_ROLES.contains(&r)) {
            if let Some(v) = self.dom.attr(n, Attr::AriaLevel).and_then(js_number) {
                if v.fract() == 0.0 && v >= 1.0 && v.is_finite() {
                    return v.min(u32::MAX as f64) as u32;
                }
            }
        }
        0
    }

    pub(crate) fn pressed(&mut self, n: u32) -> Tri {
        if !self.role(n).is_some_and(|r| PRESSED_ROLES.contains(&r)) {
            return Tri::False;
        }
        match self.dom.attr(n, Attr::AriaPressed) {
            Some("true") => Tri::True,
            Some("mixed") => Tri::Mixed,
            _ => Tri::False,
        }
    }

    pub(crate) fn selected(&mut self, n: u32) -> bool {
        if self.dom.tag(n) == Tag::Option && !self.dom.flag(n, super::dom::IN_SVG) {
            return self.option_selected(n);
        }
        self.role(n).is_some_and(|r| SELECTED_ROLES.contains(&r))
            && self.dom.attr(n, Attr::AriaSelected) == Some("true")
    }

    /// `input.value` / `textarea.value`, as the markup sets it.
    pub(crate) fn value(&self, n: u32) -> String {
        let d = self.dom;
        if d.tag(n) == Tag::Textarea {
            let mut t = String::new();
            d.text_content(n, &mut t);
            // The parser drops a newline right after `<textarea>`.
            if let Some(rest) = t.strip_prefix('\n') {
                return rest.to_owned();
            }
            return t;
        }
        let raw = d.attr(n, Attr::Value);
        match self.input_type(n) {
            "text" | "search" | "tel" | "password" => raw.unwrap_or("").replace(['\n', '\r'], ""),
            "url" | "email" => {
                let v = raw.unwrap_or("").replace(['\n', '\r'], "");
                v.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0c}' | '\r'))
                    .to_owned()
            }
            "number" => raw
                .filter(|v| {
                    !v.is_empty() && js_number(v).is_some_and(f64::is_finite) && !v.starts_with(char::is_whitespace)
                })
                .unwrap_or("")
                .to_owned(),
            "range" => {
                let num = |a: Option<&str>| a.and_then(js_number).filter(|v| v.is_finite());
                let min = num(d.attr(n, Attr::Min)).unwrap_or(0.0);
                let max = num(d.attr(n, Attr::Max)).unwrap_or(100.0).max(min);
                let v = num(raw).unwrap_or(if max < min { min } else { min + (max - min) / 2.0 });
                format_number(v.clamp(min, max))
            }
            "color" => raw
                .filter(|v| v.len() == 7 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                .map_or_else(|| "#000000".to_owned(), str::to_ascii_lowercase),
            "checkbox" | "radio" => raw.unwrap_or("on").to_owned(),
            "file" => String::new(),
            _ => raw.unwrap_or("").to_owned(),
        }
    }
}

/// HTML's rules for parsing a non-negative integer.
fn parse_uint(s: &str) -> Option<u32> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\u{0c}', '\r']);
    let s = s.strip_prefix('+').unwrap_or(s);
    let digits: &str = &s[..s.bytes().take_while(u8::is_ascii_digit).count()];
    if digits.is_empty() {
        return None;
    }
    Some(
        digits
            .parse::<u64>()
            .map_or(u32::MAX, |v| v.min(u32::MAX as u64) as u32),
    )
}

/// A number as JavaScript writes it (`50`, `2.5`).
fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e21 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
