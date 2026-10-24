//! The items every format is read into, and how they are written out.
//!
//! A node has types, an optional id, and properties in order, each with
//! one or more values; a value can be another node (by index, so cycles
//! and references cost nothing). Types and property names are full IRIs
//! (microformats keep their own names). Written out, an item looks like
//! compact JSON-LD: `{ "@type": "MusicEvent", "name": "…", "location": {…} }`,
//! schema.org names short, references embedded once, loops cut.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use super::vocab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    JsonLd,
    Microdata,
    Rdfa,
    Microformats,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::JsonLd => "json-ld",
            Format::Microdata => "microdata",
            Format::Rdfa => "rdfa",
            Format::Microformats => "microformats",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Text(String),
    /// A string in a language.
    Lang(String, String),
    /// A string of a datatype (an IRI).
    Typed(String, String),
    Num(f64),
    Bool(bool),
    /// A link: a URL, or the id of a node (embedded when one has it).
    Iri(String),
    Node(usize),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub format: Format,
    pub types: Vec<String>,
    pub id: Option<String>,
    pub props: Vec<(String, Vec<Val>)>,
    /// Stated at the top of its source (not only as some other item's value).
    pub top: bool,
}

impl Node {
    pub fn new(format: Format) -> Node {
        Node {
            format,
            types: Vec::new(),
            id: None,
            props: Vec::new(),
            top: false,
        }
    }

    pub fn push(&mut self, key: &str, value: Val) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some((_, vals)) => vals.push(value),
            None => self.props.push((key.to_owned(), vec![value])),
        }
    }
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
}

impl Graph {
    pub fn add(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }
}

/// How items are written out.
#[derive(Clone, Debug)]
pub struct Render {
    /// Every property value as an array, even a single one.
    pub arrays: bool,
    /// Literals with their language or datatype as `{ "@value", "@language" | "@type" }`, links as `{ "@id" }`.
    pub detailed: bool,
    /// Embed the node a link names, when there is one.
    pub embed: bool,
    /// Nested items deeper than this are written as their id (or left out).
    pub max_depth: usize,
    /// Keep only items of these types (schema.org subtypes count: `Event` keeps `Festival`),
    /// wherever they are; empty: every item at the top of its source.
    pub types: Vec<String>,
    /// Collapse whitespace in strings (`"\n  Nora  "` → `"Nora"`).
    pub collapse: bool,
    /// Give each item (but microformats') its JSON-LD `@context`: schema.org, and the prefixes it
    /// uses (`og:`, `dc:`), so an item is the same JSON-LD whichever format the page used.
    pub context: bool,
    /// Every item, also those inside others (an event's place, a product's offer), each on its
    /// own too: nothing only reachable by walking into another item.
    pub flatten: bool,
}

impl Default for Render {
    fn default() -> Self {
        Render {
            arrays: false,
            detailed: false,
            embed: true,
            max_depth: 16,
            types: Vec::new(),
            collapse: true,
            context: true,
            flatten: false,
        }
    }
}

/// Whether a node's types include one of `wanted` (by schema.org hierarchy, short or full name).
pub fn matches_type(node: &Node, wanted: &[String]) -> bool {
    if node.format == Format::Microformats {
        return node.types.iter().any(|t| wanted.contains(t));
    }
    node.types.iter().any(|t| {
        let short = vocab::short_name(t, true);
        // Microformats' names (`h-event`) are no schema.org type: no hierarchy to ask.
        wanted
            .iter()
            .any(|w| w == t || *w == short || (!w.starts_with("h-") && vocab::is_a(&short, w)))
    })
}

pub struct Writer<'g> {
    graph: &'g Graph,
    opts: &'g Render,
    ids: HashMap<&'g str, usize>,
    /// Within the item being written: the nodes already embedded (a node is written out once,
    /// then by its `@id`, as JSON-LD's `@embed: @once`), and how many nodes it may still hold.
    embedded: RefCell<HashSet<usize>>,
    budget: Cell<usize>,
}

/// The most nodes one item holds written out (a page of references could otherwise multiply).
const MAX_NODES_PER_ITEM: usize = 10_000;

impl<'g> Writer<'g> {
    pub fn new(graph: &'g Graph, opts: &'g Render) -> Writer<'g> {
        // A link resolves to the node of that id with the most properties.
        let mut ids: HashMap<&str, usize> = HashMap::new();
        for (i, n) in graph.nodes.iter().enumerate() {
            if let Some(id) = &n.id {
                let better = ids
                    .get(id.as_str())
                    .is_none_or(|&j| graph.nodes[j].props.len() < n.props.len());
                if better {
                    ids.insert(id.as_str(), i);
                }
            }
        }
        Writer {
            graph,
            opts,
            ids,
            embedded: RefCell::new(HashSet::new()),
            budget: Cell::new(MAX_NODES_PER_ITEM),
        }
    }

    /// The items to return: the top of every source, or every node of the wanted types.
    pub fn items(&self) -> Vec<Value> {
        let picked: Vec<usize> = if self.opts.flatten && self.opts.types.is_empty() {
            (0..self.graph.nodes.len()).collect()
        } else if self.opts.types.is_empty() {
            (0..self.graph.nodes.len())
                .filter(|&i| self.graph.nodes[i].top)
                .collect()
        } else {
            (0..self.graph.nodes.len())
                .filter(|&i| matches_type(&self.graph.nodes[i], &self.opts.types))
                .collect()
        };
        picked
            .into_iter()
            .filter_map(|i| {
                let item = self.node(i, &mut Vec::new(), true);
                // An item that says nothing (no type, no property: a broken block, a bare `@id`) is none.
                if !item
                    .as_object()?
                    .keys()
                    .any(|k| !matches!(k.as_str(), "@id" | "@format"))
                {
                    return None;
                }
                Some(
                    if self.opts.context && self.graph.nodes[i].format != Format::Microformats {
                        with_context(item)
                    } else {
                        item
                    },
                )
            })
            .collect()
    }

    pub fn node(&self, i: usize, path: &mut Vec<usize>, top: bool) -> Value {
        if path.is_empty() {
            self.embedded.borrow_mut().clear();
            self.budget.set(MAX_NODES_PER_ITEM);
        }
        self.embedded.borrow_mut().insert(i);
        self.budget.set(self.budget.get().saturating_sub(1));
        let n = &self.graph.nodes[i];
        let raw = n.format == Format::Microformats;
        let mut o = Map::new();
        if !n.types.is_empty() {
            let types: Vec<Value> = n
                .types
                .iter()
                .map(|t| Value::String(if raw { t.clone() } else { vocab::short_name(t, true) }))
                .collect();
            o.insert("@type".into(), self.many(types));
        }
        if let Some(id) = &n.id {
            if !id.starts_with("_:") {
                o.insert("@id".into(), Value::String(id.clone()));
            }
        }
        if top {
            o.insert("@format".into(), Value::String(n.format.as_str().into()));
        }
        path.push(i);
        for (key, vals) in &n.props {
            let name = if raw {
                key.clone()
            } else {
                vocab::short_name(key, false)
            };
            let written: Vec<Value> = vals.iter().filter_map(|v| self.value(v, path, &name)).collect();
            if written.is_empty() {
                continue;
            }
            match o.get_mut(&name) {
                Some(Value::Array(existing)) => existing.extend(written),
                Some(existing) => {
                    let first = existing.take();
                    let mut all = vec![first];
                    all.extend(written);
                    *existing = Value::Array(all);
                }
                None => {
                    o.insert(name, self.many(written));
                }
            }
        }
        path.pop();
        Value::Object(o)
    }

    /// Whether node `i` is written out here: not yet in this item, and the item not full.
    fn room_for(&self, i: usize) -> bool {
        self.budget.get() > 0 && (!self.embedded.borrow().contains(&i) || self.graph.nodes[i].id.is_none())
    }

    fn many(&self, mut values: Vec<Value>) -> Value {
        if values.len() == 1 && !self.opts.arrays {
            values.pop().unwrap_or(Value::Null)
        } else {
            Value::Array(values)
        }
    }

    fn text(&self, s: &str) -> String {
        if self.opts.collapse {
            crate::text::collapse(s)
        } else {
            s.to_owned()
        }
    }

    /// A string that is a schema.org IRI of an enumeration member, by the member's name; for a
    /// property that takes one (`eventStatus`), the member's name however written (`eventcancelled`).
    fn member(&self, s: &str, property: &str) -> Option<String> {
        if vocab::schema_local(s).is_none() && !vocab::takes_enumeration(property) {
            return None;
        }
        vocab::enum_value(s).map(str::to_owned)
    }

    fn value(&self, v: &Val, path: &mut Vec<usize>, property: &str) -> Option<Value> {
        // An empty string (`"url": ""`, `content=""`) says nothing.
        if let Val::Text(s) | Val::Lang(s, _) | Val::Typed(s, _) | Val::Iri(s) = v {
            if s.trim().is_empty() {
                return None;
            }
        }
        if !self.opts.detailed {
            if let Val::Text(s) | Val::Lang(s, _) | Val::Typed(s, _) = v {
                if let Some(m) = self.member(s, property) {
                    return Some(Value::String(m));
                }
            }
        }
        Some(match v {
            Val::Text(s) => Value::String(self.text(s)),
            Val::Lang(s, lang) if self.opts.detailed => json!({ "@value": self.text(s), "@language": lang }),
            Val::Lang(s, _) => Value::String(self.text(s)),
            Val::Typed(s, dt) if self.opts.detailed => {
                json!({ "@value": self.text(s), "@type": vocab::short_name(dt, true) })
            }
            Val::Typed(s, _) => Value::String(self.text(s)),
            // `25`, not `25.0`: a whole number as it was written.
            Val::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::from(*n as i64),
            Val::Num(n) => serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number),
            Val::Bool(b) => Value::Bool(*b),
            Val::Iri(iri) => {
                if self.opts.embed {
                    if let Some(&i) = self.ids.get(iri.as_str()) {
                        if !path.contains(&i)
                            && self.room_for(i)
                            && path.len() < self.opts.max_depth
                            && !self.graph.nodes[i].props.is_empty()
                        {
                            return Some(self.node(i, path, false));
                        }
                    }
                }
                // An enumeration member by name (`EventCancelled`), a blank node not at all.
                if let Some(member) = vocab::enum_value(iri).filter(|_| vocab::schema_local(iri).is_some()) {
                    return Some(Value::String(member.into()));
                }
                if iri.starts_with("_:") {
                    return None;
                }
                if self.opts.detailed {
                    json!({ "@id": iri })
                } else {
                    Value::String(iri.clone())
                }
            }
            Val::Node(i) => {
                if path.contains(i) || path.len() >= self.opts.max_depth || !self.room_for(*i) {
                    let id = self.graph.nodes[*i].id.clone()?;
                    return (!id.starts_with("_:")).then(|| Value::String(id));
                }
                self.node(*i, path, false)
            }
        })
    }
}

/// `item` with its `@context` first: `"https://schema.org"`, or that and the prefixes its names use.
fn with_context(item: Value) -> Value {
    let Value::Object(fields) = item else { return item };
    let mut prefixes = Map::new();
    let mut note = |name: &str| {
        if let Some((prefix, rest)) = name.split_once(':') {
            if !rest.starts_with("//") && !prefixes.contains_key(prefix) {
                if let Some(ns) = vocab::prefix_namespace(prefix) {
                    prefixes.insert(prefix.to_owned(), Value::String(ns.to_owned()));
                }
            }
        }
    };
    fn walk(v: &Value, note: &mut impl FnMut(&str)) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if k == "@type" {
                        match v {
                            Value::String(t) => note(t),
                            Value::Array(ts) => ts.iter().filter_map(Value::as_str).for_each(&mut *note),
                            _ => {}
                        }
                    } else {
                        if !k.starts_with('@') {
                            note(k);
                        }
                        walk(v, note);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, note)),
            _ => {}
        }
    }
    let item = Value::Object(fields);
    walk(&item, &mut note);
    let context = if prefixes.is_empty() {
        Value::String("https://schema.org".into())
    } else {
        Value::Array(vec![
            Value::String("https://schema.org".into()),
            Value::Object(prefixes),
        ])
    };
    let Value::Object(fields) = item else { unreachable!() };
    let mut o = Map::with_capacity(fields.len() + 1);
    o.insert("@context".into(), context);
    o.extend(fields);
    Value::Object(o)
}
