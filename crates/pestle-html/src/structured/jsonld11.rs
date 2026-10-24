//! JSON-LD 1.1 Processing Algorithms (W3C Recommendation, 16 July 2020), as
//! written: Context Processing (§ 4.1), Create Term Definition (§ 4.2), IRI
//! Expansion (§ 5.2), Expansion (§ 5.1), Value Expansion (§ 5.3), Node Map
//! Generation (§ 7.2) and Deserialize JSON-LD to RDF (§ 8.1). Errors carry the
//! spec's error codes. Remote contexts come from a [`Loader`]: nothing is
//! fetched over the network here.
//!
//! Section numbers in comments are the spec's steps.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::iri;

/// A JSON-LD error, by its spec code (`invalid IRI mapping`, `colliding keywords` …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);

type R<T> = Result<T, Error>;

/// Remote documents (contexts) by URL: a document and the URL it came from.
pub trait Loader {
    fn load(&self, url: &str) -> Option<(Value, String)>;
}

/// No remote documents.
pub struct NoLoader;

impl Loader for NoLoader {
    fn load(&self, _: &str) -> Option<(Value, String)> {
        None
    }
}

const KEYWORDS: &[&str] = &[
    "@base",
    "@container",
    "@context",
    "@default",
    "@direction",
    "@embed",
    "@explicit",
    "@first",
    "@graph",
    "@id",
    "@import",
    "@included",
    "@index",
    "@json",
    "@language",
    "@list",
    "@nest",
    "@none",
    "@omitDefault",
    "@prefix",
    "@preserve",
    "@protected",
    "@requireAll",
    "@reverse",
    "@set",
    "@type",
    "@value",
    "@version",
    "@vocab",
    "@propagate",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// `@` followed by one or more ALPHA: reserved, ignored with a warning.
fn keyword_form(s: &str) -> bool {
    s.len() > 1 && s.starts_with('@') && s[1..].bytes().all(|c| c.is_ascii_alphabetic())
}

fn is_absolute(s: &str) -> bool {
    iri::is_absolute(s)
}

fn is_blank(s: &str) -> bool {
    s.starts_with("_:")
}

/// Ends with a gen-delim (`: / ? # [ ] @`): an IRI a term can be a prefix of.
fn ends_with_gen_delim(s: &str) -> bool {
    s.ends_with([':', '/', '?', '#', '[', ']', '@'])
}

#[derive(Clone, Debug, Default, PartialEq)]
struct TermDef {
    /// `None` for a term mapped to null (`"term": null`).
    iri: Option<String>,
    prefix: bool,
    protected: bool,
    reverse: bool,
    base_url: Option<String>,
    context: Option<Value>,
    container: Vec<String>,
    /// `Some(None)`: explicitly no direction.
    direction: Option<Option<String>>,
    index: Option<String>,
    /// `Some(None)`: explicitly no language.
    language: Option<Option<String>>,
    nest: Option<String>,
    type_mapping: Option<String>,
}

impl TermDef {
    fn has_container(&self, c: &str) -> bool {
        self.container.iter().any(|x| x == c)
    }

    /// Equal but for `protected` (§ 4.2.2 step 28).
    fn same_as(&self, other: &TermDef) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.protected = false;
        b.protected = false;
        a == b
    }
}

#[derive(Clone, Debug)]
pub struct Context {
    base: Option<String>,
    original_base: Option<String>,
    vocab: Option<String>,
    language: Option<String>,
    direction: Option<String>,
    previous: Option<Rc<Context>>,
    terms: HashMap<String, TermDef>,
}

impl Context {
    pub fn new(base: Option<&str>) -> Context {
        Context {
            base: base.map(str::to_owned),
            original_base: base.map(str::to_owned),
            vocab: None,
            language: None,
            direction: None,
            previous: None,
            terms: HashMap::new(),
        }
    }
}

/// What processing is asked: a base IRI, a loader, how many remote contexts deep.
pub struct Processor<'l> {
    loader: &'l dyn Loader,
    /// Remote contexts already loaded, by URL.
    cache: std::cell::RefCell<HashMap<String, (Value, String)>>,
}

const MAX_REMOTE: usize = 32;

impl<'l> Processor<'l> {
    pub fn new(loader: &'l dyn Loader) -> Processor<'l> {
        Processor {
            loader,
            cache: std::cell::RefCell::new(HashMap::new()),
        }
    }

    fn load(&self, url: &str) -> R<(Value, String)> {
        if let Some(d) = self.cache.borrow().get(url) {
            return Ok(d.clone());
        }
        let d = self.loader.load(url).ok_or(Error("loading remote context failed"))?;
        self.cache.borrow_mut().insert(url.to_owned(), d.clone());
        Ok(d)
    }

    // ── § 4.1.2 Context Processing ───────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn process_context(
        &self,
        active: &Context,
        local: &Value,
        base_url: Option<&str>,
        remote: &mut Vec<String>,
        override_protected: bool,
        mut propagate: bool,
        validate_scoped: bool,
    ) -> R<Context> {
        // 1.
        let mut result = active.clone();
        // 2.
        if let Some(p) = local.get("@propagate") {
            propagate = p.as_bool().ok_or(Error("invalid @propagate value"))?;
        }
        // 3.
        if !propagate && result.previous.is_none() {
            result.previous = Some(Rc::new(active.clone()));
        }
        // 4.
        let locals: Vec<&Value> = match local {
            Value::Array(a) => a.iter().collect(),
            v => vec![v],
        };
        // 5.
        for context in locals {
            match context {
                // 5.1
                Value::Null => {
                    if !override_protected && result.terms.values().any(|t| t.protected) {
                        return Err(Error("invalid context nullification"));
                    }
                    let previous = result.clone();
                    result = Context::new(active.original_base.as_deref());
                    if !propagate {
                        result.previous = Some(Rc::new(previous));
                    }
                }
                // 5.2
                Value::String(s) => {
                    let url = match base_url {
                        Some(b) => iri::resolve(b, s),
                        None if is_absolute(s) => s.clone(),
                        None => return Err(Error("loading remote context failed")),
                    };
                    if !validate_scoped && remote.contains(&url) {
                        continue;
                    }
                    if remote.len() >= MAX_REMOTE {
                        return Err(Error("context overflow"));
                    }
                    remote.push(url.clone());
                    let (doc, doc_url) = self.load(&url)?;
                    let ctx = doc
                        .get("@context")
                        .filter(|_| doc.is_object())
                        .ok_or(Error("invalid remote context"))?
                        .clone();
                    result =
                        self.process_context(&result, &ctx, Some(&doc_url), remote, false, true, validate_scoped)?;
                }
                // 5.3
                Value::Object(def) => {
                    let mut def = def.clone();
                    // 5.5
                    if let Some(v) = def.get("@version") {
                        if v.as_f64() != Some(1.1) {
                            return Err(Error("invalid @version value"));
                        }
                    }
                    // 5.6
                    if let Some(imp) = def.get("@import").cloned() {
                        let s = imp.as_str().ok_or(Error("invalid @import value"))?;
                        let url = match base_url {
                            Some(b) => iri::resolve(b, s),
                            None => s.to_owned(),
                        };
                        let (doc, _) = self.load(&url).map_err(|_| Error("loading remote context failed"))?;
                        let imported = doc
                            .get("@context")
                            .and_then(Value::as_object)
                            .ok_or(Error("invalid remote context"))?;
                        if imported.contains_key("@import") {
                            return Err(Error("invalid context entry"));
                        }
                        let mut merged = imported.clone();
                        for (k, v) in def.iter() {
                            merged.insert(k.clone(), v.clone());
                        }
                        def = merged;
                    }
                    // 5.7
                    if let (Some(b), true) = (def.get("@base"), remote.is_empty()) {
                        match b {
                            Value::Null => result.base = None,
                            Value::String(s) if is_absolute(s) => result.base = Some(s.clone()),
                            Value::String(s) => match &result.base {
                                Some(base) => result.base = Some(iri::resolve(base, s)),
                                None => return Err(Error("invalid base IRI")),
                            },
                            _ => return Err(Error("invalid base IRI")),
                        }
                    }
                    // 5.8
                    if let Some(v) = def.get("@vocab") {
                        match v {
                            Value::Null => result.vocab = None,
                            Value::String(s) => {
                                let e = self.iri(&result, s, true, true)?;
                                match e {
                                    Some(e) if is_absolute(&e) || is_blank(&e) || s.is_empty() || !s.contains(':') => {
                                        result.vocab = Some(e)
                                    }
                                    _ => return Err(Error("invalid vocab mapping")),
                                }
                            }
                            _ => return Err(Error("invalid vocab mapping")),
                        }
                    }
                    // 5.9
                    if let Some(v) = def.get("@language") {
                        result.language = match v {
                            Value::Null => None,
                            Value::String(s) => Some(s.to_ascii_lowercase()),
                            _ => return Err(Error("invalid default language")),
                        };
                    }
                    // 5.10
                    if let Some(v) = def.get("@direction") {
                        result.direction = match v {
                            Value::Null => None,
                            Value::String(s) if s == "ltr" || s == "rtl" => Some(s.clone()),
                            _ => return Err(Error("invalid base direction")),
                        };
                    }
                    // 5.11
                    if let Some(v) = def.get("@propagate") {
                        if !v.is_boolean() {
                            return Err(Error("invalid @propagate value"));
                        }
                    }
                    let protected = match def.get("@protected") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err(Error("invalid @protected value")),
                    };
                    // 5.12, 5.13
                    let mut defined: HashMap<String, bool> = HashMap::new();
                    let local_def = Value::Object(def.clone());
                    for key in def.keys() {
                        if matches!(
                            key.as_str(),
                            "@base"
                                | "@direction"
                                | "@import"
                                | "@language"
                                | "@propagate"
                                | "@protected"
                                | "@version"
                                | "@vocab"
                        ) {
                            continue;
                        }
                        self.create_term(
                            &mut result,
                            &local_def,
                            key,
                            &mut defined,
                            base_url,
                            protected,
                            override_protected,
                            remote,
                            validate_scoped,
                        )?;
                    }
                }
                _ => return Err(Error("invalid local context")),
            }
        }
        Ok(result)
    }

    // ── § 4.2.2 Create Term Definition ───────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn create_term(
        &self,
        active: &mut Context,
        local: &Value,
        term: &str,
        defined: &mut HashMap<String, bool>,
        base_url: Option<&str>,
        protected: bool,
        override_protected: bool,
        remote: &mut Vec<String>,
        // The spec's parameter, which Create Term Definition only passes on.
        _validate_scoped: bool,
    ) -> R<()> {
        // 1.
        match defined.get(term) {
            Some(true) => return Ok(()),
            Some(false) => return Err(Error("cyclic IRI mapping")),
            None => {}
        }
        // 2.
        if term.is_empty() {
            return Err(Error("invalid term definition"));
        }
        defined.insert(term.to_owned(), false);
        // 3.
        let value = local.get(term).cloned().unwrap_or(Value::Null);
        // 4.
        if term == "@type" {
            let ok = value.as_object().is_some_and(|o| {
                !o.is_empty()
                    && o.iter()
                        .all(|(k, v)| (k == "@container" && v == "@set") || (k == "@protected" && v.is_boolean()))
            });
            if !ok {
                return Err(Error("keyword redefinition"));
            }
        } else if is_keyword(term) {
            // 5.
            return Err(Error("keyword redefinition"));
        } else if keyword_form(term) {
            defined.insert(term.to_owned(), true);
            return Ok(());
        }
        // 6.
        let previous = active.terms.remove(term);
        // 7., 8., 9.
        let (value, simple) = match value {
            Value::Null => (json!({ "@id": null }), false),
            Value::String(s) => (json!({ "@id": s }), true),
            Value::Object(_) => (value, false),
            _ => return Err(Error("invalid term definition")),
        };
        let v = value.as_object().unwrap_or_else(|| unreachable!());
        // 10.
        let mut def = TermDef {
            prefix: false,
            ..TermDef::default()
        };
        // 11.
        def.protected = match v.get("@protected") {
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(Error("invalid @protected value")),
            None => protected,
        };
        // 12.
        if let Some(t) = v.get("@type") {
            let t = t.as_str().ok_or(Error("invalid type mapping"))?;
            let t = self
                .expand_iri_def(active, t, local, defined)?
                .ok_or(Error("invalid type mapping"))?;
            if !(matches!(t.as_str(), "@id" | "@json" | "@none" | "@vocab") || is_absolute(&t)) {
                return Err(Error("invalid type mapping"));
            }
            def.type_mapping = Some(t);
        }
        // 13.
        if let Some(rev) = v.get("@reverse") {
            if v.contains_key("@id") || v.contains_key("@nest") {
                return Err(Error("invalid reverse property"));
            }
            let rev = rev.as_str().ok_or(Error("invalid IRI mapping"))?;
            if keyword_form(rev) {
                defined.insert(term.to_owned(), true);
                return Ok(());
            }
            let i = self
                .expand_iri_def(active, rev, local, defined)?
                .ok_or(Error("invalid IRI mapping"))?;
            if !(is_absolute(&i) || is_blank(&i)) {
                return Err(Error("invalid IRI mapping"));
            }
            def.iri = Some(i);
            if let Some(c) = v.get("@container") {
                match c {
                    Value::Null => {}
                    Value::String(s) if s == "@set" || s == "@index" => def.container = vec![s.clone()],
                    _ => return Err(Error("invalid reverse property")),
                }
            }
            def.reverse = true;
            active.terms.insert(term.to_owned(), def);
            defined.insert(term.to_owned(), true);
            return Ok(());
        }
        // 15.
        match v.get("@id") {
            Some(id) if id.as_str() != Some(term) => match id {
                Value::Null => def.iri = None,
                Value::String(s) => {
                    if !is_keyword(s) && keyword_form(s) {
                        defined.insert(term.to_owned(), true);
                        return Ok(());
                    }
                    let i = self
                        .expand_iri_def(active, s, local, defined)?
                        .ok_or(Error("invalid IRI mapping"))?;
                    if !(is_keyword(&i) || is_absolute(&i) || is_blank(&i)) {
                        return Err(Error("invalid IRI mapping"));
                    }
                    if i == "@context" {
                        return Err(Error("invalid keyword alias"));
                    }
                    let inner = term.get(1..term.len().saturating_sub(1)).unwrap_or("");
                    if inner.contains(':') || term.contains('/') {
                        defined.insert(term.to_owned(), true);
                        let expanded = self.expand_iri_def(active, term, local, defined)?;
                        if expanded.as_deref() != Some(i.as_str()) {
                            return Err(Error("invalid IRI mapping"));
                        }
                    }
                    if !term.contains(':') && !term.contains('/') && simple && (ends_with_gen_delim(&i) || is_blank(&i))
                    {
                        def.prefix = true;
                    }
                    def.iri = Some(i);
                }
                _ => return Err(Error("invalid IRI mapping")),
            },
            _ => {
                if term[1..].contains(':') {
                    // 16. A compact IRI or an IRI.
                    let (prefix, suffix) = term.split_once(':').unwrap_or((term, ""));
                    if local.get(prefix).is_some() {
                        self.create_term(
                            active,
                            local,
                            prefix,
                            defined,
                            base_url,
                            false,
                            false,
                            remote,
                            _validate_scoped,
                        )?;
                    }
                    def.iri = Some(match active.terms.get(prefix).and_then(|t| t.iri.clone()) {
                        Some(p) => format!("{p}{suffix}"),
                        None => term.to_owned(),
                    });
                } else if term.contains('/') {
                    // 17.
                    let i = self
                        .iri(active, term, false, true)?
                        .ok_or(Error("invalid IRI mapping"))?;
                    if !is_absolute(&i) {
                        return Err(Error("invalid IRI mapping"));
                    }
                    def.iri = Some(i);
                } else if term == "@type" {
                    // 18.
                    def.iri = Some("@type".into());
                } else if let Some(vocab) = &active.vocab {
                    // 19.
                    def.iri = Some(format!("{vocab}{term}"));
                } else {
                    return Err(Error("invalid IRI mapping"));
                }
            }
        }
        // 20.
        if let Some(c) = v.get("@container") {
            let list: Vec<String> = match c {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a
                    .iter()
                    .map(|x| x.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or(Error("invalid container mapping"))?,
                Value::Null => Vec::new(),
                _ => return Err(Error("invalid container mapping")),
            };
            if !valid_container(&list) {
                return Err(Error("invalid container mapping"));
            }
            if list.iter().any(|x| x == "@type") {
                match def.type_mapping.as_deref() {
                    None => def.type_mapping = Some("@id".into()),
                    Some("@id" | "@vocab") => {}
                    Some(_) => return Err(Error("invalid type mapping")),
                }
            }
            def.container = list;
        }
        // 21.
        if let Some(i) = v.get("@index") {
            if !def.has_container("@index") {
                return Err(Error("invalid term definition"));
            }
            let i = i.as_str().ok_or(Error("invalid term definition"))?;
            let e = self.iri(active, i, false, true)?;
            if is_keyword(i) || !e.as_deref().is_some_and(is_absolute) {
                return Err(Error("invalid term definition"));
            }
            def.index = Some(i.to_owned());
        }
        // 22.
        if let Some(ctx) = v.get("@context") {
            let mut r = remote.clone();
            self.process_context(active, ctx, base_url, &mut r, true, true, false)
                .map_err(|_| Error("invalid scoped context"))?;
            def.context = Some(ctx.clone());
            def.base_url = base_url.map(str::to_owned);
        }
        // 23.
        if let (Some(l), false) = (v.get("@language"), v.contains_key("@type")) {
            def.language = Some(match l {
                Value::Null => None,
                Value::String(s) => Some(s.to_ascii_lowercase()),
                _ => return Err(Error("invalid language mapping")),
            });
        }
        // 24.
        if let (Some(d), false) = (v.get("@direction"), v.contains_key("@type")) {
            def.direction = Some(match d {
                Value::Null => None,
                Value::String(s) if s == "ltr" || s == "rtl" => Some(s.clone()),
                _ => return Err(Error("invalid base direction")),
            });
        }
        // 25.
        if let Some(n) = v.get("@nest") {
            let n = n.as_str().ok_or(Error("invalid @nest value"))?;
            if is_keyword(n) && n != "@nest" {
                return Err(Error("invalid @nest value"));
            }
            def.nest = Some(n.to_owned());
        }
        // 26.
        if let Some(p) = v.get("@prefix") {
            if term.contains(':') || term.contains('/') {
                return Err(Error("invalid term definition"));
            }
            def.prefix = p.as_bool().ok_or(Error("invalid @prefix value"))?;
            if def.prefix && def.iri.as_deref().is_some_and(is_keyword) {
                return Err(Error("invalid term definition"));
            }
        }
        // 27.
        for k in v.keys() {
            if !matches!(
                k.as_str(),
                "@id"
                    | "@reverse"
                    | "@container"
                    | "@context"
                    | "@direction"
                    | "@index"
                    | "@language"
                    | "@nest"
                    | "@prefix"
                    | "@protected"
                    | "@type"
            ) {
                return Err(Error("invalid term definition"));
            }
        }
        // 28.
        if !override_protected {
            if let Some(prev) = &previous {
                if prev.protected {
                    if !def.same_as(prev) {
                        return Err(Error("protected term redefinition"));
                    }
                    def = prev.clone();
                }
            }
        }
        // 29.
        active.terms.insert(term.to_owned(), def);
        defined.insert(term.to_owned(), true);
        Ok(())
    }

    // ── § 5.2.2 IRI Expansion ────────────────────────────────────────────

    /// IRI Expansion, `vocab` and `document relative` as asked.
    fn iri(&self, active: &Context, value: &str, document_relative: bool, vocab: bool) -> R<Option<String>> {
        // 1.
        if is_keyword(value) {
            return Ok(Some(value.to_owned()));
        }
        // 2.
        if keyword_form(value) {
            return Ok(None);
        }
        // 4.
        if let Some(t) = active.terms.get(value) {
            if t.iri.as_deref().is_some_and(is_keyword) {
                return Ok(t.iri.clone());
            }
        }
        // 5.
        if vocab {
            if let Some(t) = active.terms.get(value) {
                return Ok(t.iri.clone());
            }
        }
        // 6.
        if value.len() > 1 && value[1..].contains(':') {
            let (prefix, suffix) = value.split_once(':').unwrap_or((value, ""));
            if prefix == "_" || suffix.starts_with("//") {
                return Ok(Some(value.to_owned()));
            }
            if let Some(t) = active.terms.get(prefix) {
                if let (Some(i), true) = (&t.iri, t.prefix) {
                    return Ok(Some(format!("{i}{suffix}")));
                }
            }
            if is_absolute(value) {
                return Ok(Some(value.to_owned()));
            }
        }
        // 7.
        if vocab {
            if let Some(v) = &active.vocab {
                return Ok(Some(format!("{v}{value}")));
            }
        }
        // 8.
        if document_relative {
            if let Some(b) = &active.base {
                return Ok(Some(iri::resolve(b, value)));
            }
        }
        // 9.
        Ok(Some(value.to_owned()))
    }

    /// IRI Expansion from Create Term Definition (vocab true, not document relative): the terms of
    /// `local` it needs are defined in `active` first (steps 3 and 6.2).
    fn expand_iri_def(
        &self,
        active: &mut Context,
        value: &str,
        local: &Value,
        defined: &mut HashMap<String, bool>,
    ) -> R<Option<String>> {
        if is_keyword(value) || keyword_form(value) {
            return self.iri(active, value, false, true);
        }
        // 3.
        if local.get(value).is_some() && defined.get(value) != Some(&true) {
            self.create_term(active, local, value, defined, None, false, false, &mut Vec::new(), true)?;
        }
        // 6.2
        if let Some((prefix, suffix)) = value
            .get(1..)
            .and_then(|v| v.find(':'))
            .map(|i| (&value[..=i], &value[i + 2..]))
        {
            if prefix != "_"
                && !suffix.starts_with("//")
                && local.get(prefix).is_some()
                && defined.get(prefix) != Some(&true)
            {
                self.create_term(
                    active,
                    local,
                    prefix,
                    defined,
                    None,
                    false,
                    false,
                    &mut Vec::new(),
                    true,
                )?;
            }
        }
        self.iri(active, value, false, true)
    }

    // ── § 5.1.2 Expansion ────────────────────────────────────────────────

    pub fn expand_document(&self, input: &Value, base: Option<&str>) -> R<Vec<Value>> {
        let active = Context::new(base);
        let e = self.expand(&active, None, input, base, false)?;
        Ok(match e {
            Value::Object(o) if o.len() == 1 && o.contains_key("@graph") => as_array(o["@graph"].clone()),
            Value::Null => Vec::new(),
            Value::Array(a) => a,
            v => vec![v],
        })
    }

    fn expand(
        &self,
        active: &Context,
        prop: Option<&str>,
        element: &Value,
        base_url: Option<&str>,
        from_map: bool,
    ) -> R<Value> {
        // 1.
        if element.is_null() {
            return Ok(Value::Null);
        }
        // 3.
        let term = prop.and_then(|p| active.terms.get(p));
        let scoped = term.and_then(|t| t.context.clone());
        let scoped_base = term.and_then(|t| t.base_url.clone());
        match element {
            // 4.
            Value::Array(items) => {
                // 5.
                let mut out = Vec::new();
                for item in items {
                    let mut e = self.expand(active, prop, item, base_url, from_map)?;
                    if term.is_some_and(|t| t.has_container("@list")) && e.is_array() {
                        e = json!({ "@list": e });
                    }
                    match e {
                        Value::Array(a) => out.extend(a),
                        Value::Null => {}
                        v => out.push(v),
                    }
                }
                Ok(Value::Array(out))
            }
            Value::Object(map) => self.expand_map(
                active,
                prop,
                map,
                base_url,
                from_map,
                scoped.as_ref(),
                scoped_base.as_deref(),
            ),
            scalar => {
                // 4.1
                if prop.is_none() || prop == Some("@graph") {
                    return Ok(Value::Null);
                }
                // 4.2
                let active = match &scoped {
                    Some(ctx) => {
                        self.process_context(active, ctx, scoped_base.as_deref(), &mut Vec::new(), false, true, true)?
                    }
                    None => active.clone(),
                };
                // 4.3
                self.expand_value(&active, prop.unwrap_or_default(), scalar)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_map(
        &self,
        active_in: &Context,
        prop: Option<&str>,
        element: &Map<String, Value>,
        base_url: Option<&str>,
        from_map: bool,
        scoped: Option<&Value>,
        scoped_base: Option<&str>,
    ) -> R<Value> {
        let mut active = active_in.clone();
        // 7.
        if let Some(prev) = &active.previous {
            let keys: Vec<Option<String>> = element
                .keys()
                .map(|k| self.iri(&active, k, false, true).ok().flatten())
                .collect();
            let has_value = keys.iter().any(|k| k.as_deref() == Some("@value"));
            let only_id = keys.len() == 1 && keys[0].as_deref() == Some("@id");
            if !from_map && !has_value && !only_id {
                active = (**prev).clone();
            }
        }
        // 8.
        if let Some(ctx) = scoped {
            active = self.process_context(&active, ctx, scoped_base, &mut Vec::new(), true, true, true)?;
        }
        // 9.
        if let Some(ctx) = element.get("@context") {
            active = self.process_context(&active, ctx, base_url, &mut Vec::new(), false, true, true)?;
        }
        // 10.
        let type_scoped = active.clone();
        // 11.
        let mut keys: Vec<&String> = element.keys().collect();
        keys.sort();
        let mut input_type: Option<String> = None;
        for key in &keys {
            if self.iri(&active, key, false, true)?.as_deref() != Some("@type") {
                continue;
            }
            let mut values: Vec<&str> = match &element[key.as_str()] {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if input_type.is_none() {
                if let Some(last) = values.last() {
                    input_type = self.iri(&active, last, false, true)?;
                }
            }
            values.sort();
            for t in values {
                if let Some(ctx) = type_scoped.terms.get(t).and_then(|d| d.context.clone()) {
                    let b = type_scoped.terms.get(t).and_then(|d| d.base_url.clone());
                    active = self.process_context(&active, &ctx, b.as_deref(), &mut Vec::new(), false, false, true)?;
                }
            }
        }
        // 12.
        let mut result = Map::new();
        let mut nests: Vec<String> = Vec::new();
        self.expand_entries(
            &active,
            &type_scoped,
            prop,
            element,
            base_url,
            input_type.as_deref(),
            &mut result,
            &mut nests,
        )?;
        // 14.
        self.expand_nests(
            &active,
            &type_scoped,
            prop,
            element,
            &nests,
            base_url,
            input_type.as_deref(),
            &mut result,
        )?;
        // 15.
        if result.contains_key("@value") {
            let allowed = result
                .keys()
                .all(|k| matches!(k.as_str(), "@direction" | "@index" | "@language" | "@type" | "@value"));
            let has_type = result.contains_key("@type");
            if !allowed || (has_type && (result.contains_key("@language") || result.contains_key("@direction"))) {
                return Err(Error("invalid value object"));
            }
            if result.get("@type").and_then(Value::as_str) == Some("@json") {
                // Anything goes.
            } else if result["@value"].is_null() {
                return Ok(Value::Null);
            } else if !result["@value"].is_string() && result.contains_key("@language") {
                return Err(Error("invalid language-tagged value"));
            } else if let Some(t) = result.get("@type") {
                if !t
                    .as_str()
                    .is_some_and(|t| is_absolute(t) && !t.contains(char::is_whitespace))
                {
                    return Err(Error("invalid typed value"));
                }
            }
        } else if let Some(t) = result.get("@type") {
            // 16.
            if !t.is_array() {
                let t = t.clone();
                result.insert("@type".into(), json!([t]));
            }
        } else if result.contains_key("@set") || result.contains_key("@list") {
            // 17.
            if result.len() > 2 || (result.len() == 2 && !result.contains_key("@index")) {
                return Err(Error("invalid set or list object"));
            }
            if let Some(s) = result.get("@set") {
                return Ok(s.clone());
            }
        }
        // An `@id` that expanded to null: a node of nothing else is dropped, else it is a blank node.
        if result.get("@id").is_some_and(Value::is_null) {
            if result.len() == 1 {
                return Ok(Value::Null);
            }
            result.remove("@id");
        }
        // 18.
        if result.len() == 1 && result.contains_key("@language") {
            return Ok(Value::Null);
        }
        // 19.
        if prop.is_none() || prop == Some("@graph") {
            if result.is_empty() || result.contains_key("@value") || result.contains_key("@list") {
                return Ok(Value::Null);
            }
            if result.len() == 1 && result.contains_key("@id") {
                return Ok(Value::Null);
            }
        }
        Ok(Value::Object(result))
    }

    /// § 5.1.2 step 14: the entries of nested values into `result`, each nest under its term's
    /// scoped context, nests inside them too.
    #[allow(clippy::too_many_arguments)]
    fn expand_nests(
        &self,
        active: &Context,
        type_scoped: &Context,
        prop: Option<&str>,
        element: &Map<String, Value>,
        nests: &[String],
        base_url: Option<&str>,
        input_type: Option<&str>,
        result: &mut Map<String, Value>,
    ) -> R<()> {
        for key in nests {
            let scoped;
            let nest_ctx = match active
                .terms
                .get(key.as_str())
                .and_then(|t| t.context.as_ref().map(|c| (c, t.base_url.clone())))
            {
                Some((ctx, b)) => {
                    scoped = self.process_context(active, ctx, b.as_deref(), &mut Vec::new(), true, true, true)?;
                    &scoped
                }
                None => active,
            };
            for nested in as_array(element.get(key.as_str()).cloned().unwrap_or(Value::Null)) {
                let Value::Object(nm) = nested else {
                    return Err(Error("invalid @nest value"));
                };
                for k in nm.keys() {
                    if self.iri(nest_ctx, k, false, true)?.as_deref() == Some("@value") {
                        return Err(Error("invalid @nest value"));
                    }
                }
                let mut more = Vec::new();
                self.expand_entries(
                    nest_ctx,
                    type_scoped,
                    prop,
                    &nm,
                    base_url,
                    input_type,
                    result,
                    &mut more,
                )?;
                self.expand_nests(nest_ctx, type_scoped, prop, &nm, &more, base_url, input_type, result)?;
            }
        }
        Ok(())
    }

    /// § 5.1.2 step 13: one map's entries into `result`.
    #[allow(clippy::too_many_arguments)]
    fn expand_entries(
        &self,
        active: &Context,
        type_scoped: &Context,
        prop: Option<&str>,
        element: &Map<String, Value>,
        base_url: Option<&str>,
        input_type: Option<&str>,
        result: &mut Map<String, Value>,
        nests: &mut Vec<String>,
    ) -> R<()> {
        let mut keys: Vec<&String> = element.keys().collect();
        keys.sort();
        for key in keys {
            let value = &element[key.as_str()];
            // 13.1
            if key == "@context" {
                continue;
            }
            // 13.2, 13.3
            let Some(expanded) = self.iri(active, key, false, true)? else {
                continue;
            };
            if !expanded.contains(':') && !is_keyword(&expanded) {
                continue;
            }
            // 13.4
            if is_keyword(&expanded) {
                if prop == Some("@reverse") {
                    return Err(Error("invalid reverse property map"));
                }
                if result.contains_key(&expanded) && !matches!(expanded.as_str(), "@included" | "@type") {
                    return Err(Error("colliding keywords"));
                }
                let ev: Value = match expanded.as_str() {
                    "@id" => {
                        let s = value.as_str().ok_or(Error("invalid @id value"))?;
                        self.iri(active, s, true, false)?
                            .map(Value::String)
                            .unwrap_or(Value::Null)
                    }
                    "@type" => {
                        let items: Vec<&str> = match value {
                            Value::String(s) => vec![s],
                            Value::Array(a) => a
                                .iter()
                                .map(Value::as_str)
                                .collect::<Option<_>>()
                                .ok_or(Error("invalid type value"))?,
                            _ => return Err(Error("invalid type value")),
                        };
                        let mut out: Vec<Value> = Vec::new();
                        if let Some(prev) = result.get("@type") {
                            out.extend(as_array(prev.clone()));
                        }
                        for t in items {
                            if let Some(e) = self.iri(type_scoped, t, true, true)? {
                                out.push(Value::String(e));
                            }
                        }
                        if out.len() == 1 && !value.is_array() && !result.contains_key("@type") {
                            out.pop().unwrap_or(Value::Null)
                        } else {
                            Value::Array(out)
                        }
                    }
                    "@graph" => Value::Array(as_array(self.expand(active, Some("@graph"), value, base_url, false)?)),
                    "@included" => {
                        // As a property's values, so strings and value objects are kept to be refused.
                        let items = as_array(self.expand(active, Some("@included"), value, base_url, false)?);
                        for i in &items {
                            let node = i.as_object().is_some_and(|o| {
                                !o.contains_key("@value") && !o.contains_key("@list") && !o.contains_key("@set")
                            });
                            if !node {
                                return Err(Error("invalid @included value"));
                            }
                        }
                        let mut all = result.get("@included").cloned().map(as_array).unwrap_or_default();
                        all.extend(items);
                        Value::Array(all)
                    }
                    "@value" => {
                        if input_type == Some("@json") {
                            value.clone()
                        } else {
                            if value.is_object() || value.is_array() {
                                return Err(Error("invalid value object value"));
                            }
                            if value.is_null() {
                                result.insert("@value".into(), Value::Null);
                                continue;
                            }
                            value.clone()
                        }
                    }
                    "@language" => {
                        let s = value.as_str().ok_or(Error("invalid language-tagged string"))?;
                        Value::String(s.to_ascii_lowercase())
                    }
                    "@direction" => match value.as_str() {
                        Some("ltr" | "rtl") => value.clone(),
                        _ => return Err(Error("invalid base direction")),
                    },
                    "@index" => {
                        if !value.is_string() {
                            return Err(Error("invalid @index value"));
                        }
                        value.clone()
                    }
                    "@list" => {
                        if prop.is_none() || prop == Some("@graph") {
                            continue;
                        }
                        Value::Array(as_array(self.expand(active, prop, value, base_url, false)?))
                    }
                    "@set" => self.expand(active, prop, value, base_url, false)?,
                    "@reverse" => {
                        if !value.is_object() {
                            return Err(Error("invalid @reverse value"));
                        }
                        let ev = self.expand(active, Some("@reverse"), value, base_url, false)?;
                        let Value::Object(ev) = ev else { continue };
                        if let Some(Value::Object(rr)) = ev.get("@reverse") {
                            for (p, items) in rr {
                                add_value(result, p, items.clone());
                            }
                        }
                        if ev.keys().any(|k| k != "@reverse") {
                            let mut rmap = result
                                .get("@reverse")
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default();
                            for (p, items) in ev.iter().filter(|(k, _)| *k != "@reverse") {
                                for item in as_array(items.clone()) {
                                    if is_value_or_list(&item) {
                                        return Err(Error("invalid reverse property value"));
                                    }
                                    add_value(&mut rmap, p, item);
                                }
                            }
                            result.insert("@reverse".into(), Value::Object(rmap));
                        }
                        continue;
                    }
                    "@nest" => {
                        nests.push(key.clone());
                        continue;
                    }
                    _ => continue,
                };
                // An `@id` that expands to null (`@ignoreMe`) is kept as null until step 19.
                result.insert(expanded, ev);
                continue;
            }
            let term = active.terms.get(key.as_str());
            // 13.5
            let container: Vec<String> = term.map(|t| t.container.clone()).unwrap_or_default();
            let has = |c: &str| container.iter().any(|x| x == c);
            let mut ev: Value = if term.and_then(|t| t.type_mapping.as_deref()) == Some("@json") {
                // 13.6
                json!({ "@value": value, "@type": "@json" })
            } else if has("@language") && value.is_object() {
                // 13.7
                let mut out = Vec::new();
                let direction = match term.and_then(|t| t.direction.clone()) {
                    Some(d) => d,
                    None => active.direction.clone(),
                };
                let mut langs: Vec<(&String, &Value)> =
                    value.as_object().map(|o| o.iter().collect()).unwrap_or_default();
                langs.sort_by(|a, b| a.0.cmp(b.0));
                for (lang, lv) in langs {
                    for item in as_array(lv.clone()) {
                        if item.is_null() {
                            continue;
                        }
                        let Value::String(s) = item else {
                            return Err(Error("invalid language map value"));
                        };
                        let mut v = Map::new();
                        v.insert("@value".into(), Value::String(s));
                        let none = lang == "@none" || self.iri(active, lang, false, true)?.as_deref() == Some("@none");
                        if !none {
                            v.insert("@language".into(), Value::String(lang.to_ascii_lowercase()));
                        }
                        if let Some(d) = &direction {
                            v.insert("@direction".into(), Value::String(d.clone()));
                        }
                        out.push(Value::Object(v));
                    }
                }
                Value::Array(out)
            } else if (has("@index") || has("@type") || has("@id")) && value.is_object() {
                // 13.8
                let mut out = Vec::new();
                let index_key = term.and_then(|t| t.index.clone()).unwrap_or_else(|| "@index".into());
                let mut entries: Vec<(&String, &Value)> =
                    value.as_object().map(|o| o.iter().collect()).unwrap_or_default();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                for (index, iv) in entries {
                    let mut map_context = if has("@id") || has("@type") {
                        active.previous.as_deref().cloned().unwrap_or_else(|| active.clone())
                    } else {
                        active.clone()
                    };
                    if has("@type") {
                        if let Some(ctx) = map_context.terms.get(index.as_str()).and_then(|t| t.context.clone()) {
                            let b = map_context.terms.get(index.as_str()).and_then(|t| t.base_url.clone());
                            map_context = self.process_context(
                                &map_context,
                                &ctx,
                                b.as_deref(),
                                &mut Vec::new(),
                                false,
                                true,
                                true,
                            )?;
                        }
                    } else if !has("@id") {
                        map_context = active.clone();
                    }
                    let expanded_index = self.iri(active, index, false, true)?;
                    let items = as_array(self.expand(
                        &map_context,
                        Some(key),
                        &Value::Array(as_array(iv.clone())),
                        base_url,
                        true,
                    )?);
                    for mut item in items {
                        if has("@graph") && !is_graph(&item) {
                            item = json!({ "@graph": [item] });
                        }
                        let none = expanded_index.as_deref() == Some("@none");
                        if has("@index") && index_key != "@index" && !none {
                            let re = self.expand_value(active, &index_key, &Value::String(index.clone()))?;
                            let ek = self.iri(active, &index_key, false, true)?.unwrap_or_default();
                            let o = item.as_object_mut().ok_or(Error("invalid value object"))?;
                            if o.contains_key("@value") {
                                return Err(Error("invalid value object"));
                            }
                            let mut vals = vec![re];
                            if let Some(prev) = o.remove(&ek) {
                                vals.extend(as_array(prev));
                            }
                            o.insert(ek, Value::Array(vals));
                        } else if has("@index") && !none && item.get("@index").is_none() {
                            if let Some(o) = item.as_object_mut() {
                                o.insert("@index".into(), Value::String(index.clone()));
                            }
                        } else if has("@id") && !none && item.get("@id").is_none() {
                            let id = self.iri(active, index, true, false)?;
                            if let (Some(o), Some(id)) = (item.as_object_mut(), id) {
                                o.insert("@id".into(), Value::String(id));
                            }
                        } else if has("@type") && !none {
                            let t = self.iri(active, index, true, true)?.unwrap_or_default();
                            if let Some(o) = item.as_object_mut() {
                                let mut types = vec![Value::String(t)];
                                if let Some(prev) = o.remove("@type") {
                                    types.extend(as_array(prev));
                                }
                                o.insert("@type".into(), Value::Array(types));
                            }
                        }
                        out.push(item);
                    }
                }
                Value::Array(out)
            } else {
                // 13.9
                self.expand(active, Some(key), value, base_url, false)?
            };
            // 13.10
            if ev.is_null() {
                continue;
            }
            // 13.11
            if has("@list") && !ev.get("@list").is_some() {
                ev = json!({ "@list": as_array(ev) });
            }
            // 13.12
            if has("@graph") && !has("@id") && !has("@index") {
                ev = Value::Array(
                    as_array(ev)
                        .into_iter()
                        .map(|v| json!({ "@graph": as_array(v) }))
                        .collect(),
                );
            }
            // 13.13
            if term.is_some_and(|t| t.reverse) {
                let mut rmap = result
                    .get("@reverse")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for item in as_array(ev) {
                    if is_value_or_list(&item) {
                        return Err(Error("invalid reverse property value"));
                    }
                    add_value(&mut rmap, &expanded, item);
                }
                result.insert("@reverse".into(), Value::Object(rmap));
            } else {
                // 13.14
                add_value(result, &expanded, ev);
            }
        }
        Ok(())
    }

    // ── § 5.3.2 Value Expansion ──────────────────────────────────────────

    fn expand_value(&self, active: &Context, prop: &str, value: &Value) -> R<Value> {
        let term = active.terms.get(prop);
        let tm = term.and_then(|t| t.type_mapping.as_deref());
        if let Value::String(s) = value {
            if tm == Some("@id") {
                return Ok(json!({ "@id": self.iri(active, s, true, false)? }));
            }
            if tm == Some("@vocab") {
                return Ok(json!({ "@id": self.iri(active, s, true, true)? }));
            }
        }
        let mut out = Map::new();
        out.insert("@value".into(), value.clone());
        match tm {
            Some(t) if !matches!(t, "@id" | "@vocab" | "@none") => {
                out.insert("@type".into(), Value::String(t.to_owned()));
            }
            _ => {
                if value.is_string() {
                    let language = match term.and_then(|t| t.language.clone()) {
                        Some(l) => l,
                        None => active.language.clone(),
                    };
                    let direction = match term.and_then(|t| t.direction.clone()) {
                        Some(d) => d,
                        None => active.direction.clone(),
                    };
                    if let Some(l) = language {
                        out.insert("@language".into(), Value::String(l));
                    }
                    if let Some(d) = direction {
                        out.insert("@direction".into(), Value::String(d));
                    }
                }
            }
        }
        Ok(Value::Object(out))
    }
}

fn valid_container(list: &[String]) -> bool {
    let has = |c: &str| list.iter().any(|x| x == c);
    let known = list.iter().all(|c| {
        matches!(
            c.as_str(),
            "@graph" | "@id" | "@index" | "@language" | "@list" | "@set" | "@type"
        )
    });
    if !known {
        return false;
    }
    match list.len() {
        0 | 1 => true,
        _ => {
            if has("@list") {
                return false;
            }
            if has("@graph") {
                return list
                    .iter()
                    .all(|c| matches!(c.as_str(), "@graph" | "@id" | "@index" | "@set"))
                    && !(has("@id") && has("@index"));
            }
            if has("@set") {
                return list.len() == 2
                    && list
                        .iter()
                        .any(|c| matches!(c.as_str(), "@index" | "@id" | "@type" | "@language"));
            }
            false
        }
    }
}

fn as_array(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Null => Vec::new(),
        v => vec![v],
    }
}

fn is_value_or_list(v: &Value) -> bool {
    v.get("@value").is_some() || v.get("@list").is_some()
}

fn is_graph(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        o.contains_key("@graph") && o.keys().all(|k| matches!(k.as_str(), "@graph" | "@id" | "@index"))
    })
}

/// Adds `value` (or each of an array's items) to `map[key]`, always an array.
fn add_value(map: &mut Map<String, Value>, key: &str, value: Value) {
    let entry = map.entry(key.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        let prev = entry.take();
        *entry = Value::Array(vec![prev]);
    }
    if let Value::Array(a) = entry {
        match value {
            Value::Array(items) => a.extend(items),
            v => a.push(v),
        }
    }
}

// ── § 7.2.2 Node Map Generation ──────────────────────────────────────────

/// Graph name → subject → node.
pub type NodeMap = BTreeMap<String, BTreeMap<String, Map<String, Value>>>;

#[derive(Default)]
struct Issuer {
    map: HashMap<String, String>,
    next: usize,
    /// The nodes stated at the top of a graph (no property's value), in order.
    tops: Vec<String>,
}

impl Issuer {
    fn id(&mut self, old: Option<&str>) -> String {
        if let Some(o) = old {
            if let Some(n) = self.map.get(o) {
                return n.clone();
            }
        }
        let n = format!("_:b{}", self.next);
        self.next += 1;
        if let Some(o) = old {
            self.map.insert(o.to_owned(), n.clone());
        }
        n
    }
}

pub fn node_map(expanded: &[Value]) -> R<NodeMap> {
    Ok(flatten(expanded)?.0)
}

/// The node map, and the ids of the nodes stated at the top of a graph rather than as a value.
pub fn flatten(expanded: &[Value]) -> R<(NodeMap, Vec<String>)> {
    let mut map = NodeMap::new();
    map.insert("@default".into(), BTreeMap::new());
    let mut issuer = Issuer::default();
    for e in expanded {
        generate(e, &mut map, "@default", None, None, None, &mut issuer)?;
    }
    Ok((map, issuer.tops))
}

/// The subject a node map entry is written under (or a reverse node, as `Subject::Node`).
enum Subject<'a> {
    Id(&'a str),
    Node(&'a Value),
}

fn push_unique(node: &mut Map<String, Value>, prop: &str, v: Value) {
    let entry = node.entry(prop.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(a) = entry {
        if !a.contains(&v) {
            a.push(v);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn generate(
    element: &Value,
    map: &mut NodeMap,
    graph: &str,
    subject: Option<Subject<'_>>,
    prop: Option<&str>,
    list: Option<&mut Vec<Value>>,
    issuer: &mut Issuer,
) -> R<()> {
    // 1.
    if let Value::Array(items) = element {
        let mut list = list;
        for item in items {
            let s = match &subject {
                Some(Subject::Id(s)) => Some(Subject::Id(s)),
                Some(Subject::Node(n)) => Some(Subject::Node(n)),
                None => None,
            };
            generate(item, map, graph, s, prop, list.as_deref_mut(), issuer)?;
        }
        return Ok(());
    }
    let Value::Object(el) = element else { return Ok(()) };
    map.entry(graph.to_owned()).or_default();
    let mut el = el.clone();
    // 3.
    if let Some(t) = el.get_mut("@type") {
        let relabelled: Vec<Value> = as_array(t.clone())
            .into_iter()
            .map(|x| match x.as_str() {
                Some(s) if is_blank(s) => Value::String(issuer.id(Some(s))),
                _ => x,
            })
            .collect();
        *t = if t.is_array() {
            Value::Array(relabelled)
        } else {
            relabelled.into_iter().next().unwrap_or(Value::Null)
        };
    }
    // 4.
    if el.contains_key("@value") {
        let v = Value::Object(el);
        match list {
            None => {
                if let (Some(Subject::Id(s)), Some(p)) = (subject, prop) {
                    if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(s)) {
                        push_unique(node, p, v);
                    }
                }
            }
            Some(l) => l.push(v),
        }
        return Ok(());
    }
    // 5.
    if let Some(items) = el.get("@list").cloned() {
        let mut inner = Vec::new();
        let s = match &subject {
            Some(Subject::Id(s)) => Some(Subject::Id(s)),
            Some(Subject::Node(n)) => Some(Subject::Node(n)),
            None => None,
        };
        generate(&items, map, graph, s, prop, Some(&mut inner), issuer)?;
        let result = json!({ "@list": inner });
        match list {
            None => {
                if let (Some(Subject::Id(s)), Some(p)) = (subject, prop) {
                    if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(s)) {
                        let entry = node.entry(p.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
                        if let Value::Array(a) = entry {
                            a.push(result);
                        }
                    }
                }
            }
            Some(l) => l.push(result),
        }
        return Ok(());
    }
    // 6.
    let id = match el.remove("@id").and_then(|v| v.as_str().map(str::to_owned)) {
        Some(i) if is_blank(&i) => issuer.id(Some(&i)),
        Some(i) => i,
        None => issuer.id(None),
    };
    {
        let g = map.get_mut(graph).ok_or(Error("internal"))?;
        g.entry(id.clone()).or_insert_with(|| {
            let mut m = Map::new();
            m.insert("@id".into(), Value::String(id.clone()));
            m
        });
    }
    match (&subject, prop) {
        // 6.5 (reverse): the active subject node is the value.
        (Some(Subject::Node(n)), Some(p)) => {
            let n = (*n).clone();
            if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(&id)) {
                push_unique(node, p, n);
            }
        }
        (Some(Subject::Id(s)), Some(p)) => {
            let reference = json!({ "@id": id });
            match list {
                None => {
                    if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(*s)) {
                        push_unique(node, p, reference);
                    }
                }
                Some(l) => l.push(reference),
            }
        }
        (None, _) | (_, None) => match list {
            Some(l) => l.push(json!({ "@id": id })),
            None => issuer.tops.push(id.clone()),
        },
    }
    // 6.6
    if let Some(types) = el.remove("@type") {
        if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(&id)) {
            for t in as_array(types) {
                push_unique(node, "@type", t);
            }
        }
    }
    // 6.7
    if let Some(index) = el.remove("@index") {
        if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(&id)) {
            if let Some(prev) = node.get("@index") {
                if *prev != index {
                    return Err(Error("conflicting indexes"));
                }
            }
            node.insert("@index".into(), index);
        }
    }
    // 6.8
    if let Some(Value::Object(rev)) = el.remove("@reverse") {
        let referenced = json!({ "@id": id });
        for (p, values) in rev {
            for v in as_array(values) {
                generate(&v, map, graph, Some(Subject::Node(&referenced)), Some(&p), None, issuer)?;
            }
        }
    }
    // 6.9
    if let Some(g) = el.remove("@graph") {
        generate(&g, map, &id, None, None, None, issuer)?;
    }
    // 6.10
    if let Some(inc) = el.remove("@included") {
        generate(&inc, map, graph, None, None, None, issuer)?;
    }
    // 6.11
    let mut props: Vec<(String, Value)> = el.into_iter().collect();
    props.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, v) in props {
        let p = if is_blank(&p) { issuer.id(Some(&p)) } else { p };
        if let Some(node) = map.get_mut(graph).and_then(|g| g.get_mut(&id)) {
            node.entry(p.clone()).or_insert_with(|| Value::Array(Vec::new()));
        }
        generate(&v, map, graph, Some(Subject::Id(&id)), Some(&p), None, issuer)?;
    }
    Ok(())
}

// ── § 8.1.2 Deserialize JSON-LD to RDF ───────────────────────────────────

/// An RDF term.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Term {
    Iri(String),
    Blank(String),
    Literal {
        value: String,
        datatype: String,
        language: Option<String>,
    },
}

/// A quad: subject, predicate, object, graph (none: the default graph).
pub type Quad = (Term, Term, Term, Option<Term>);

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn well_formed(s: &str) -> Option<Term> {
    if is_blank(s) {
        Some(Term::Blank(s.to_owned()))
    } else if is_absolute(s)
        && !s.contains(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\'))
        && s.matches('#').count() <= 1
    {
        Some(Term::Iri(s.to_owned()))
    } else {
        None
    }
}

pub fn to_rdf(map: &NodeMap) -> Vec<Quad> {
    let mut out = Vec::new();
    let mut bnodes = 0usize;
    for (graph_name, graph) in map {
        let g = if graph_name == "@default" {
            None
        } else {
            match well_formed(graph_name) {
                Some(t) => Some(t),
                None => continue,
            }
        };
        for (subject, node) in graph {
            let Some(s) = well_formed(subject) else { continue };
            let mut props: Vec<(&String, &Value)> = node.iter().collect();
            props.sort_by(|a, b| a.0.cmp(b.0));
            for (p, values) in props {
                if p == "@type" {
                    for t in as_array(values.clone()) {
                        if let Some(t) = t.as_str().and_then(well_formed) {
                            out.push((s.clone(), Term::Iri(format!("{RDF}type")), t, g.clone()));
                        }
                    }
                    continue;
                }
                if is_keyword(p) || is_blank(p) {
                    continue;
                }
                let Some(pred @ Term::Iri(_)) = well_formed(p) else {
                    continue;
                };
                for item in as_array(values.clone()) {
                    let mut list_quads = Vec::new();
                    if let Some(o) = object_to_rdf(&item, &mut list_quads, &mut bnodes, &g) {
                        out.push((s.clone(), pred.clone(), o, g.clone()));
                        out.extend(list_quads);
                    }
                }
            }
        }
    }
    out
}

fn object_to_rdf(item: &Value, list_quads: &mut Vec<Quad>, bnodes: &mut usize, g: &Option<Term>) -> Option<Term> {
    // A node object or reference.
    if let Some(id) = item.get("@id").filter(|_| item.get("@value").is_none()) {
        return well_formed(id.as_str()?);
    }
    if let Some(Value::Array(items)) = item.get("@list") {
        return Some(list_to_rdf(items, list_quads, bnodes, g));
    }
    let value = item.get("@value")?;
    let mut datatype = item.get("@type").and_then(Value::as_str).map(str::to_owned);
    if let Some(dt) = &datatype {
        if dt != "@json" && well_formed(dt).is_none() {
            return None;
        }
    }
    if let Some(l) = item.get("@language").and_then(Value::as_str) {
        if !well_formed_language(l) {
            return None;
        }
    }
    let lexical = match value {
        _ if datatype.as_deref() == Some("@json") => {
            datatype = Some(format!("{RDF}JSON"));
            canonical_json(value)
        }
        Value::Bool(b) => {
            datatype.get_or_insert_with(|| format!("{XSD}boolean"));
            b.to_string()
        }
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            // § 8.6.2 step 10: a non-zero fractional part or ≥ 1e21 is a double (10.0 is 10).
            let integral = n.is_i64() || n.is_u64() || (f.fract() == 0.0 && f.abs() < 1e21);
            if !integral || datatype.as_deref() == Some(&format!("{XSD}double")) {
                datatype.get_or_insert_with(|| format!("{XSD}double"));
                canonical_double(f)
            } else {
                datatype.get_or_insert_with(|| format!("{XSD}integer"));
                if let Some(i) = n.as_i64() {
                    i.to_string()
                } else if let Some(u) = n.as_u64() {
                    u.to_string()
                } else {
                    format!("{:.0}", f + 0.0) // -0 is 0
                }
            }
        }
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let language = item.get("@language").and_then(Value::as_str).map(str::to_owned);
    let datatype = datatype.unwrap_or_else(|| {
        if language.is_some() {
            format!("{RDF}langString")
        } else {
            format!("{XSD}string")
        }
    });
    Some(Term::Literal {
        value: lexical,
        datatype,
        language,
    })
}

fn list_to_rdf(items: &[Value], quads: &mut Vec<Quad>, bnodes: &mut usize, g: &Option<Term>) -> Term {
    if items.is_empty() {
        return Term::Iri(format!("{RDF}nil"));
    }
    let heads: Vec<Term> = items
        .iter()
        .map(|_| {
            *bnodes += 1;
            Term::Blank(format!("_:l{}", *bnodes))
        })
        .collect();
    for (k, item) in items.iter().enumerate() {
        let mut inner = Vec::new();
        if let Some(o) = object_to_rdf(item, &mut inner, bnodes, g) {
            quads.push((heads[k].clone(), Term::Iri(format!("{RDF}first")), o, g.clone()));
        }
        let rest = heads
            .get(k + 1)
            .cloned()
            .unwrap_or_else(|| Term::Iri(format!("{RDF}nil")));
        quads.push((heads[k].clone(), Term::Iri(format!("{RDF}rest")), rest, g.clone()));
        quads.extend(inner);
    }
    heads[0].clone()
}

/// BCP 47 in its simple form (`en`, `en-US`, `zh-Hant-TW`).
fn well_formed_language(l: &str) -> bool {
    let mut parts = l.split('-');
    let first = parts.next().unwrap_or("");
    (1..=8).contains(&first.len())
        && first.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric()))
}

/// XSD canonical double: `1.1E0`, `1.0E1`, `-4.2E-3`.
fn canonical_double(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0E0".into()
        } else {
            "0.0E0".into()
        };
    }
    let s = format!("{f:E}");
    let (mantissa, exp) = s.split_once('E').unwrap_or((&s, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_owned()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{exp}")
}

/// RFC 8785 JSON canonicalization (keys sorted, numbers in their ECMAScript form).
fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical_json(&o[k.as_str()])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical_json).collect::<Vec<_>>().join(",")),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => es_number(n.as_f64().unwrap_or(0.0)),
        },
        other => other.to_string(),
    }
}

/// ECMAScript's Number::toString (ECMA-262 § 6.1.6.1.20), which RFC 8785 serializes numbers as.
fn es_number(f: f64) -> String {
    if f == 0.0 || !f.is_finite() {
        return "0".into();
    }
    // The shortest digits that round-trip, and the exponent n with value = 0.digits × 10^n.
    let e = format!("{:e}", f.abs());
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap_or(0) + 1;
    let mut out = String::from(if f < 0.0 { "-" } else { "" });
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n > 0 { '+' } else { '-' });
        out.push_str(&(n - 1).abs().to_string());
    }
    out
}

/// N-Quads text of quads.
pub fn nquads(quads: &[Quad]) -> String {
    fn term(t: &Term) -> String {
        match t {
            Term::Iri(i) => format!("<{i}>"),
            Term::Blank(b) => b.clone(),
            Term::Literal {
                value,
                datatype,
                language,
            } => {
                let mut esc = String::with_capacity(value.len() + 2);
                for c in value.chars() {
                    match c {
                        '\\' => esc.push_str("\\\\"),
                        '"' => esc.push_str("\\\""),
                        '\n' => esc.push_str("\\n"),
                        '\r' => esc.push_str("\\r"),
                        c => esc.push(c),
                    }
                }
                match language {
                    Some(l) => format!("\"{esc}\"@{l}"),
                    None if datatype == "http://www.w3.org/2001/XMLSchema#string" => format!("\"{esc}\""),
                    None => format!("\"{esc}\"^^<{datatype}>"),
                }
            }
        }
    }
    let mut out = String::new();
    for (s, p, o, g) in quads {
        out.push_str(&term(s));
        out.push(' ');
        out.push_str(&term(p));
        out.push(' ');
        out.push_str(&term(o));
        if let Some(g) = g {
            out.push(' ');
            out.push_str(&term(g));
        }
        out.push_str(" .\n");
    }
    out
}

/// A JSON-LD document to RDF, as the spec's `toRdf`: N-Quads, or the error.
pub fn document_to_rdf(input: &Value, base: Option<&str>, loader: &dyn Loader) -> R<String> {
    let p = Processor::new(loader);
    let expanded = p.expand_document(input, base)?;
    let map = node_map(&expanded)?;
    Ok(nquads(&to_rdf(&map)))
}
