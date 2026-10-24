//! RDFa 1.1 (Core, with HTML+RDFa's rules), processed as the document
//! streams by: the evaluation context is inherited down the element tree,
//! exactly the stack of open elements the reader keeps.
//!
//! Everything of RDFa Core § 7.5: `vocab`, `prefix` (and `xmlns:`), `lang`,
//! `about`, `resource`, `href`, `src`, `typeof`, `property`, `rel`, `rev`,
//! `content`, `datatype`, `inlist`; new subjects, typed resources, current
//! object resources, incomplete triples completed by descendants, lists
//! (`rdf:first`/`rdf:rest`), skipped elements; terms, CURIEs, safe CURIEs,
//! blank nodes, the initial context's prefixes and terms; typed, plain,
//! language-tagged and XML literals. HTML+RDFa: `head` and `body` take the
//! parent object as subject, `rel` values that are no CURIE are dropped
//! next to `property`, and `datetime` (and `<time>`) give typed literals by
//! their form (`xsd:date`, `xsd:dateTime`, `xsd:duration`, …).
//!
//! OpenGraph (`<meta property="og:title" content=…>`) and Dublin Core are
//! RDFa: they come out of this as statements about the page.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};
use url::Url;

use super::graph::{Format, Graph, Node, Val};
use super::vocab::{PREFIXES, RDF, XSD};
use crate::tree::Tree;

/// An RDF term in object position.
#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    /// An IRI or a blank node (`_:…`).
    Res(String),
    Lit {
        value: String,
        lang: Option<String>,
        datatype: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Triple {
    pub s: String,
    pub p: String,
    pub o: Obj,
}

#[derive(Clone, Debug)]
enum Item {
    Res(String),
    Lit(Obj),
    /// A literal its element's end will give.
    Pending(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dir {
    Forward,
    Reverse,
    /// Into a list.
    None,
}

#[derive(Clone, Debug)]
struct Incomplete {
    predicate: String,
    dir: Dir,
    list: usize,
}

/// A list mapping: predicate → list (an index into `lists`). One object, shared by the elements
/// of one subject: a list a child makes is its siblings' too (RDFa Core § 7.5 steps 8 and 14).
type ListMap = Rc<RefCell<Vec<(String, usize)>>>;

#[derive(Clone, Debug)]
struct Ctx {
    parent_subject: Option<String>,
    parent_object: Option<String>,
    incomplete: Rc<Vec<Incomplete>>,
    lists: ListMap,
    language: Option<Rc<str>>,
    prefixes: Rc<HashMap<String, String>>,
    vocab: Option<Rc<str>>,
    /// An `href`/`src` that is the parent subject and object, not yet resolved: an element with
    /// one and no RDFa sets the subject of its descendants, which most never use.
    raw_subject: Option<Rc<str>>,
}

/// A literal waiting for its element's text (or markup).
#[derive(Debug)]
struct Pending {
    subject: String,
    predicates: Vec<String>,
    lang: Option<String>,
    datatype: Option<String>,
    /// The literal's value when known now (`content`, `datetime`): no text needed.
    fixed: Option<String>,
    /// A `<time>` or `datetime` value: its datatype comes from its form.
    by_form: bool,
    /// An XML or HTML literal: the element's inner markup.
    markup: bool,
    text: String,
    /// The predicates of `predicates` that go into lists instead (`inlist`), with their list slots.
    slots: Vec<(usize, usize)>,
}

#[derive(Debug)]
struct Frame {
    /// The context for children (shared with the parent's when the element says nothing).
    child: Rc<Ctx>,
    subject: Option<String>,
    pending: Option<usize>,
    /// Where the markup capture of this element started.
    markup_from: usize,
    /// Its list mapping is its own (a new subject: step 8): its lists are made into statements when it ends.
    owns_lists: bool,
}

/// How RDFa is read.
#[derive(Clone, Copy, Debug)]
pub struct RdfaOptions {
    pub max_text: usize,
    /// At most this many triples (a guard against pathological pages).
    pub max_triples: usize,
    /// The vocabulary of terms (`typeof="Event"`, `property="name"`) where the page sets none
    /// (`vocab` forgotten): schema.org by default; none, as RDFa itself has it.
    pub default_vocab: Option<&'static str>,
}

impl Default for RdfaOptions {
    fn default() -> Self {
        RdfaOptions {
            max_text: 64 * 1024,
            max_triples: 100_000,
            default_vocab: Some("https://schema.org/"),
        }
    }
}

pub struct Rdfa {
    opts: RdfaOptions,
    base: String,
    /// `base`, parsed once.
    base_url: Option<Url>,
    /// Statements were left out at `max_triples`.
    pub truncated: bool,
    /// The document's list mapping (lists of the document as subject: made into statements at the end).
    root_lists: ListMap,
    pub triples: Vec<Triple>,
    frames: Vec<Frame>,
    lists: Vec<Vec<Item>>,
    pending: Vec<Pending>,
    capturing: Vec<usize>,
    markup: String,
    markup_depth: usize,
    blanks: usize,
    /// The one node `_:` and `[_:]` name.
    empty_blank: Option<String>,
}

const INITIAL_TERMS: &[(&str, &str)] = &[
    ("describedby", "http://www.w3.org/2007/05/powder-s#describedby"),
    ("license", "http://www.w3.org/1999/xhtml/vocab#license"),
    ("role", "http://www.w3.org/1999/xhtml/vocab#role"),
];

fn initial_prefixes() -> Rc<HashMap<String, String>> {
    static P: OnceLock<HashMap<String, String>> = OnceLock::new();
    Rc::new(
        P.get_or_init(|| PREFIXES.iter().map(|(p, i)| (p.to_string(), i.to_string())).collect())
            .clone(),
    )
}

fn datatype_by_form(v: &str) -> Option<&'static str> {
    static FORMS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let forms = FORMS.get_or_init(|| {
        let tz = r"(Z|[+-]\d{2}:?\d{2})?";
        [
            (
                r"^-?P(\d+Y)?(\d+M)?(\d+D)?(T(\d+H)?(\d+M)?(\d+(\.\d+)?S)?)?$".to_owned(),
                "duration",
            ),
            (
                format!(r"^-?\d{{4,}}-\d{{2}}-\d{{2}}T\d{{2}}:\d{{2}}(:\d{{2}}(\.\d+)?)?{tz}$"),
                "dateTime",
            ),
            (format!(r"^-?\d{{4,}}-\d{{2}}-\d{{2}}{tz}$"), "date"),
            (format!(r"^\d{{2}}:\d{{2}}(:\d{{2}}(\.\d+)?)?{tz}$"), "time"),
            (format!(r"^-?\d{{4,}}-\d{{2}}{tz}$"), "gYearMonth"),
            (format!(r"^-?\d{{4,}}{tz}$"), "gYear"),
        ]
        .into_iter()
        .filter_map(|(re, dt)| Regex::new(&re).ok().map(|r| (r, dt)))
        .collect()
    });
    if v.len() < 2 || v == "P" || v == "PT" {
        return None;
    }
    forms.iter().find(|(re, _)| re.is_match(v)).map(|(_, dt)| *dt)
}

impl Rdfa {
    pub fn new(base: Option<&Url>, opts: RdfaOptions) -> Rdfa {
        let base = base.map(|b| {
            let mut b = b.clone();
            b.set_fragment(None);
            b.to_string()
        });
        Rdfa {
            opts,
            base_url: base.as_deref().and_then(|b| Url::parse(b).ok()),
            base: base.unwrap_or_default(),
            truncated: false,
            root_lists: Rc::new(RefCell::new(Vec::new())),
            triples: Vec::new(),
            frames: Vec::new(),
            lists: Vec::new(),
            pending: Vec::new(),
            capturing: Vec::new(),
            markup: String::new(),
            markup_depth: 0,
            blanks: 0,
            empty_blank: None,
        }
    }

    /// The document's base changed (`<base href>`), before any statement used it.
    pub fn set_base(&mut self, base: &Url) {
        let mut b = base.clone();
        b.set_fragment(None);
        let new = b.to_string();
        self.base_url = Some(b);
        let old = std::mem::replace(&mut self.base, new.clone());
        if old == new {
            return;
        }
        // What was said of the document before its base was known is said of it now.
        let swap = |s: &mut String| {
            if *s == old {
                s.clone_from(&new);
            }
        };
        for t in &mut self.triples {
            swap(&mut t.s);
            if let Obj::Res(r) = &mut t.o {
                swap(r);
            }
        }
        for f in &mut self.frames {
            let child = Rc::make_mut(&mut f.child);
            for x in [&mut child.parent_subject, &mut child.parent_object]
                .into_iter()
                .flatten()
            {
                swap(x);
            }
            if let Some(x) = &mut f.subject {
                swap(x);
            }
        }
        for p in &mut self.pending {
            swap(&mut p.subject);
        }
    }

    pub fn wants_text(&self) -> bool {
        !self.capturing.is_empty()
    }

    pub fn wants_markup(&self) -> bool {
        self.markup_depth > 0
    }

    fn emit(&mut self, s: String, p: String, o: Obj) {
        if self.triples.len() < self.opts.max_triples {
            self.triples.push(Triple { s, p, o });
        } else {
            self.truncated = true;
        }
    }

    fn blank(&mut self) -> String {
        self.blanks += 1;
        format!("_:b{}", self.blanks)
    }

    /// An IRI reference resolved against the base as RFC 3986 has it (RDFa Core § 7.5 step 4…).
    fn resolve(&self, iri: &str) -> String {
        let iri = iri.trim();
        if self.base.is_empty() {
            return iri.to_owned();
        }
        super::iri::resolve(&self.base, iri)
    }

    /// A CURIE (`prefix:reference`, `_:label`, `:reference`) expanded; None when its prefix is unknown.
    fn curie(&mut self, value: &str, ctx_prefixes: &HashMap<String, String>, blank_ok: bool) -> Option<String> {
        let (prefix, reference) = value.split_once(':')?;
        if prefix == "_" {
            if !blank_ok {
                return None;
            }
            if reference.is_empty() {
                return Some(self.empty_blank.get_or_insert_with(|| "_:_".into()).clone());
            }
            return Some(format!("_:{reference}"));
        }
        if reference.starts_with("//") {
            return None;
        }
        if prefix.is_empty() {
            return Some(format!("http://www.w3.org/1999/xhtml/vocab#{reference}"));
        }
        ctx_prefixes
            .get(&prefix.to_ascii_lowercase())
            .map(|ns| format!("{ns}{reference}"))
    }

    /// `about`, `resource`: a safe CURIE, a CURIE, or an IRI (relative to the base).
    fn safe(&mut self, value: &str, ctx: &Ctx) -> Option<String> {
        let v = value.trim();
        if let Some(inner) = v.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            return self.curie(inner, &ctx.prefixes, true);
        }
        if let Some(c) = self.curie(v, &ctx.prefixes, true) {
            return Some(c);
        }
        Some(self.resolve(v))
    }

    /// `property`, `typeof`, `rel`, `rev`, `datatype`: a term, a CURIE, or an absolute IRI.
    /// `defaulted`: a bare term takes [`RdfaOptions::default_vocab`] when no vocabulary is in
    /// scope (`typeof` and `property`; `rel` values are HTML's link types, `stylesheet`, `icon`).
    fn term(&mut self, value: &str, ctx: &Ctx, defaulted: bool) -> Option<String> {
        if !value.contains(':') {
            if let Some((_, iri)) = INITIAL_TERMS.iter().find(|(t, _)| t.eq_ignore_ascii_case(value)) {
                if ctx.vocab.is_none() {
                    return Some((*iri).to_owned());
                }
            }
            return match &ctx.vocab {
                Some(v) => Some(format!("{v}{value}")),
                None => self
                    .opts
                    .default_vocab
                    .filter(|_| defaulted)
                    .map(|v| format!("{v}{value}")),
            };
        }
        if let Some(c) = self.curie(value, &ctx.prefixes, false) {
            return Some(c);
        }
        if value.starts_with("_:") {
            return None;
        }
        // An absolute IRI, not an undeclared prefix (`foo:bar` is a mistake, not a scheme).
        Url::parse(value)
            .ok()
            .filter(|u| u.has_host() || matches!(u.scheme(), "urn" | "tag" | "mailto"))
            .map(|_| value.to_owned())
    }

    fn terms(&mut self, values: Option<&str>, ctx: &Ctx, defaulted: bool) -> Vec<String> {
        values
            .unwrap_or("")
            .split_ascii_whitespace()
            .filter_map(|v| self.term(v, ctx, defaulted))
            .collect()
    }

    fn initial(&self) -> Ctx {
        Ctx {
            parent_subject: Some(self.base.clone()),
            parent_object: Some(self.base.clone()),
            incomplete: Rc::new(Vec::new()),
            lists: self.root_lists.clone(),
            language: None,
            prefixes: initial_prefixes(),
            vocab: None,
            raw_subject: None,
        }
    }

    pub fn open(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        let el = &tree.frames[depth].el;
        // An element without RDFa (most of a page): its children see what it sees.
        if let Some(parent) = self.frames.last() {
            let mut link = None;
            let says_something = el.attrs().any(|(n, v)| {
                if matches!(n, "href" | "src") {
                    if n == "href" || link.is_none() {
                        link = Some(v);
                    }
                    return false;
                }
                matches!(
                    n,
                    "vocab"
                        | "prefix"
                        | "lang"
                        | "xml:lang"
                        | "property"
                        | "rel"
                        | "rev"
                        | "typeof"
                        | "about"
                        | "resource"
                        | "content"
                        | "datatype"
                        | "inlist"
                ) || n.starts_with("xmlns:")
            });
            // An `href` alone completes its parent's open statements (a `rel` above it): read in full then.
            let plain = !says_something
                && !matches!(el.name(), "head" | "body")
                && (link.is_none() || parent.child.incomplete.is_empty());
            if plain {
                let child = match link {
                    None => parent.child.clone(),
                    Some(raw) => Rc::new(Ctx {
                        raw_subject: Some(raw.into()),
                        ..(*parent.child).clone()
                    }),
                };
                let subject = child.parent_subject.clone();
                self.frames.push(Frame {
                    child,
                    subject,
                    pending: None,
                    markup_from: self.markup.len(),
                    owns_lists: false,
                });
                return;
            }
        }
        let mut ctx = self
            .frames
            .last()
            .map_or_else(|| self.initial(), |f| (*f.child).clone());
        if let Some(raw) = ctx.raw_subject.take() {
            let subject = self.resolve(&raw);
            ctx.parent_subject = Some(subject.clone());
            ctx.parent_object = Some(subject);
        }
        let name = el.name();
        // The document's root is `<html>`; a page without one (a fragment) has none, and its
        // top-level `typeof` elements are subjects of their own, not the page.
        let is_root = depth == 0 && name == "html";
        let attr = |a: &str| el.attr(a);

        // Local values.
        let mut prefixes = ctx.prefixes.clone();
        let mut language = ctx.language.clone();
        let mut vocab = ctx.vocab.clone();
        if let Some(v) = attr("vocab") {
            let v = v.trim();
            if v.is_empty() {
                vocab = None;
            } else {
                let iri = self.resolve(v);
                self.emit(
                    self.base.clone(),
                    "http://www.w3.org/ns/rdfa#usesVocabulary".into(),
                    Obj::Res(iri.clone()),
                );
                vocab = Some(iri.into());
            }
        }
        let mut more = Vec::new();
        for (n, v) in el.attrs() {
            if let Some(p) = n.strip_prefix("xmlns:") {
                more.push((p.to_ascii_lowercase(), v.trim().to_owned()));
            }
        }
        if let Some(p) = attr("prefix") {
            let mut parts = p.split_ascii_whitespace();
            while let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                if let Some(k) = k.strip_suffix(':') {
                    if !k.is_empty() && k != "_" {
                        more.push((k.to_ascii_lowercase(), v.to_owned()));
                    }
                }
            }
        }
        if !more.is_empty() {
            let p = Rc::make_mut(&mut prefixes);
            for (k, v) in more {
                p.insert(k, v);
            }
        }
        if let Some(l) = attr("lang").or_else(|| attr("xml:lang")) {
            language = (!l.trim().is_empty()).then(|| l.trim().to_ascii_lowercase().into());
        }
        let local = Ctx {
            prefixes: prefixes.clone(),
            vocab: vocab.clone(),
            language: language.clone(),
            ..ctx.clone()
        };

        let has_property = attr("property").is_some();
        let mut rel_attr = attr("rel").map(str::to_owned);
        let mut rev_attr = attr("rev").map(str::to_owned);
        // HTML+RDFa: next to `property`, `rel`/`rev` values that are no CURIE or IRI are dropped.
        if has_property {
            for a in [&mut rel_attr, &mut rev_attr] {
                if let Some(v) = a.as_ref() {
                    let kept: Vec<&str> = v.split_ascii_whitespace().filter(|t| t.contains(':')).collect();
                    *a = (!kept.is_empty()).then(|| kept.join(" "));
                }
            }
        }
        let has_rel = rel_attr.is_some() || rev_attr.is_some();
        let has_typeof = attr("typeof").is_some();
        let about = attr("about").and_then(|v| self.safe(v, &local));
        // The spec asks whether `@about` is there, not whether it named something (`about="[]"`).
        let has_about = attr("about").is_some();
        let resource = attr("resource").and_then(|v| self.safe(v, &local));
        let href = attr("href").map(|v| self.resolve(v));
        let src = attr("src").map(|v| self.resolve(v));
        let content = attr("content").map(str::to_owned);
        let datatype_attr = attr("datatype").map(str::to_owned);

        let mut skip = false;
        let mut new_subject: Option<String>;
        let mut current_object: Option<String> = None;
        let mut typed: Option<String> = None;
        let head_or_body = matches!(name, "head" | "body");

        if !has_rel {
            if has_property && content.is_none() && datatype_attr.is_none() {
                new_subject = about.clone().or_else(|| {
                    if is_root {
                        Some(self.base.clone())
                    } else {
                        ctx.parent_object.clone()
                    }
                });
                if has_typeof {
                    typed = about
                        .clone()
                        .or_else(|| is_root.then(|| self.base.clone()))
                        .or_else(|| resource.clone().or_else(|| href.clone()).or_else(|| src.clone()));
                    if typed.is_none() {
                        typed = Some(self.blank());
                    }
                    current_object = typed.clone();
                }
            } else {
                new_subject = about
                    .clone()
                    .or_else(|| resource.clone())
                    .or_else(|| href.clone())
                    .or_else(|| src.clone());
                if new_subject.is_none() {
                    if is_root {
                        new_subject = Some(self.base.clone());
                    } else if head_or_body {
                        new_subject = ctx.parent_object.clone();
                    } else if has_typeof {
                        new_subject = Some(self.blank());
                    } else if ctx.parent_object.is_some() {
                        new_subject = ctx.parent_object.clone();
                        if !has_property {
                            skip = true;
                        }
                    }
                }
                if has_typeof {
                    typed = new_subject.clone();
                }
            }
        } else {
            new_subject = about.clone();
            if has_typeof {
                typed = new_subject.clone();
            }
            if new_subject.is_none() {
                new_subject = if is_root {
                    Some(self.base.clone())
                } else {
                    ctx.parent_object.clone()
                };
            }
            current_object = resource.clone().or_else(|| href.clone()).or_else(|| src.clone());
            if current_object.is_none() && has_typeof && !has_about {
                current_object = Some(self.blank());
            }
            if has_typeof && !has_about {
                typed = current_object.clone();
            }
        }

        // 7: types.
        if let Some(t) = &typed {
            let types = self.terms(attr("typeof"), &local, true);
            for ty in types {
                self.emit(t.clone(), format!("{RDF}type"), Obj::Res(ty));
            }
        }

        // 8: a new subject starts new lists.
        let owns_lists = new_subject.is_some() && new_subject != ctx.parent_object;
        let mut lists: ListMap = if owns_lists {
            Rc::new(RefCell::new(Vec::new()))
        } else {
            ctx.lists.clone()
        };
        let inlist = attr("inlist").is_some();
        let rels = self.terms(rel_attr.as_deref(), &local, false);
        let revs = self.terms(rev_attr.as_deref(), &local, false);
        let mut incomplete: Vec<Incomplete> = Vec::new();
        let subject = new_subject.clone();

        // 9 and 10: links.
        if let Some(obj) = &current_object {
            if let Some(s) = &subject {
                for p in &rels {
                    if inlist {
                        let l = self.list(&mut lists, p);
                        self.lists[l].push(Item::Res(obj.clone()));
                    } else {
                        self.emit(s.clone(), p.clone(), Obj::Res(obj.clone()));
                    }
                }
                for p in &revs {
                    self.emit(obj.clone(), p.clone(), Obj::Res(s.clone()));
                }
            }
        } else if !rels.is_empty() || !revs.is_empty() {
            current_object = Some(self.blank());
            for p in &rels {
                if inlist {
                    let l = self.list(&mut lists, p);
                    incomplete.push(Incomplete {
                        predicate: p.clone(),
                        dir: Dir::None,
                        list: l,
                    });
                } else {
                    incomplete.push(Incomplete {
                        predicate: p.clone(),
                        dir: Dir::Forward,
                        list: 0,
                    });
                }
            }
            for p in &revs {
                incomplete.push(Incomplete {
                    predicate: p.clone(),
                    dir: Dir::Reverse,
                    list: 0,
                });
            }
        }

        // 11: the property value.
        let mut pending = None;
        let properties = self.terms(attr("property"), &local, true);
        if let (false, Some(s)) = (properties.is_empty(), subject.clone()) {
            let datatype = match datatype_attr.as_deref() {
                Some(d) if d.trim().is_empty() => Some(None),
                Some(d) => Some(self.term(d.trim(), &local, false)),
                None => None,
            };
            let lang = language.as_deref().map(str::to_owned);
            let datetime = attr("datetime").map(str::to_owned);
            let xml =
                matches!(datatype, Some(Some(ref d)) if *d == format!("{RDF}XMLLiteral") || *d == format!("{RDF}HTML"));
            let mut value: Option<Item> = None;
            let mut literal: Option<Pending> = None;
            let mk =
                |fixed: Option<String>, datatype: Option<String>, lang: Option<String>, by_form: bool, markup: bool| {
                    Pending {
                        subject: s.clone(),
                        predicates: properties.clone(),
                        lang,
                        datatype,
                        fixed,
                        by_form,
                        markup,
                        text: String::new(),
                        slots: Vec::new(),
                    }
                };
            match datatype {
                Some(Some(dt)) if xml => literal = Some(mk(None, Some(dt), None, false, true)),
                Some(Some(dt)) => {
                    literal = Some(mk(content.clone().or(datetime.clone()), Some(dt), None, false, false))
                }
                Some(None) => literal = Some(mk(content.clone(), None, lang.clone(), false, false)),
                None => {
                    if let Some(c) = &content {
                        literal = Some(mk(Some(c.clone()), None, lang.clone(), false, false));
                    } else if let Some(dt) = datetime.clone().filter(|_| true) {
                        literal = Some(mk(Some(dt), None, lang.clone(), true, false));
                    } else if name == "time" {
                        literal = Some(mk(None, None, lang.clone(), true, false));
                    } else if !has_rel && (resource.is_some() || href.is_some() || src.is_some()) {
                        value = resource
                            .clone()
                            .or_else(|| href.clone())
                            .or_else(|| src.clone())
                            .map(Item::Res);
                    } else if has_typeof && !has_about {
                        value = typed.clone().map(Item::Res);
                    } else {
                        literal = Some(mk(None, None, lang.clone(), false, false));
                    }
                }
            }
            if let Some(v) = value {
                for p in &properties {
                    if inlist {
                        let l = self.list(&mut lists, p);
                        self.lists[l].push(v.clone());
                    } else if let Item::Res(o) = &v {
                        self.emit(s.clone(), p.clone(), Obj::Res(o.clone()));
                    }
                }
            }
            if let Some(mut lit) = literal {
                let id = self.pending.len();
                if inlist {
                    for (k, p) in properties.iter().enumerate() {
                        let l = self.list(&mut lists, p);
                        self.lists[l].push(Item::Pending(id));
                        lit.slots.push((k, l));
                    }
                }
                let needs_text = lit.fixed.is_none();
                let markup = lit.markup;
                self.pending.push(lit);
                pending = Some(id);
                if needs_text {
                    self.capturing.push(id);
                }
                if markup {
                    self.markup_depth += 1;
                }
            }
        }

        // 12: complete the parent's incomplete triples.
        if !skip {
            if let Some(s) = &subject {
                let parent_subject = ctx.parent_subject.clone();
                for inc in ctx.incomplete.iter() {
                    match inc.dir {
                        Dir::None => self.lists[inc.list].push(Item::Res(s.clone())),
                        Dir::Forward => {
                            if let Some(ps) = &parent_subject {
                                self.emit(ps.clone(), inc.predicate.clone(), Obj::Res(s.clone()));
                            }
                        }
                        Dir::Reverse => {
                            if let Some(ps) = &parent_subject {
                                self.emit(s.clone(), inc.predicate.clone(), Obj::Res(ps.clone()));
                            }
                        }
                    }
                }
            }
        }

        // 13: the children's context.
        let child = if skip {
            Ctx {
                language,
                prefixes,
                vocab,
                lists: lists.clone(),
                ..ctx.clone()
            }
        } else {
            Ctx {
                parent_subject: subject.clone().or_else(|| ctx.parent_subject.clone()),
                parent_object: current_object
                    .clone()
                    .or_else(|| subject.clone())
                    .or_else(|| ctx.parent_subject.clone()),
                incomplete: Rc::new(incomplete),
                lists: lists.clone(),
                language,
                prefixes,
                vocab,
                raw_subject: None,
            }
        };
        self.frames.push(Frame {
            child: Rc::new(child),
            subject: subject.or(ctx.parent_subject),
            pending,
            markup_from: self.markup.len(),
            owns_lists,
        });
    }

    /// The list of predicate `p` in `lists`, made when there is none.
    fn list(&mut self, lists: &mut ListMap, p: &str) -> usize {
        if let Some((_, l)) = lists.borrow().iter().find(|(x, _)| x == p) {
            return *l;
        }
        self.lists.push(Vec::new());
        let l = self.lists.len() - 1;
        lists.borrow_mut().push((p.to_owned(), l));
        l
    }

    pub fn text(&mut self, text: &str) {
        let max = self.opts.max_text;
        for &p in &self.capturing {
            let t = &mut self.pending[p].text;
            if t.len() < max {
                t.push_str(crate::text::cut(text, max - t.len()));
            }
        }
    }

    pub fn markup(&mut self, raw: &str) {
        if self.markup_depth > 0 && self.markup.len() < self.opts.max_text {
            self.markup.push_str(raw);
        }
    }

    pub fn close(&mut self, tree: &Tree) {
        let Some(frame) = self.frames.pop() else { return };
        if let Some(id) = frame.pending {
            if self.capturing.last() == Some(&id) {
                self.capturing.pop();
            }
            let markup = if self.pending[id].markup {
                self.markup_depth -= 1;
                // The element's inner markup: from after its start tag, without its end tag.
                let inner = self.markup[frame.markup_from.min(self.markup.len())..].to_owned();
                let name = &tree.frames[tree.frames.len() - 1].el.name();
                let end = inner
                    .rfind("</")
                    .filter(|&i| inner[i + 2..].to_ascii_lowercase().starts_with(name));
                let inner = end.map_or(inner.as_str(), |i| &inner[..i]).to_owned();
                if self.markup_depth == 0 {
                    self.markup.clear();
                }
                Some(inner)
            } else {
                None
            };
            self.finish_literal(id, markup);
        }
        // 14: an element whose list mapping is its own makes its lists into statements.
        if frame.owns_lists {
            if let Some(subject) = frame.subject.clone() {
                let lists: Vec<(String, usize)> = frame.child.lists.borrow().clone();
                for (p, l) in lists {
                    self.emit_list(&subject, &p, l);
                }
            }
        }
    }

    /// The end of the document: the lists of the document's own mapping.
    pub fn finish(&mut self) {
        let lists: Vec<(String, usize)> = std::mem::take(&mut *self.root_lists.borrow_mut());
        let base = self.base.clone();
        for (p, l) in lists {
            self.emit_list(&base, &p, l);
        }
    }

    /// List `l` of predicate `p` of `subject` as statements: `rdf:first`/`rdf:rest` cells, or `rdf:nil`.
    fn emit_list(&mut self, subject: &str, p: &str, l: usize) {
        let items: Vec<Obj> = self.lists[l]
            .iter()
            .filter_map(|i| match i {
                Item::Res(r) => Some(Obj::Res(r.clone())),
                Item::Lit(o) => Some(o.clone()),
                Item::Pending(_) => None,
            })
            .collect();
        if items.is_empty() {
            self.emit(subject.to_owned(), p.to_owned(), Obj::Res(format!("{RDF}nil")));
            return;
        }
        let heads: Vec<String> = items.iter().map(|_| self.blank()).collect();
        for (k, item) in items.into_iter().enumerate() {
            self.emit(heads[k].clone(), format!("{RDF}first"), item);
            let rest = heads.get(k + 1).cloned().unwrap_or_else(|| format!("{RDF}nil"));
            self.emit(heads[k].clone(), format!("{RDF}rest"), Obj::Res(rest));
        }
        self.emit(subject.to_owned(), p.to_owned(), Obj::Res(heads[0].clone()));
    }

    fn finish_literal(&mut self, id: usize, markup: Option<String>) {
        let p = &self.pending[id];
        let value = markup.or_else(|| p.fixed.clone()).unwrap_or_else(|| p.text.clone());
        let (lang, datatype) = if p.by_form {
            // The lexical form as it is (RDFa: `" 2012-03-18"` is no date).
            match datatype_by_form(&value) {
                Some(dt) => (None, Some(format!("{XSD}{dt}"))),
                None => (p.lang.clone(), None),
            }
        } else {
            (
                if p.datatype.is_some() { None } else { p.lang.clone() },
                p.datatype.clone(),
            )
        };
        let obj = Obj::Lit { value, lang, datatype };
        let subject = p.subject.clone();
        let predicates = p.predicates.clone();
        let slots = p.slots.clone();
        if slots.is_empty() {
            for pred in predicates {
                self.emit(subject.clone(), pred, obj.clone());
            }
        } else {
            for (_, l) in slots {
                for item in &mut self.lists[l] {
                    if matches!(item, Item::Pending(x) if *x == id) {
                        *item = Item::Lit(obj.clone());
                    }
                }
            }
        }
    }

    /// The statements as JSON: `{ subject, predicate, object: { type, value, language?, datatype? } }`.
    /// The statements after vocabulary expansion's property copying (HTML+RDFa 1.1 § 3.5, RDFa
    /// Core § 10.1): a resource that `rdfa:copy`s an `rdfa:Pattern` gets the pattern's statements
    /// (patterns may copy patterns: until nothing changes); then the copies and the patterns go.
    pub fn statements(&self) -> std::borrow::Cow<'_, [Triple]> {
        const COPY: &str = "http://www.w3.org/ns/rdfa#copy";
        const PATTERN: &str = "http://www.w3.org/ns/rdfa#Pattern";
        if !self.triples.iter().any(|t| t.p == COPY) {
            return std::borrow::Cow::Borrowed(&self.triples);
        }
        let rdf_type = format!("{RDF}type");
        let is_pattern_type = |t: &Triple| t.p == rdf_type && t.o == Obj::Res(PATTERN.into());
        let patterns: HashSet<String> = self
            .triples
            .iter()
            .filter(|t| is_pattern_type(t))
            .map(|t| t.s.clone())
            .collect();
        let mut out = self.triples.clone();
        let mut seen: HashSet<(String, String, String)> = out.iter().map(key).collect();
        for _ in 0..64 {
            let mut more = Vec::new();
            for c in out.iter().filter(|t| t.p == COPY) {
                let Obj::Res(pattern) = &c.o else { continue };
                if !patterns.contains(pattern) {
                    continue;
                }
                for t in out.iter().filter(|t| &t.s == pattern && !is_pattern_type(t)) {
                    let copy = Triple {
                        s: c.s.clone(),
                        p: t.p.clone(),
                        o: t.o.clone(),
                    };
                    if seen.insert(key(&copy)) {
                        more.push(copy);
                    }
                }
            }
            if more.is_empty() || out.len() + more.len() > self.opts.max_triples {
                break;
            }
            out.extend(more);
        }
        // The patterns some resource copies go, with the copy statements; one nobody copies stays.
        let copied: HashSet<String> = out
            .iter()
            .filter(|t| t.p == COPY)
            .filter_map(|t| match &t.o {
                Obj::Res(o) if patterns.contains(o) => Some(o.clone()),
                _ => None,
            })
            .collect();
        out.retain(|t| !(t.p == COPY && matches!(&t.o, Obj::Res(o) if copied.contains(o))) && !copied.contains(&t.s));
        std::borrow::Cow::Owned(out)
    }

    pub fn json(&self) -> Value {
        Value::Array(
            self.statements()
                .iter()
                .map(|t| {
                    let object = match &t.o {
                        Obj::Res(r) if r.starts_with("_:") => json!({ "type": "bnode", "value": r }),
                        Obj::Res(r) => json!({ "type": "iri", "value": r }),
                        Obj::Lit { value, lang, datatype } => {
                            let mut o = json!({ "type": "literal", "value": value });
                            if let Some(l) = lang {
                                o["language"] = json!(l);
                            }
                            if let Some(d) = datatype {
                                o["datatype"] = json!(d);
                            }
                            o
                        }
                    };
                    json!({ "subject": t.s, "predicate": t.p, "object": object })
                })
                .collect(),
        )
    }

    /// The statements as items: one per subject, lists as arrays, the vocabulary statement left out.
    pub fn into_graph(&self, graph: &mut Graph) {
        let statements = self.statements();
        let rdf_type = format!("{RDF}type");
        let first = format!("{RDF}first");
        let rest = format!("{RDF}rest");
        let nil = format!("{RDF}nil");
        let uses = "http://www.w3.org/ns/rdfa#usesVocabulary";
        // List cells.
        let mut cells: HashMap<&str, (Option<&Obj>, Option<&str>)> = HashMap::new();
        for t in statements.iter() {
            if t.p == first {
                cells.entry(&t.s).or_default().0 = Some(&t.o);
            } else if t.p == rest {
                if let Obj::Res(r) = &t.o {
                    cells.entry(&t.s).or_default().1 = Some(r);
                }
            }
        }
        let mut nodes: HashMap<&str, usize> = HashMap::new();
        let mut order: Vec<&str> = Vec::new();
        for t in statements.iter() {
            if t.p == uses || cells.contains_key(t.s.as_str()) {
                continue;
            }
            if !nodes.contains_key(t.s.as_str()) {
                let mut n = Node::new(Format::Rdfa);
                n.id = Some(t.s.clone());
                nodes.insert(&t.s, graph.add(n));
                order.push(&t.s);
            }
        }
        let mut objects: HashSet<&str> = HashSet::new();
        let val = |o: &Obj| -> Val {
            match o {
                Obj::Res(r) => Val::Iri(r.clone()),
                Obj::Lit {
                    value, lang: Some(l), ..
                } => Val::Lang(value.clone(), l.clone()),
                Obj::Lit {
                    value,
                    datatype: Some(d),
                    ..
                } => Val::Typed(value.clone(), d.clone()),
                Obj::Lit { value, .. } => Val::Text(value.clone()),
            }
        };
        for t in statements.iter() {
            // Processor notes and HTML's link types (`stylesheet`, `icon`, `alternate`) are no data;
            // a license is.
            if t.p == uses
                || t.p
                    .strip_prefix("http://www.w3.org/1999/xhtml/vocab#")
                    .is_some_and(|l| l != "license")
            {
                continue;
            }
            let Some(&i) = nodes.get(t.s.as_str()) else { continue };
            if t.p == rdf_type {
                if let Obj::Res(ty) = &t.o {
                    if !graph.nodes[i].types.contains(ty) {
                        graph.nodes[i].types.push(ty.clone());
                    }
                }
                continue;
            }
            // A list: its items, in order.
            if let Obj::Res(r) = &t.o {
                if r == &nil {
                    continue;
                }
                if cells.contains_key(r.as_str()) {
                    let mut at = Some(r.as_str());
                    let mut guard = 0;
                    while let Some(c) = at.and_then(|a| cells.get(a)) {
                        if let Some(o) = c.0 {
                            if let Obj::Res(x) = o {
                                objects.insert(x);
                            }
                            graph.nodes[i].push(&t.p, val(o));
                        }
                        at = c.1.filter(|x| *x != nil);
                        guard += 1;
                        if guard > 10_000 {
                            break;
                        }
                    }
                    continue;
                }
                if *r != t.s {
                    objects.insert(r);
                }
            }
            // `<meta property="og:image" content="/a.jpg">`: a URL, as the property means.
            if let Obj::Lit { value, .. } = &t.o {
                let v = value.trim();
                if !v.is_empty() && !v.contains(char::is_whitespace) && super::vocab::is_url_property(&t.p) {
                    graph.nodes[i].push(&t.p, Val::Iri(self.resolve(v)));
                    continue;
                }
            }
            graph.nodes[i].push(&t.p, val(&t.o));
        }
        for s in order {
            let i = nodes[s];
            graph.nodes[i].top = !objects.contains(s);
        }
    }
}

fn key(t: &Triple) -> (String, String, String) {
    let o = match &t.o {
        Obj::Res(r) => format!("<{r}>"),
        Obj::Lit { value, lang, datatype } => format!(
            "\"{value}\"@{}^^{}",
            lang.as_deref().unwrap_or(""),
            datatype.as_deref().unwrap_or("")
        ),
    };
    (t.s.clone(), t.p.clone(), o)
}

#[cfg(test)]
mod tests {
    use super::datatype_by_form;

    #[test]
    fn datetime_forms() {
        assert_eq!(datatype_by_form("2026-11-14"), Some("date"));
        assert_eq!(datatype_by_form("2026-11-14T20:00:00+01:00"), Some("dateTime"));
        assert_eq!(datatype_by_form("20:00:00"), Some("time"));
        assert_eq!(datatype_by_form("P1DT2H"), Some("duration"));
        assert_eq!(datatype_by_form("2026"), Some("gYear"));
        assert_eq!(datatype_by_form("2026-11"), Some("gYearMonth"));
        assert_eq!(datatype_by_form("next friday"), None);
    }
}
