//! CSS selectors matched as elements open, against the open-element stack.
//!
//! Matching follows css-select (what htmlparser2-based code queries with):
//! type, `*`, `#id`, `.class`, every attribute operator (`=`, `~=`, `|=`,
//! `^=`, `$=`, `*=`, `!=`, with `i`/`s` and HTML's case-insensitive
//! attributes), the four combinators, `:not()`, `:is()`/`:where()`/
//! `:matches()`, `:first-child`, `:first-of-type`, `:nth-child()`,
//! `:nth-of-type()`, `:root` and `:scope`. A selector under a scope picks
//! elements under it, each matched in the page's context, as the DOM's
//! `element.querySelectorAll` does (css-select with `relativeSelector:
//! false`): `.card h3` under a `.card` is its `h3`. `:scope h3` keeps to
//! the scope, and `> a` means `:scope > a`. Inside `:has()` a selector is
//! relative to the element tested: `:has(a b)` means `:has(:scope a b)`.
//!
//! What only the rest of the document can tell is allowed on the selected
//! element (the last compound) and settled later: `:has()`, `:contains()`
//! (`:icontains()`), `:empty` when the element closes; `:last-child`,
//! `:only-child`, `:last-of-type`, `:only-of-type`, `:nth-last-child()`,
//! `:nth-last-of-type()` when its parent closes. Anywhere else they are an
//! error, as are pseudo-elements and the pseudo-classes of a live page
//! (`:hover`, `:checked`, …).

mod parse;

use crate::tree::{Element, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Combinator {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttrOp {
    Exists,
    Equals,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substring,
    NotEquals,
}

/// `an+b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Nth {
    pub(crate) a: i32,
    pub(crate) b: i32,
}

impl Nth {
    fn matches(self, index: u32) -> bool {
        let i = index as i32;
        if self.a == 0 {
            return i == self.b;
        }
        let d = i - self.b;
        d % self.a == 0 && d / self.a >= 0
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Cond {
    Id(String),
    Class(String),
    Attr {
        name: String,
        op: AttrOp,
        value: String,
        ignore_case: Option<bool>,
    },
    /// `:nth-child()` and its kin; `last` counts from the end (settled when the parent closes).
    Nth {
        nth: Nth,
        of_type: bool,
        last: bool,
    },
    /// `:only-child`, `:only-of-type` (settled when the parent closes).
    Only {
        of_type: bool,
    },
    Root,
    Scope,
    /// Settled when the element closes.
    Empty,
    Not(Vec<Complex>),
    Is(Vec<Complex>),
    /// Settled when the element closes.
    Has(Vec<Complex>),
    /// Settled when the element closes.
    Contains {
        text: String,
        ignore_case: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Compound {
    pub(crate) tag: Option<String>,
    /// [`crate::tree::tag_hash`] of `tag`.
    pub(crate) hash: u32,
    pub(crate) conds: Vec<Cond>,
}

/// A complex selector: compounds and the combinator before each.
#[derive(Clone, Debug)]
pub(crate) struct Complex {
    pub(crate) parts: Vec<(Combinator, Compound)>,
    /// A combinator before the first compound (`> a`).
    pub(crate) leading: Option<Combinator>,
}

/// A condition on the parent's children, settled when it closes.
#[derive(Clone, Copy, Debug)]
pub enum SiblingCheck {
    /// Position from the end (`:last-child` is `Nth(0n+1)`).
    FromEnd {
        nth: (i32, i32),
        of_type: bool,
    },
    Only {
        of_type: bool,
    },
}

impl SiblingCheck {
    /// Whether it holds for a child at `index` (and `type_index`) once its parent holds `count` (`type_count`).
    pub fn holds(&self, index: u32, type_index: u32, count: u32, type_count: u32) -> bool {
        match *self {
            SiblingCheck::FromEnd { nth: (a, b), of_type } => {
                let from_end = if of_type {
                    type_count - type_index + 1
                } else {
                    count - index + 1
                };
                Nth { a, b }.matches(from_end)
            }
            SiblingCheck::Only { of_type } => {
                if of_type {
                    type_count == 1
                } else {
                    count == 1
                }
            }
        }
    }
}

/// What a match leaves to be settled later.
#[derive(Debug, Default)]
pub struct Deferred {
    /// Some element inside must match each (relative to the selected element).
    pub has: Vec<Selector>,
    /// The element's text (all of it, as `textContent`) must contain each.
    pub contains: Vec<(String, bool)>,
    /// No element and only whitespace inside.
    pub empty: bool,
    pub siblings: Vec<SiblingCheck>,
}

impl Deferred {
    pub fn is_empty(&self) -> bool {
        self.has.is_empty() && self.contains.is_empty() && !self.empty && self.siblings.is_empty()
    }

    /// Settled when the element closes.
    pub fn on_close(&self) -> bool {
        !self.has.is_empty() || !self.contains.is_empty() || self.empty
    }
}

struct Alt {
    parts: Vec<(Combinator, Compound)>,
    deferred: Deferred,
    /// `:scope` was put in front (`a b` → `:scope a b`). css-select does so
    /// only for a scope element with an element parent: under a top-level
    /// one, `a b` may match through the scope itself.
    implicit_scope: bool,
}

/// A compiled selector list.
pub struct Selector {
    alts: Vec<Alt>,
    source: String,
    siblings: bool,
    types: bool,
    /// Some alternative starts with a combinator (`> a`).
    leading: bool,
}

impl std::fmt::Debug for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Selector({:?})", self.source)
    }
}

/// Which alternatives of a list matched (by index); empty when none did.
pub type Matched = smallvec::SmallVec<[u16; 2]>;

impl Selector {
    /// A selector for the document (`scoped` false) or for the elements under a scope element,
    /// matched in the page's context.
    pub fn new(source: &str, scoped: bool) -> Result<Selector, String> {
        let list = parse::parse_list(source)?;
        Self::compile(list, source, scoped, false, true)
    }

    /// `relative`: what names no `:scope` is put under it (`a b` → `:scope a b`), as in `:has()`.
    fn compile(
        list: Vec<Complex>,
        source: &str,
        scoped: bool,
        relative: bool,
        deferrable: bool,
    ) -> Result<Selector, String> {
        let mut alts = Vec::with_capacity(list.len());
        let mut siblings = false;
        let leading = list.iter().any(|c| c.leading.is_some());
        for complex in list {
            let mut parts = complex.parts;
            if matches!(complex.leading, Some(Combinator::Adjacent | Combinator::Sibling)) {
                return Err(format!("{source:?}: a selector cannot start from the scope's siblings"));
            }
            let scope = (
                Combinator::Descendant,
                Compound {
                    tag: None,
                    hash: 0,
                    conds: vec![Cond::Scope],
                },
            );
            let mut implicit_scope = false;
            if let Some(c) = complex.leading {
                parts[0].0 = c;
                parts.insert(0, scope);
            } else if scoped && relative && !parts.iter().any(|(_, c)| mentions_scope(c)) {
                parts.insert(0, scope);
                implicit_scope = true;
            }
            let mut deferred = Deferred::default();
            let last = parts.len() - 1;
            for (i, (comb, compound)) in parts.iter_mut().enumerate() {
                siblings |= matches!(comb, Combinator::Adjacent | Combinator::Sibling) && i > 0;
                let mut kept = Vec::with_capacity(compound.conds.len());
                for cond in std::mem::take(&mut compound.conds) {
                    let later = is_deferred(&cond);
                    if later && (i != last || !deferrable) {
                        return Err(format!(
                            "{source:?}: {} can only be tested on the selected element itself",
                            describe(&cond)
                        ));
                    }
                    match cond {
                        Cond::Nth {
                            nth,
                            of_type,
                            last: true,
                        } => deferred.siblings.push(SiblingCheck::FromEnd {
                            nth: (nth.a, nth.b),
                            of_type,
                        }),
                        Cond::Only { of_type } => deferred.siblings.push(SiblingCheck::Only { of_type }),
                        Cond::Empty => deferred.empty = true,
                        Cond::Contains { text, ignore_case } => deferred.contains.push((text, ignore_case)),
                        Cond::Has(list) => {
                            let inner = Selector::compile(list, source, true, true, false)?;
                            siblings |= inner.siblings;
                            deferred.has.push(inner);
                        }
                        Cond::Not(list) | Cond::Is(list) if list.iter().any(complex_defers) => {
                            return Err(format!(
                                "{source:?}: :not() and :is() cannot hold what is settled later"
                            ));
                        }
                        other => {
                            if let Cond::Not(l) | Cond::Is(l) = &other {
                                siblings |= l.iter().any(|c| {
                                    c.parts
                                        .iter()
                                        .skip(1)
                                        .any(|(k, _)| matches!(k, Combinator::Adjacent | Combinator::Sibling))
                                });
                            }
                            kept.push(other)
                        }
                    }
                }
                compound.conds = kept;
            }
            alts.push(Alt {
                parts,
                deferred,
                implicit_scope,
            });
        }
        // `:nth-of-type()` and the rest count siblings by name.
        let types = source.contains("of-type");
        Ok(Selector {
            alts,
            source: source.to_owned(),
            siblings,
            types,
            leading,
        })
    }

    /// Whether some alternative starts with a combinator (`> a`: from the scope).
    pub(crate) fn leading(&self) -> bool {
        self.leading
    }

    /// Whether some alternative is settled only when the selected element's parent closes (`:last-child` …).
    pub(crate) fn waits_on_siblings(&self) -> bool {
        self.alts.iter().any(|a| !a.deferred.siblings.is_empty())
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether matching needs each element's earlier siblings (`+`, `~`).
    pub fn needs_siblings(&self) -> bool {
        self.siblings
    }

    /// The tags the selected element can have, when every alternative names one (`a[href], h2`);
    /// None when an alternative could select any element.
    pub fn subject_tags(&self) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for alt in &self.alts {
            let (_, subject) = alt.parts.last()?;
            out.push(subject.tag.clone()?);
        }
        out.sort();
        out.dedup();
        Some(out)
    }

    /// Every attribute name the selector tests, onto `out`.
    pub fn attributes(&self, out: &mut Vec<String>) {
        fn compound(c: &Compound, out: &mut Vec<String>) {
            for cond in &c.conds {
                match cond {
                    Cond::Id(_) => out.push("id".into()),
                    Cond::Class(_) => out.push("class".into()),
                    Cond::Attr { name, .. } => out.push(name.clone()),
                    Cond::Not(l) | Cond::Is(l) | Cond::Has(l) => {
                        for x in l {
                            for (_, c) in &x.parts {
                                compound(c, out);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for alt in &self.alts {
            for (_, c) in &alt.parts {
                compound(c, out);
            }
            for has in &alt.deferred.has {
                has.attributes(out);
            }
        }
    }

    /// Whether matching needs per-name sibling counts (`*-of-type`).
    pub fn needs_types(&self) -> bool {
        self.types
    }

    /// What alternative `alt` leaves to settle.
    pub fn deferred(&self, alt: u16) -> &Deferred {
        &self.alts[alt as usize].deferred
    }

    /// Whether any alternative settles later at all.
    pub fn defers(&self) -> bool {
        self.alts.iter().any(|a| !a.deferred.is_empty())
    }

    /// The alternatives the innermost open element matches, given the scope element's frame
    /// (only an element under it can match).
    pub fn matches(&self, tree: &Tree, scope: Option<usize>) -> Matched {
        let Some(top) = tree.frames.len().checked_sub(1) else {
            return Matched::new();
        };
        if scope.is_some_and(|s| top <= s) || tree.in_template(top, scope) {
            return Matched::new();
        }
        let cx = Cx { tree, scope };
        let mut out = Matched::new();
        for (i, alt) in self.alts.iter().enumerate() {
            let parts = if alt.implicit_scope && scope == Some(0) {
                &alt.parts[1..]
            } else {
                &alt.parts[..]
            };
            if cx.from(parts, parts.len() - 1, Pos::Frame(top)) {
                out.push(i as u16);
            }
        }
        out
    }

    /// Whether the innermost open element matches with nothing left to settle.
    pub fn matches_now(&self, tree: &Tree, scope: Option<usize>) -> bool {
        self.matches(tree, scope)
            .iter()
            .any(|&a| self.alts[a as usize].deferred.is_empty())
    }
}

fn mentions_scope(c: &Compound) -> bool {
    c.conds.iter().any(|cond| match cond {
        Cond::Scope => true,
        Cond::Not(l) | Cond::Is(l) => l.iter().any(|x| x.parts.iter().any(|(_, c)| mentions_scope(c))),
        _ => false,
    })
}

fn is_deferred(c: &Cond) -> bool {
    matches!(
        c,
        Cond::Nth { last: true, .. } | Cond::Only { .. } | Cond::Empty | Cond::Has(_) | Cond::Contains { .. }
    )
}

fn complex_defers(c: &Complex) -> bool {
    c.parts.iter().any(|(_, compound)| {
        compound
            .conds
            .iter()
            .any(|cond| is_deferred(cond) || matches!(cond, Cond::Not(l) | Cond::Is(l) if l.iter().any(complex_defers)))
    })
}

fn describe(c: &Cond) -> &'static str {
    match c {
        Cond::Nth { of_type: false, .. } => ":last-child / :nth-last-child()",
        Cond::Nth { of_type: true, .. } => ":last-of-type / :nth-last-of-type()",
        Cond::Only { .. } => ":only-child / :only-of-type",
        Cond::Empty => ":empty",
        Cond::Has(_) => ":has()",
        Cond::Contains { .. } => ":contains()",
        _ => "this",
    }
}

/// An element a match is at: an open one, or an earlier sibling of one.
#[derive(Clone, Copy)]
enum Pos {
    Frame(usize),
    /// The `k`-th child of the element at `parent` (`None`: of the document).
    Sib {
        parent: Option<usize>,
        k: usize,
    },
}

struct Cx<'t> {
    tree: &'t Tree,
    scope: Option<usize>,
}

impl Cx<'_> {
    fn parent(&self, pos: Pos) -> Option<usize> {
        match pos {
            Pos::Frame(i) => i.checked_sub(1),
            Pos::Sib { parent, .. } => parent,
        }
    }

    fn element(&self, pos: Pos) -> &Element {
        match pos {
            Pos::Frame(i) => &self.tree.frames[i].el,
            Pos::Sib { parent, k } => &self.tree.children_of(parent).list()[k],
        }
    }

    /// Its position among siblings, and among siblings of its name (1-based).
    fn index(&self, pos: Pos) -> (u32, u32) {
        match pos {
            Pos::Frame(i) => (self.tree.frames[i].index, self.tree.frames[i].type_index),
            Pos::Sib { parent, k } => {
                let list = self.tree.children_of(parent).list();
                let name = list[k].name();
                let of_type = list[..=k].iter().filter(|e| e.name() == name).count();
                (k as u32 + 1, of_type as u32)
            }
        }
    }

    /// The siblings before it, nearest first.
    fn earlier(&self, pos: Pos) -> impl Iterator<Item = Pos> + '_ {
        let (parent, before) = match pos {
            Pos::Frame(i) => (i.checked_sub(1), self.tree.frames[i].index as usize - 1),
            Pos::Sib { parent, k } => (parent, k),
        };
        // Without kept siblings the list is empty: no sibling matches.
        let kept = self.tree.children_of(parent).list().len();
        (0..before.min(kept)).rev().map(move |k| Pos::Sib { parent, k })
    }

    /// Whether `parts[..=at]` match with `parts[at]` at `pos`.
    fn from(&self, parts: &[(Combinator, Compound)], at: usize, pos: Pos) -> bool {
        if !self.compound(&parts[at].1, pos) {
            return false;
        }
        if at == 0 {
            return true;
        }
        match parts[at].0 {
            Combinator::Child => self
                .parent(pos)
                .is_some_and(|p| self.from(parts, at - 1, Pos::Frame(p))),
            Combinator::Descendant => {
                let mut p = self.parent(pos);
                while let Some(i) = p {
                    if self.from(parts, at - 1, Pos::Frame(i)) {
                        return true;
                    }
                    p = i.checked_sub(1);
                }
                false
            }
            Combinator::Adjacent => self.earlier(pos).next().is_some_and(|s| self.from(parts, at - 1, s)),
            Combinator::Sibling => self.earlier(pos).any(|s| self.from(parts, at - 1, s)),
        }
    }

    fn compound(&self, c: &Compound, pos: Pos) -> bool {
        let el = self.element(pos);
        if c.tag.as_ref().is_some_and(|t| !el.is(t, c.hash)) {
            return false;
        }
        c.conds.iter().all(|cond| self.cond(cond, el, pos))
    }

    fn cond(&self, cond: &Cond, el: &Element, pos: Pos) -> bool {
        match cond {
            Cond::Id(id) => el.attr("id") == Some(id.as_str()),
            Cond::Class(class) => el.has_class(class),
            Cond::Attr {
                name,
                op,
                value,
                ignore_case,
            } => {
                let ci = ignore_case.unwrap_or_else(|| case_insensitive(name));
                attr_matches(el.attr(name), *op, value, ci)
            }
            Cond::Nth {
                nth,
                of_type,
                last: false,
            } => {
                let (index, type_index) = self.index(pos);
                nth.matches(if *of_type { type_index } else { index })
            }
            Cond::Root => self.parent(pos).is_none(),
            Cond::Scope => match (self.scope, pos) {
                (Some(s), Pos::Frame(i)) => i == s,
                (Some(_), Pos::Sib { .. }) => false,
                (None, _) => self.parent(pos).is_none(),
            },
            Cond::Not(list) => !list.iter().any(|c| self.from(&c.parts, c.parts.len() - 1, pos)),
            Cond::Is(list) => list.iter().any(|c| self.from(&c.parts, c.parts.len() - 1, pos)),
            // Moved out at compile time.
            Cond::Nth { last: true, .. } | Cond::Only { .. } | Cond::Empty | Cond::Has(_) | Cond::Contains { .. } => {
                true
            }
        }
    }
}

/// Attributes whose values HTML compares without case (css-select's list).
fn case_insensitive(name: &str) -> bool {
    matches!(
        name,
        "accept"
            | "accept-charset"
            | "align"
            | "alink"
            | "axis"
            | "bgcolor"
            | "charset"
            | "checked"
            | "clear"
            | "codetype"
            | "color"
            | "compact"
            | "declare"
            | "defer"
            | "dir"
            | "direction"
            | "disabled"
            | "enctype"
            | "face"
            | "frame"
            | "hreflang"
            | "http-equiv"
            | "lang"
            | "language"
            | "link"
            | "media"
            | "method"
            | "multiple"
            | "nohref"
            | "noresize"
            | "noshade"
            | "nowrap"
            | "readonly"
            | "rel"
            | "rev"
            | "rules"
            | "scope"
            | "scrolling"
            | "selected"
            | "shape"
            | "target"
            | "text"
            | "type"
            | "valign"
            | "valuetype"
            | "vlink"
    )
}

fn attr_matches(actual: Option<&str>, op: AttrOp, want: &str, ci: bool) -> bool {
    if op == AttrOp::NotEquals {
        return !actual.is_some_and(|a| eq(a, want, ci));
    }
    let Some(a) = actual else {
        return false;
    };
    let lower;
    let (a, want) = if ci {
        lower = (a.to_lowercase(), want.to_lowercase());
        (lower.0.as_str(), lower.1.as_str())
    } else {
        (a, want)
    };
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => a == want,
        AttrOp::Includes => {
            !want.is_empty() && !want.contains(char::is_whitespace) && a.split_ascii_whitespace().any(|w| w == want)
        }
        AttrOp::Dash => a == want || (a.starts_with(want) && a.as_bytes().get(want.len()) == Some(&b'-')),
        AttrOp::Prefix => !want.is_empty() && a.starts_with(want),
        AttrOp::Suffix => !want.is_empty() && a.ends_with(want),
        AttrOp::Substring => !want.is_empty() && a.contains(want),
        AttrOp::NotEquals => unreachable!(),
    }
}

fn eq(a: &str, b: &str, ci: bool) -> bool {
    if ci {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests;
