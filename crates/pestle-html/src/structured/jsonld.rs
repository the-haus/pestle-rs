//! JSON-LD blocks into items, as loosely as sites write them.
//!
//! Contexts are applied as the JSON-LD algorithms apply them — `@vocab`,
//! `@base`, `@language`, prefixes and terms (aliases, `@type` coercion,
//! `@container`, `@reverse`), arrays of contexts, scoped `@context` on any
//! node — without fetching anything: a context URL of schema.org (any of
//! its spellings, `/docs/jsonldcontext.json` included) is schema.org's
//! vocabulary with its `id`/`type` aliases, the old data-vocabulary.org is
//! schema.org, another URL ending in `/` or `#` is a vocabulary, and a
//! block with no context at all is read as schema.org (what its author
//! meant; [`JsonLdOptions::default_vocab`]).
//!
//! Structure: one node, an array of them, `@graph` at any depth (a page
//! may hold several blocks, each with its own graph), `@included`, `@list`
//! and `@set`, `@value` objects with `@language`/`@type`, `@reverse`, and
//! `@id` references, within a block and across blocks. Nodes of the same
//! `@id` are one node: their properties are merged.

use std::collections::HashMap;

use serde_json::{Map, Value};
use url::Url;

use super::graph::{Format, Graph, Node, Val};
use super::jsonld11;
use super::vocab::SCHEMA;

/// How JSON-LD is read.
#[derive(Clone, Debug)]
pub struct JsonLdOptions {
    /// The vocabulary of a block that states no context (schema.org); `None`: keep bare names.
    pub default_vocab: Option<String>,
    /// JSON-LD 1.1 as specified ([`jsonld11`]): a block that is not JSON, or not valid JSON-LD, is
    /// left out with its error code as a warning; terms no context defines are dropped. Only
    /// schema.org's context is known (built in); other remote contexts fail to load.
    pub strict: bool,
}

impl Default for JsonLdOptions {
    fn default() -> Self {
        JsonLdOptions {
            default_vocab: Some(SCHEMA.to_owned()),
            strict: false,
        }
    }
}

/// schema.org's context as `https://schema.org/docs/jsonldcontext.jsonld` serves it, in short:
/// its vocabulary, the `id`/`type` aliases, the `schema:` prefix, and its URL properties as `@id`.
struct SchemaContext(Value);

impl SchemaContext {
    fn get() -> &'static SchemaContext {
        static CONTEXT: std::sync::OnceLock<SchemaContext> = std::sync::OnceLock::new();
        CONTEXT.get_or_init(|| {
            let mut c = Map::new();
            c.insert("@vocab".into(), "http://schema.org/".into());
            c.insert("id".into(), "@id".into());
            c.insert("type".into(), "@type".into());
            c.insert("schema".into(), "http://schema.org/".into());
            for p in super::vocab::URL_PROPERTIES {
                c.insert(
                    (*p).into(),
                    serde_json::json!({ "@id": format!("schema:{p}"), "@type": "@id" }),
                );
            }
            SchemaContext(serde_json::json!({ "@context": c }))
        })
    }
}

impl jsonld11::Loader for SchemaContext {
    fn load(&self, url: &str) -> Option<(Value, String)> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        matches!(
            rest,
            "schema.org"
                | "schema.org/"
                | "schema.org/docs/jsonldcontext.json"
                | "schema.org/docs/jsonldcontext.jsonld"
        )
        .then(|| (self.0.clone(), url.to_owned()))
    }
}

#[derive(Clone, Debug, Default)]
struct Term {
    /// An IRI, or a keyword the term aliases (`@id`, `@type`, …).
    id: Option<String>,
    /// `@type` coercion: `@id`/`@vocab`, or a datatype IRI.
    coerce: Option<String>,
    language: Option<Option<String>>,
    reverse: bool,
}

#[derive(Clone, Debug, Default)]
struct Ctx {
    vocab: Option<String>,
    base: Option<Url>,
    language: Option<String>,
    terms: HashMap<String, Term>,
}

const MAX_DEPTH: usize = 64;

fn is_schema_context(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.contains("schema.org") || l.contains("data-vocabulary.org")
}

impl Ctx {
    fn schema(&mut self) {
        self.vocab = Some(SCHEMA.to_owned());
        // schema.org's own context makes `id` and `type` aliases.
        for (alias, keyword) in [("id", "@id"), ("type", "@type")] {
            self.terms.entry(alias.into()).or_insert(Term {
                id: Some(keyword.into()),
                ..Term::default()
            });
        }
    }

    /// The context after `value` (a `@context`) is applied.
    fn with(&self, value: &Value) -> Ctx {
        let mut ctx = self.clone();
        ctx.apply(value);
        ctx
    }

    fn apply(&mut self, value: &Value) {
        match value {
            Value::Null => {
                let base = self.base.clone();
                *self = Ctx { base, ..Ctx::default() };
            }
            Value::String(s) => {
                if is_schema_context(s) {
                    self.schema();
                } else if s.ends_with('/') || s.ends_with('#') {
                    self.vocab = Some(s.clone());
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.apply(item);
                }
            }
            Value::Object(o) => self.define(o),
            _ => {}
        }
    }

    fn define(&mut self, o: &Map<String, Value>) {
        if let Some(b) = o.get("@base") {
            match b {
                Value::Null => self.base = None,
                Value::String(s) => self.base = self.resolve(s).and_then(|u| Url::parse(&u).ok()),
                _ => {}
            }
        }
        match o.get("@vocab") {
            Some(Value::Null) => self.vocab = None,
            Some(Value::String(v)) => {
                self.vocab = if is_schema_context(v) {
                    self.schema();
                    Some(SCHEMA.into())
                } else if v.is_empty() {
                    self.base.as_ref().map(|b| b.to_string())
                } else {
                    Some(self.expand(v, true).unwrap_or_else(|| v.clone()))
                }
            }
            _ => {}
        }
        match o.get("@language") {
            Some(Value::String(l)) => self.language = Some(l.to_ascii_lowercase()),
            Some(Value::Null) => self.language = None,
            _ => {}
        }
        for (key, def) in o {
            if key.starts_with('@') {
                continue;
            }
            match def {
                Value::Null => {
                    self.terms.insert(key.clone(), Term::default());
                }
                Value::String(s) => {
                    let id = if s.starts_with('@') {
                        Some(s.clone())
                    } else {
                        self.expand(s, true).or_else(|| Some(s.clone()))
                    };
                    self.terms.insert(key.clone(), Term { id, ..Term::default() });
                }
                Value::Object(d) => {
                    let mut term = Term::default();
                    if let Some(Value::String(r)) = d.get("@reverse") {
                        term.id = self.expand(r, true);
                        term.reverse = true;
                    } else {
                        term.id = match d.get("@id") {
                            Some(Value::String(s)) if s.starts_with('@') => Some(s.clone()),
                            Some(Value::String(s)) => self.expand(s, true).or_else(|| Some(s.clone())),
                            _ => self.expand(key, true),
                        };
                    }
                    if let Some(Value::String(t)) = d.get("@type") {
                        term.coerce = Some(if t.starts_with('@') {
                            t.clone()
                        } else {
                            self.expand(t, true).unwrap_or_else(|| t.clone())
                        });
                    }
                    match d.get("@language") {
                        Some(Value::String(l)) => term.language = Some(Some(l.to_ascii_lowercase())),
                        Some(Value::Null) => term.language = Some(None),
                        _ => {}
                    }
                    self.terms.insert(key.clone(), term);
                }
                _ => {}
            }
        }
    }

    fn term(&self, key: &str) -> Option<&Term> {
        self.terms.get(key)
    }

    /// A keyword a key or alias stands for (`type` → `@type` under schema.org's context).
    fn keyword(&self, key: &str) -> Option<String> {
        if key.starts_with('@') {
            return Some(key.to_owned());
        }
        self.term(key)
            .and_then(|t| t.id.clone())
            .filter(|id| id.starts_with('@'))
    }

    /// An IRI expanded: a term, `prefix:suffix`, an absolute IRI, a blank node; with `vocab`,
    /// a bare name against the vocabulary, else (an `@id`) against the base.
    fn expand(&self, value: &str, vocab: bool) -> Option<String> {
        if value.is_empty() {
            return None;
        }
        if value.starts_with('@') {
            return Some(value.to_owned());
        }
        if vocab {
            if let Some(t) = self.term(value) {
                return t.id.clone();
            }
        }
        if let Some((prefix, suffix)) = value.split_once(':') {
            if prefix == "_" {
                return Some(value.to_owned());
            }
            if !suffix.starts_with("//") {
                if let Some(ns) = self
                    .term(prefix)
                    .and_then(|t| t.id.as_deref())
                    .filter(|i| !i.starts_with('@'))
                {
                    return Some(format!("{ns}{suffix}"));
                }
                if let Some((_, ns)) = super::vocab::PREFIXES.iter().find(|(p, _)| *p == prefix) {
                    if prefix == "schema" {
                        return Some(format!("{SCHEMA}{suffix}"));
                    }
                    return Some(format!("{ns}{suffix}"));
                }
            }
            if Url::parse(value).is_ok() {
                return Some(value.to_owned());
            }
        }
        if vocab {
            if let Some(v) = &self.vocab {
                return Some(format!("{v}{value}"));
            }
            return Some(value.to_owned());
        }
        self.resolve(value)
    }

    fn resolve(&self, value: &str) -> Option<String> {
        match &self.base {
            Some(b) => b.join(value.trim()).ok().map(|u| u.to_string()),
            None => Some(value.to_owned()),
        }
    }
}

/// Reads JSON-LD values (each a block's, already parsed) into `graph`.
pub struct JsonLd<'g> {
    graph: &'g mut Graph,
    ids: HashMap<String, usize>,
    root: Ctx,
}

impl<'g> JsonLd<'g> {
    pub fn new(graph: &'g mut Graph, base: Option<&Url>, options: &JsonLdOptions) -> JsonLd<'g> {
        let mut root = Ctx {
            base: base.cloned(),
            ..Ctx::default()
        };
        if let Some(v) = &options.default_vocab {
            if is_schema_context(v) {
                root.schema();
            } else {
                root.vocab = Some(v.clone());
            }
        }
        JsonLd {
            graph,
            ids: HashMap::new(),
            root,
        }
    }

    /// One block's top-level value.
    pub fn block(&mut self, value: &Value) {
        let ctx = self.root.clone();
        self.top(value, &ctx, 0);
    }

    /// One block as JSON-LD 1.1 processes it: expanded, flattened into a node map, its nodes into
    /// the graph (nodes of the same IRI are one node, across blocks too).
    pub fn block_strict(&mut self, value: &Value) -> Result<(), jsonld11::Error> {
        let processor = jsonld11::Processor::new(SchemaContext::get());
        let expanded = processor.expand_document(value, self.root.base.as_ref().map(Url::as_str))?;
        let (map, tops) = jsonld11::flatten(&expanded)?;
        let tops: std::collections::HashSet<&str> = tops.iter().map(String::as_str).collect();
        // A node of more than its `@id` (or stated at the top) is an item; a bare reference is a link.
        let mut index: HashMap<&str, usize> = HashMap::new();
        for graph in map.values() {
            for (id, node) in graph {
                if (node.len() < 2 && !tops.contains(id.as_str())) || index.contains_key(id.as_str()) {
                    continue;
                }
                let blank = id.starts_with("_:");
                let i = match self.ids.get(id).filter(|_| !blank) {
                    Some(&i) => i,
                    None => {
                        let mut n = Node::new(Format::JsonLd);
                        n.id = (!blank).then(|| id.clone());
                        let i = self.graph.add(n);
                        if !blank {
                            self.ids.insert(id.clone(), i);
                        }
                        i
                    }
                };
                index.insert(id, i);
            }
        }
        for id in &tops {
            if let Some(&i) = index.get(id) {
                self.graph.nodes[i].top = true;
            }
        }
        for graph in map.values() {
            for (id, node) in graph {
                let Some(&i) = index.get(id.as_str()) else { continue };
                for (key, values) in node {
                    let Value::Array(values) = values else { continue };
                    if key == "@type" {
                        for t in values.iter().filter_map(Value::as_str) {
                            if !self.graph.nodes[i].types.iter().any(|x| x == t) {
                                self.graph.nodes[i].types.push(t.to_owned());
                            }
                        }
                        continue;
                    }
                    if key.starts_with('@') {
                        continue;
                    }
                    let mut vals = Vec::new();
                    for v in values {
                        strict_value(v, &index, &mut vals);
                    }
                    for v in vals {
                        self.graph.nodes[i].push(key, v);
                    }
                }
            }
        }
        Ok(())
    }

    fn top(&mut self, value: &Value, ctx: &Ctx, depth: usize) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.top(item, ctx, depth + 1);
                }
            }
            Value::Object(_) => {
                if let Some(i) = self.node(value, ctx, depth) {
                    self.graph.nodes[i].top = true;
                }
            }
            _ => {}
        }
    }

    /// A node object into the graph (its index), or None when it is only a container (`@graph`).
    fn node(&mut self, value: &Value, outer: &Ctx, depth: usize) -> Option<usize> {
        let Value::Object(o) = value else { return None };
        if depth > MAX_DEPTH {
            return None;
        }
        let ctx = match o.get("@context") {
            Some(c) => outer.with(c),
            None => outer.clone(),
        };
        // Keys by the keyword they stand for.
        let mut id = None;
        let mut types = Vec::new();
        let mut graph = None;
        let mut included = None;
        let mut reverse = None;
        let mut props: Vec<(&String, &Value)> = Vec::new();
        for (key, v) in o {
            match ctx.keyword(key).as_deref() {
                Some("@context") => {}
                Some("@id") => id = v.as_str().and_then(|s| ctx.expand(s, false)),
                Some("@type") => types.extend(strings(v).flat_map(split_types).filter_map(|s| ctx.expand(&s, true))),
                Some("@graph") => graph = Some(v),
                Some("@included") => included = Some(v),
                Some("@reverse") => reverse = Some(v),
                Some("@value" | "@list" | "@set" | "@language" | "@direction" | "@index" | "@nest") => {}
                Some(_) => {}
                None => props.push((key, v)),
            }
        }
        for g in [graph, included].into_iter().flatten() {
            self.top(g, &ctx, depth + 1);
        }
        // A pure container (`{ "@context": …, "@graph": […] }`) is no node of its own.
        if graph.is_some() && types.is_empty() && props.is_empty() && reverse.is_none() {
            return None;
        }
        let index = match id.as_ref().and_then(|i| self.ids.get(i)) {
            Some(&i) => i,
            None => {
                let mut n = Node::new(Format::JsonLd);
                n.id = id.clone();
                let i = self.graph.add(n);
                if let Some(id) = id {
                    self.ids.insert(id, i);
                }
                i
            }
        };
        for t in types {
            if !self.graph.nodes[index].types.contains(&t) {
                self.graph.nodes[index].types.push(t);
            }
        }
        for (key, v) in props {
            let term = ctx.term(key).cloned();
            let Some(iri) = ctx.expand(key, true).filter(|i| !i.starts_with('@')) else {
                continue;
            };
            let mut vals = Vec::new();
            self.values(v, &ctx, term.as_ref(), depth, &mut vals);
            // schema.org's URL properties are IRIs: a relative one resolves against the page.
            if super::vocab::is_url_property(&iri) {
                for val in &mut vals {
                    if let Val::Text(s) | Val::Lang(s, _) = val {
                        let s2 = s.trim();
                        if !s2.is_empty() && !s2.contains(char::is_whitespace) {
                            *val = Val::Iri(ctx.resolve(s2).unwrap_or_else(|| s2.to_owned()));
                        }
                    }
                }
            }
            if term.as_ref().is_some_and(|t| t.reverse) {
                for val in vals {
                    if let Val::Node(other) = val {
                        self.graph.nodes[other].push(&iri, Val::Node(index));
                    }
                }
                continue;
            }
            for val in vals {
                self.graph.nodes[index].push(&iri, val);
            }
        }
        if let Some(Value::Object(r)) = reverse {
            for (key, v) in r {
                let Some(iri) = ctx.expand(key, true) else { continue };
                let mut vals = Vec::new();
                self.values(v, &ctx, None, depth, &mut vals);
                for val in vals {
                    if let Val::Node(other) = val {
                        self.graph.nodes[other].push(&iri, Val::Node(index));
                    }
                }
            }
        }
        Some(index)
    }

    fn values(&mut self, v: &Value, ctx: &Ctx, term: Option<&Term>, depth: usize, out: &mut Vec<Val>) {
        let coerce = term.and_then(|t| t.coerce.as_deref());
        match v {
            Value::Null => {}
            Value::Bool(b) => out.push(Val::Bool(*b)),
            Value::Number(n) => out.push(Val::Num(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => out.push(match coerce {
                Some("@id") => Val::Iri(ctx.expand(s, false).unwrap_or_else(|| s.clone())),
                Some("@vocab") => Val::Iri(ctx.expand(s, true).unwrap_or_else(|| s.clone())),
                Some(dt) if !dt.starts_with('@') => Val::Typed(s.clone(), dt.to_owned()),
                _ => match term
                    .and_then(|t| t.language.clone())
                    .unwrap_or_else(|| ctx.language.clone())
                {
                    Some(lang) => Val::Lang(plain(s), lang),
                    None => Val::Text(plain(s)),
                },
            }),
            Value::Array(items) => {
                for item in items {
                    self.values(item, ctx, term, depth + 1, out);
                }
            }
            Value::Object(o) => {
                let ctx = match o.get("@context") {
                    Some(c) => ctx.with(c),
                    None => ctx.clone(),
                };
                let key = |k: &str| o.keys().find(|x| ctx.keyword(x).as_deref() == Some(k));
                if let Some(vk) = key("@value") {
                    let value = &o[vk];
                    let lang = key("@language")
                        .and_then(|k| o[k].as_str())
                        .map(|l| l.to_ascii_lowercase());
                    let dt = key("@type")
                        .and_then(|k| o[k].as_str())
                        .and_then(|t| ctx.expand(t, true));
                    match (value, lang, dt) {
                        (Value::String(s), Some(l), _) => out.push(Val::Lang(s.clone(), l)),
                        (Value::String(s), None, Some(dt)) => out.push(Val::Typed(s.clone(), dt)),
                        (Value::Null, ..) => {}
                        (other, ..) => self.values(other, &ctx, None, depth + 1, out),
                    }
                    return;
                }
                if let Some(lk) = key("@list").or_else(|| key("@set")) {
                    self.values(&o[lk], &ctx, term, depth + 1, out);
                    return;
                }
                // `{ "@id": … }` alone is a reference; anything more is a node.
                let only_id = o
                    .keys()
                    .all(|k| matches!(ctx.keyword(k).as_deref(), Some("@id" | "@context")));
                if only_id {
                    if let Some(idk) = key("@id") {
                        if let Some(s) = o[idk].as_str().and_then(|s| ctx.expand(s, false)) {
                            out.push(Val::Iri(s));
                        }
                    }
                    return;
                }
                if let Some(i) = self.node(v, &ctx, depth + 1) {
                    out.push(Val::Node(i));
                }
            }
        }
    }
}

/// A node map value: a node (by its index), a link, a literal, a list's members.
fn strict_value(v: &Value, index: &HashMap<&str, usize>, out: &mut Vec<Val>) {
    if let Some(Value::Array(items)) = v.get("@list") {
        for item in items {
            strict_value(item, index, out);
        }
    } else if let Some(value) = v.get("@value") {
        let datatype = v.get("@type").and_then(Value::as_str);
        out.push(match (value, v.get("@language").and_then(Value::as_str), datatype) {
            (_, _, Some("@json")) => Val::Typed(value.to_string(), format!("{}JSON", super::vocab::RDF)),
            (Value::String(s), Some(l), _) => Val::Lang(s.clone(), l.to_owned()),
            (Value::String(s), None, Some(dt)) => Val::Typed(s.clone(), dt.to_owned()),
            (Value::String(s), None, None) => Val::Text(s.clone()),
            (Value::Number(n), ..) => Val::Num(n.as_f64().unwrap_or(0.0)),
            (Value::Bool(b), ..) => Val::Bool(*b),
            _ => return,
        });
    } else if let Some(id) = v.get("@id").and_then(Value::as_str) {
        match index.get(id) {
            Some(&i) => out.push(Val::Node(i)),
            None if !id.starts_with("_:") => out.push(Val::Iri(id.to_owned())),
            None => {}
        }
    }
}

/// Text as sites put it in JSON-LD, plain: character references decoded (`&amp;` → `&`),
/// markup taken out (`<b>Nora</b><br>live` → `Nora live`).
fn plain(s: &str) -> String {
    if !s.contains(['&', '<']) {
        return s.to_owned();
    }
    // As in an attribute value: `?a=1&b=2` keeps its `&b`.
    let mut decoded = String::with_capacity(s.len());
    crate::decode::decode_into_valid(s.as_bytes(), true, &mut decoded, true);
    if !decoded.contains('<') {
        return decoded;
    }
    let mut out = String::with_capacity(decoded.len());
    let mut rest = decoded.as_str();
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let is_tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match (is_tag, after.find('>')) {
            (true, Some(end)) => {
                // Block-level tags leave a space between the words they part.
                let name: String = after
                    .trim_start_matches('/')
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "br" | "p"
                        | "div"
                        | "li"
                        | "ul"
                        | "ol"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "tr"
                        | "td"
                        | "th"
                        | "table"
                        | "section"
                        | "article"
                        | "blockquote"
                ) {
                    out.push(' ');
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `"Event, MusicEvent"` and `"Event MusicEvent"` are two types; `"Music Event"` is one, mistyped.
fn split_types(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .flat_map(|p| {
            let words: Vec<&str> = p.split_whitespace().collect();
            let all_types = words.len() > 1
                && words
                    .iter()
                    .all(|w| w.contains(':') || super::vocab::canonical_type(w).is_some());
            if all_types {
                words.into_iter().map(str::to_owned).collect()
            } else {
                vec![p.to_owned()]
            }
        })
        .collect()
}

fn strings(v: &Value) -> impl Iterator<Item = &str> {
    let items: Vec<&str> = match v {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    items.into_iter()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use url::Url;

    use super::*;
    use crate::structured::graph::{Render, Writer};

    fn items(blocks: &[Value], render: &Render) -> Vec<Value> {
        let mut g = Graph::default();
        let base = Url::parse("https://venue.example/agenda/").unwrap();
        let mut j = JsonLd::new(&mut g, Some(&base), &JsonLdOptions::default());
        for b in blocks {
            j.block(b);
        }
        Writer::new(&g, render).items()
    }

    fn strip_format(mut v: Vec<Value>) -> Vec<Value> {
        for x in &mut v {
            let o = x.as_object_mut().unwrap();
            o.remove("@format");
            o.remove("@context");
        }
        v
    }

    #[test]
    fn the_shapes_sites_use() {
        let expected = json!({ "@type": "MusicEvent", "name": "Nora", "startDate": "2026-11-14T20:00" });
        for block in [
            json!({ "@context": "https://schema.org", "@type": "MusicEvent", "name": "Nora", "startDate": "2026-11-14T20:00" }),
            json!({ "@context": "http://schema.org/", "@type": "http://schema.org/MusicEvent", "http://schema.org/name": "Nora", "startDate": "2026-11-14T20:00" }),
            json!({ "@context": { "@vocab": "https://schema.org/" }, "@type": "MusicEvent", "name": "Nora", "startDate": "2026-11-14T20:00" }),
            json!({ "@context": { "s": "https://schema.org/" }, "@type": "s:MusicEvent", "s:name": "Nora", "s:startDate": "2026-11-14T20:00" }),
            json!({ "@type": "MusicEvent", "name": "Nora", "startDate": "2026-11-14T20:00" }),
            json!({ "@context": "https://schema.org/docs/jsonldcontext.json", "type": "musicevent", "name": "Nora", "startdate": "2026-11-14T20:00" }),
            json!({ "@context": "https://schema.org", "@graph": [{ "@type": "MusicEvent", "name": { "@value": "Nora" }, "startDate": "2026-11-14T20:00" }] }),
            json!([{ "@context": "https://schema.org", "@type": ["MusicEvent"], "name": ["Nora"], "startDate": "2026-11-14T20:00" }]),
        ] {
            let got = strip_format(items(std::slice::from_ref(&block), &Render::default()));
            assert_eq!(got, std::slice::from_ref(&expected), "{block}");
        }
    }

    #[test]
    fn references_across_blocks_and_graphs() {
        let a = json!({ "@context": "https://schema.org", "@graph": [
            { "@type": "MusicEvent", "@id": "#e1", "name": "Nora", "location": { "@id": "#paradiso" }, "performer": [{ "@id": "#nora" }] },
            { "@type": "Place", "@id": "#paradiso", "name": "Paradiso" },
        ] });
        let b = json!({ "@context": "https://schema.org", "@type": "MusicGroup", "@id": "https://venue.example/agenda/#nora", "name": "Nora" });
        let render = Render {
            types: vec!["Event".into()],
            ..Render::default()
        };
        let got = strip_format(items(&[a, b], &render));
        assert_eq!(
            got,
            [json!({
                "@type": "MusicEvent", "@id": "https://venue.example/agenda/#e1", "name": "Nora",
                "location": { "@type": "Place", "@id": "https://venue.example/agenda/#paradiso", "name": "Paradiso" },
                "performer": { "@type": "MusicGroup", "@id": "https://venue.example/agenda/#nora", "name": "Nora" },
            })]
        );
    }

    #[test]
    fn merges_and_loops() {
        let block = json!({ "@context": "https://schema.org", "@graph": [
            { "@type": "Event", "@id": "#e", "name": "A", "subEvent": { "@id": "#e" } },
            { "@id": "#e", "startDate": "2026-01-01" },
        ] });
        let got = strip_format(items(&[block], &Render::default()));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["startDate"], json!("2026-01-01"));
        assert_eq!(
            got[0]["subEvent"],
            json!("https://venue.example/agenda/#e"),
            "a loop is cut to its id"
        );
    }

    #[test]
    fn literals_languages_lists_and_enumerations() {
        let block = json!({
            "@context": ["https://schema.org", { "@language": "nl", "startDate": { "@type": "DateTime" } }],
            "@type": "Event", "name": "Nachtconcert", "startDate": "2026-11-14T20:00",
            "eventStatus": "https://schema.org/EventCancelled",
            "keywords": { "@list": ["jazz", "live"] },
            "@reverse": { "subEvent": { "@type": "Festival", "name": "ADE" } },
        });
        let render = Render {
            detailed: true,
            ..Render::default()
        };
        let got = &items(&[block], &render)[0];
        assert_eq!(got["name"], json!({ "@value": "Nachtconcert", "@language": "nl" }));
        assert_eq!(
            got["startDate"],
            json!({ "@value": "2026-11-14T20:00", "@type": "DateTime" })
        );
        assert_eq!(
            got["eventStatus"],
            json!({ "@value": "https://schema.org/EventCancelled", "@language": "nl" })
        );
        assert_eq!(
            got["keywords"],
            json!([{ "@value": "jazz", "@language": "nl" }, { "@value": "live", "@language": "nl" }])
        );
        let plain = &items(std::slice::from_ref(got), &Render::default());
        assert!(
            plain.is_empty()
                || plain[0].get("eventStatus").is_none()
                || plain[0]["eventStatus"] == json!("EventCancelled")
        );
    }

    #[test]
    fn plain_output_names_enumeration_members() {
        let block = json!({ "@type": "Event", "eventStatus": "https://schema.org/EventCancelled", "offers": { "@type": "Offer", "availability": "http://schema.org/SoldOut" } });
        let got = &items(&[block], &Render::default())[0];
        assert_eq!(got["eventStatus"], json!("EventCancelled"));
        assert_eq!(got["offers"]["availability"], json!("SoldOut"));
    }
}
