//! Microdata (WHATWG HTML § 5.2), read as the document streams by.
//!
//! An element with `itemscope` opens an item; `itemtype` gives its types,
//! `itemid` its id, `itemref` the ids of elements elsewhere whose
//! properties are its too. An element with `itemprop` is a property of the
//! nearest item around it (or of every item that refers to an element on
//! its way there); its value is an item when it has `itemscope`, else its
//! `content` (`meta`), `src` (`audio`, `embed`, `iframe`, `img`, `source`,
//! `track`, `video`), `href` (`a`, `area`, `link`), `data` (`object`),
//! `value` (`data`, `meter`), `datetime` (`time`), or else its text.
//!
//! Leniency where Google's reader is lenient (both on by default): `content`
//! on any element is its value (`<span itemprop="price" content="25">€25</span>`),
//! and `itemtype` without `itemscope` still opens an item.
//!
//! Properties are matched to items at the end (`itemref` can point ahead):
//! each property keeps its nearest item and the ids on its way to it.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};
use url::Url;

use super::graph::{Format, Graph, Node, Val};
use crate::tree::Tree;

struct Index<'m> {
    by_owner: Vec<Vec<usize>>,
    by_id: HashMap<&'m str, Vec<usize>>,
}

#[derive(Debug)]
struct Scope {
    types: Vec<String>,
    id: Option<String>,
    refs: Vec<String>,
    /// It is itself a property's value.
    is_prop: bool,
}

#[derive(Debug)]
enum Value_ {
    Text(String),
    Url(String),
    Item(usize),
}

#[derive(Debug)]
struct Prop {
    names: Vec<String>,
    owner: Option<usize>,
    /// The ids from its owner (not included) down to it (included): an `itemref` to any brings it in.
    path: Vec<String>,
    value: Value_,
    order: usize,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    /// The item properties inside belong to.
    owner: Option<usize>,
    /// Where, in `ids`, the ids since that item start.
    mark: usize,
    /// `ids`' length before this element.
    ids_len: usize,
    /// The property whose text this element collects.
    capture: Option<usize>,
}

/// How microdata is read.
#[derive(Clone, Copy, Debug)]
pub struct MicrodataOptions {
    /// `content` on any element is its value, `itemtype` alone opens an item.
    pub lenient: bool,
    /// The most text one value collects.
    pub max_text: usize,
}

impl Default for MicrodataOptions {
    fn default() -> Self {
        MicrodataOptions {
            lenient: true,
            max_text: 64 * 1024,
        }
    }
}

pub struct Microdata {
    opts: MicrodataOptions,
    scopes: Vec<Scope>,
    props: Vec<Prop>,
    frames: Vec<Frame>,
    ids: Vec<String>,
    /// Every id met: `itemref` names the first element of an id (document order), not a later one.
    seen_ids: HashSet<String>,
    capturing: Vec<usize>,
}

fn tokens(s: Option<&str>) -> Vec<String> {
    s.map(|s| s.split_ascii_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

impl Microdata {
    pub fn new(opts: MicrodataOptions) -> Microdata {
        Microdata {
            opts,
            scopes: Vec::new(),
            props: Vec::new(),
            frames: Vec::new(),
            ids: Vec::new(),
            seen_ids: HashSet::new(),
            capturing: Vec::new(),
        }
    }

    /// Whether text matters now (a value is being read from text).
    pub fn wants_text(&self) -> bool {
        !self.capturing.is_empty()
    }

    pub fn open(&mut self, tree: &Tree, base: Option<&Url>) {
        let el = &tree.frames[tree.frames.len() - 1].el;
        let parent = self.frames.last().copied();
        let owner = parent.and_then(|p| p.owner);
        let mark = parent.map_or(0, |p| p.mark);
        let ids_len = self.ids.len();
        // Most elements have no microdata and no id: one look at their attributes.
        if !el.attrs().any(|(n, _)| n == "id" || n.starts_with("item")) {
            self.frames.push(Frame {
                owner,
                mark,
                ids_len,
                capture: None,
            });
            return;
        }
        let own_id = el
            .attr("id")
            .filter(|s| !s.is_empty())
            .filter(|s| !self.seen_ids.contains(*s))
            .map(str::to_owned);
        if let Some(id) = &own_id {
            self.seen_ids.insert(id.clone());
        }
        let itemprop = tokens(el.attr("itemprop"));
        let is_scope = el.attr("itemscope").is_some() || (self.opts.lenient && el.attr("itemtype").is_some());
        let mut frame = Frame {
            owner,
            mark,
            ids_len,
            capture: None,
        };
        let scope = is_scope.then(|| {
            let id = el
                .attr("itemid")
                .map(|s| url_value(Some(s), base, self.opts.lenient))
                .filter(|s| !s.is_empty());
            self.scopes.push(Scope {
                types: tokens(el.attr("itemtype")),
                id,
                refs: tokens(el.attr("itemref")),
                is_prop: !itemprop.is_empty(),
            });
            self.scopes.len() - 1
        });
        if !itemprop.is_empty() {
            let mut path = self.ids[mark..].to_vec();
            if let Some(id) = &own_id {
                path.push(id.clone());
            }
            let name = el.name();
            let attr = |a: &str| el.attr(a).unwrap_or("").to_owned();
            let value = match scope {
                Some(s) => Value_::Item(s),
                None if name == "meta" || (self.opts.lenient && el.attr("content").is_some()) => {
                    let content = attr("content");
                    // `<meta itemprop="image" content="/a.jpg">`: a URL, as the property means.
                    let url_prop = itemprop.iter().any(|p| {
                        super::vocab::is_url_property(&format!("https://schema.org/{p}"))
                            || super::vocab::is_url_property(p)
                    });
                    if self.opts.lenient
                        && url_prop
                        && !content.trim().is_empty()
                        && !content.trim().contains(char::is_whitespace)
                    {
                        Value_::Url(resolve(&content, base))
                    } else {
                        Value_::Text(content)
                    }
                }
                None if matches!(
                    name,
                    "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video"
                ) =>
                {
                    Value_::Url(url_value(el.attr("src"), base, self.opts.lenient))
                }
                None if matches!(name, "a" | "area" | "link") => {
                    Value_::Url(url_value(el.attr("href"), base, self.opts.lenient))
                }
                None if name == "object" => Value_::Url(url_value(el.attr("data"), base, self.opts.lenient)),
                None if matches!(name, "data" | "meter") => Value_::Text(attr("value")),
                None if name == "time" && el.attr("datetime").is_some() => Value_::Text(attr("datetime")),
                None => Value_::Text(String::new()),
            };
            let text_value = matches!(value, Value_::Text(ref t) if t.is_empty())
                && !(name == "meta" || el.attr("content").is_some_and(|_| self.opts.lenient))
                && !(matches!(name, "data" | "meter"))
                && !(name == "time" && el.attr("datetime").is_some());
            let order = self.props.len();
            self.props.push(Prop {
                names: itemprop,
                owner,
                path,
                value,
                order,
            });
            if text_value {
                frame.capture = Some(order);
                self.capturing.push(order);
            }
        }
        if let Some(id) = own_id {
            self.ids.push(id);
        }
        if let Some(s) = scope {
            frame.owner = Some(s);
            frame.mark = self.ids.len();
        }
        self.frames.push(frame);
    }

    pub fn close(&mut self) {
        if let Some(f) = self.frames.pop() {
            self.ids.truncate(f.ids_len);
            if f.capture.is_some() {
                self.capturing.pop();
            }
        }
    }

    pub fn text(&mut self, text: &str) {
        let max = self.opts.max_text;
        for &p in &self.capturing {
            if let Value_::Text(t) = &mut self.props[p].value {
                if t.len() < max {
                    t.push_str(crate::text::cut(text, max - t.len()));
                }
            }
        }
    }

    /// Which properties each item owns, and which properties each id's element holds.
    fn index(&self) -> Index<'_> {
        let mut by_owner = vec![Vec::new(); self.scopes.len()];
        let mut by_id: HashMap<&str, Vec<usize>> = HashMap::new();
        for p in &self.props {
            if let Some(o) = p.owner {
                by_owner[o].push(p.order);
            }
            for id in &p.path {
                by_id.entry(id.as_str()).or_default().push(p.order);
            }
        }
        Index { by_owner, by_id }
    }

    /// The properties of an item, in document order: its own and those its `itemref`s bring.
    fn props_of(&self, scope: usize, index: &Index<'_>) -> Vec<usize> {
        let s = &self.scopes[scope];
        let mut out = index.by_owner[scope].clone();
        for r in &s.refs {
            if let Some(more) = index.by_id.get(r.as_str()) {
                out.extend(more);
            }
        }
        if !s.refs.is_empty() {
            out.sort_unstable();
            out.dedup();
        }
        out
    }

    /// The top-level items: those that are no property's value (and, leniently, those whose
    /// property belongs to no item).
    fn tops(&self) -> Vec<bool> {
        let referenced: HashSet<&str> = self
            .scopes
            .iter()
            .flat_map(|s| s.refs.iter().map(String::as_str))
            .collect();
        let mut top: Vec<bool> = self.scopes.iter().map(|s| !s.is_prop).collect();
        if self.opts.lenient {
            for p in &self.props {
                if let Value_::Item(x) = p.value {
                    if p.owner.is_none() && !p.path.iter().any(|id| referenced.contains(id.as_str())) {
                        top[x] = true;
                    }
                }
            }
        }
        top
    }

    /// The WHATWG JSON form: `{ "items": [{ "type", "id", "properties" }] }`.
    pub fn json(&self) -> Value {
        let index = self.index();
        let items: Vec<Value> = self
            .tops()
            .into_iter()
            .enumerate()
            .filter(|(_, top)| *top)
            .map(|(s, _)| self.item_json(s, &mut Vec::new(), &index))
            .collect();
        json!({ "items": items })
    }

    fn item_json(&self, s: usize, memory: &mut Vec<usize>, index: &Index<'_>) -> Value {
        let scope = &self.scopes[s];
        let mut o = Map::new();
        if !scope.types.is_empty() {
            o.insert("type".into(), json!(scope.types));
        }
        // An item has a global identifier only with a type (WHATWG § 5.2.2).
        if let (Some(id), false) = (&scope.id, scope.types.is_empty()) {
            o.insert("id".into(), json!(id));
        }
        memory.push(s);
        let mut props = Map::new();
        for p in self.props_of(s, index) {
            let p = &self.props[p];
            let v = match &p.value {
                Value_::Text(t) | Value_::Url(t) => json!(t),
                Value_::Item(x) if memory.contains(x) => json!("ERROR"),
                Value_::Item(x) => self.item_json(*x, memory, index),
            };
            for name in &p.names {
                match props.get_mut(name) {
                    Some(Value::Array(a)) => a.push(v.clone()),
                    _ => {
                        props.insert(name.clone(), json!([v.clone()]));
                    }
                }
            }
        }
        memory.pop();
        o.insert("properties".into(), Value::Object(props));
        Value::Object(o)
    }

    /// The items into `graph`; property names are IRIs in the item's vocabulary.
    pub fn into_graph(&self, graph: &mut Graph) {
        let mut nodes: Vec<Option<usize>> = vec![None; self.scopes.len()];
        for (s, scope) in self.scopes.iter().enumerate() {
            let mut n = Node::new(Format::Microdata);
            n.types = scope.types.clone();
            n.id = scope.id.clone();
            nodes[s] = Some(graph.add(n));
        }
        let tops = self.tops();
        let props = self.index();
        for (s, scope) in self.scopes.iter().enumerate() {
            let Some(index) = nodes[s] else { continue };
            graph.nodes[index].top = tops[s];
            // A name is an IRI as it is, or relative to the vocabulary of the item's type.
            let vocab =
                scope
                    .types
                    .first()
                    .map(|t| t.trim_end_matches(['/', '#']))
                    .map(|t| match t.rfind(['/', '#']) {
                        Some(i) => t[..=i].to_owned(),
                        None => String::new(),
                    });
            for p in self.props_of(s, &props) {
                let p = &self.props[p];
                let val = match &p.value {
                    Value_::Text(t) => Val::Text(t.clone()),
                    Value_::Url(u) => Val::Iri(u.clone()),
                    Value_::Item(x) => match nodes[*x] {
                        Some(i) => Val::Node(i),
                        None => continue,
                    },
                };
                for name in &p.names {
                    let iri = if name.contains(':') {
                        name.clone()
                    } else {
                        match &vocab {
                            Some(v) if !v.is_empty() => format!("{v}{name}"),
                            _ => name.clone(),
                        }
                    };
                    graph.nodes[index].push(&iri, val.clone());
                }
            }
        }
    }
}

fn resolve(href: &str, base: Option<&Url>) -> String {
    let href = href.trim();
    // No `href`, no `src`: no value (not the page's own address).
    if href.is_empty() {
        return String::new();
    }
    match base {
        Some(b) => b.join(href).map(|u| u.to_string()).unwrap_or_else(|_| href.to_owned()),
        None => href.to_owned(),
    }
}

/// A URL attribute's value as WHATWG microdata has it: absent, "" ; present, parsed against the
/// document (an empty one is the document's own URL), "" when it does not parse. Leniently, an
/// empty attribute is no value, and one that does not parse is kept as written.
fn url_value(attr: Option<&str>, base: Option<&Url>, lenient: bool) -> String {
    let Some(v) = attr else { return String::new() };
    if lenient {
        return resolve(v, base);
    }
    match base {
        Some(b) => b.join(v.trim()).map(|u| u.to_string()).unwrap_or_default(),
        None => Url::parse(v.trim()).map(|u| u.to_string()).unwrap_or_default(),
    }
}
