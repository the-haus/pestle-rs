//! What to extract, as plain data (JSON), compiled.
//!
//! A spec is an object of fields (or `{ "fields": { … } }`). A field is a
//! selector string, or an object:
//!
//! | key | what |
//! | --- | --- |
//! | `select` | CSS selector, relative to the field's scope; absent: the scope element itself |
//! | `all` | every match (an array), not only the first with a value |
//! | `limit` | with `all`: at most this many |
//! | `attr` | read this attribute instead of the text |
//! | `text` | `"visible"` (default: as a reader sees it), `"raw"` (all text, as is), `"own"` (only the element's own text) |
//! | `html` | read the element's outer HTML |
//! | `exists` | `true` when something matches, else `false` |
//! | `url` | resolve against the page (`true`), or only as an http(s) page, without `#…` (`"http"`) |
//! | `pattern` | a regex (case-insensitive unless `(?-i)`): its first group, else its match; no match is no value |
//! | `max` | cut the value to this many characters |
//! | `collapse` | collapse whitespace (default: for visible and own text and attributes) |
//! | `number` | read as a number (`€ 25,-` is 25) |
//! | `json` | parse as JSON (JSON-LD's CDATA and stray control characters tolerated) |
//! | `parse` | parse with a pest grammar: `"name"`, `"name.rule"`, or `{ grammar, rule, find, all, limit, lists, numbers, byRule, trim, tree }` |
//! | `fields` | an object per match, its fields relative to the match |
//! | `default` | the value when nothing matched |
//! | `emit` | (top-level, with `all`) hand items over as they complete instead of keeping them |
//!
//! Shorthands: `"h1"` is `{ "select": "h1" }`; `"meta[property=og:image]@content"`
//! reads an attribute; `["li.act"]` is every match.

use std::collections::HashMap;
#[cfg(feature = "grammar")]
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

#[cfg(feature = "grammar")]
use pestle_grammar::{Grammar, ShapeOptions};
use regex::Regex;
use serde_json::{Map, Value};

use crate::selector::Selector;

/// A spec's problems, each with the field it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecError {
    pub errors: Vec<String>,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.errors.join("\n"))
    }
}

impl std::error::Error for SpecError {}

#[derive(Debug)]
pub(crate) enum Read {
    Visible,
    Raw,
    Own,
    Attr(String),
    Html,
    Exists,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UrlMode {
    Resolve,
    Http,
}

#[cfg(feature = "grammar")]
pub(crate) struct ParseDef {
    pub(crate) grammar: Arc<Grammar>,
    pub(crate) name: String,
    pub(crate) rule: Option<String>,
    pub(crate) find: bool,
    pub(crate) all: bool,
    pub(crate) limit: usize,
    pub(crate) shape: ShapeOptions,
    pub(crate) tree: bool,
}

pub(crate) enum Then {
    Nothing,
    Number,
    Json,
    #[cfg(feature = "grammar")]
    Parse(Box<ParseDef>),
}

pub(crate) struct FieldDef {
    pub(crate) name: String,
    /// Where it is, for errors: `events[].title`.
    pub(crate) path: String,
    pub(crate) selector: Option<Selector>,
    pub(crate) all: bool,
    pub(crate) limit: usize,
    pub(crate) read: Read,
    pub(crate) url: Option<UrlMode>,
    pub(crate) collapse: bool,
    pub(crate) max: Option<usize>,
    pub(crate) pattern: Option<Regex>,
    pub(crate) then: Then,
    /// The object each match makes ([`Spec::objects`]).
    pub(crate) object: Option<usize>,
    pub(crate) default: Option<Value>,
    pub(crate) emit: bool,
}

pub(crate) struct ObjectDef {
    pub(crate) fields: Vec<FieldDef>,
    /// Fields whose selector can only select elements of a tag, by tag; the rest (with a selector) in `any`.
    pub(crate) by_tag: Vec<(String, u32, Vec<usize>)>,
    pub(crate) any: Vec<usize>,
    /// The names of the fields an object of it holds (not `emit` ones), shared by all its objects.
    pub(crate) keys: std::sync::Arc<[String]>,
}

impl ObjectDef {
    fn new(fields: Vec<FieldDef>) -> ObjectDef {
        let mut by_tag: Vec<(String, u32, Vec<usize>)> = Vec::new();
        let mut any = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            let Some(sel) = &f.selector else { continue };
            match sel.subject_tags() {
                Some(tags) => {
                    for t in tags {
                        match by_tag.iter_mut().find(|(x, _, _)| *x == t) {
                            Some((_, _, v)) => v.push(i),
                            None => {
                                let h = crate::tree::tag_hash(&t);
                                by_tag.push((t, h, vec![i]));
                            }
                        }
                    }
                }
                None => any.push(i),
            }
        }
        let keys = fields.iter().filter(|f| !f.emit).map(|f| f.name.clone()).collect();
        ObjectDef {
            fields,
            by_tag,
            any,
            keys,
        }
    }

    /// The fields that can select an element of this tag.
    pub(crate) fn candidates(&self, el: &crate::tree::Element) -> (&[usize], &[usize]) {
        let tagged = self
            .by_tag
            .iter()
            .find(|(t, h, _)| el.is(t, *h))
            .map_or(&[][..], |(_, _, v)| v.as_slice());
        (&self.any, tagged)
    }
}

/// A compiled spec.
pub struct Spec {
    /// Every object, the top level first.
    pub(crate) objects: Vec<ObjectDef>,
    pub(crate) siblings: bool,
    /// Some selector counts siblings by name (`*-of-type`).
    pub(crate) types: bool,
}

impl fmt::Debug for Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Spec").field("objects", &self.objects.len()).finish()
    }
}

/// Grammars a spec's `parse` can name.
#[cfg(feature = "grammar")]
pub type Grammars = HashMap<String, Arc<Grammar>>;

/// Without the `grammar` feature there are no grammars: `parse` is an error, `as` reads values.
#[cfg(not(feature = "grammar"))]
pub type Grammars = HashMap<String, Arc<NoGrammar>>;

#[cfg(not(feature = "grammar"))]
#[derive(Debug)]
pub enum NoGrammar {}

/// Options that change how a spec compiles.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompileOptions {
    /// A field that does not compile is left out (and reported by [`Spec::new`]'s second value) instead of failing the spec.
    pub lenient: bool,
}

impl Spec {
    /// Every attribute name the spec can read or test.
    pub(crate) fn attributes(&self) -> Vec<String> {
        // `<base href>` sets the base of URLs.
        let mut out = vec!["href".to_owned()];
        for o in &self.objects {
            for f in &o.fields {
                if let Some(sel) = &f.selector {
                    sel.attributes(&mut out);
                }
                if let Read::Attr(a) = &f.read {
                    out.push(a.clone());
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Compiles a spec; with [`CompileOptions::lenient`], also the problems of the fields left out.
    pub fn new(spec: &Value, grammars: &Grammars, options: CompileOptions) -> Result<(Spec, Vec<String>), SpecError> {
        let fields = match spec {
            Value::Object(o) if o.len() == 1 && o.get("fields").is_some_and(Value::is_object) => &o["fields"],
            other => other,
        };
        let Value::Object(fields) = fields else {
            return Err(SpecError {
                errors: vec!["a spec is an object of fields".into()],
            });
        };
        let mut cx = Compiler {
            grammars,
            objects: Vec::new(),
            errors: Vec::new(),
            siblings: false,
            types: false,
        };
        cx.objects.push(ObjectDef::new(Vec::new()));
        let root = cx.object(fields, "", true);
        cx.objects[0] = root;
        if !options.lenient && !cx.errors.is_empty() {
            return Err(SpecError { errors: cx.errors });
        }
        Ok((
            Spec {
                objects: cx.objects,
                siblings: cx.siblings,
                types: cx.types,
            },
            cx.errors,
        ))
    }
}

struct Compiler<'g> {
    grammars: &'g Grammars,
    objects: Vec<ObjectDef>,
    errors: Vec<String>,
    siblings: bool,
    types: bool,
}

const KEYS: &[&str] = &[
    "select", "all", "limit", "attr", "text", "html", "exists", "url", "pattern", "max", "collapse", "number", "json",
    "parse", "fields", "default", "emit",
];

/// `selector@attr`: the attribute after the last `@` outside brackets and quotes.
fn split_attr(s: &str) -> (&str, Option<&str>) {
    let (mut depth, mut quote) = (0i32, None);
    let mut at = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, '@') if depth == 0 => at = Some(i),
            _ => {}
        }
    }
    match at {
        Some(i)
            if s[i + 1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
                && i + 1 < s.len() =>
        {
            (s[..i].trim(), Some(&s[i + 1..]))
        }
        _ => (s, None),
    }
}

impl Compiler<'_> {
    fn object(&mut self, fields: &Map<String, Value>, prefix: &str, top: bool) -> ObjectDef {
        let mut out = Vec::with_capacity(fields.len());
        for (name, value) in fields {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            // A field with a problem of its own is left out; a problem in its nested fields leaves out only those.
            if let Some(f) = self.field(name, &path, value, top) {
                out.push(f);
            }
        }
        ObjectDef::new(out)
    }

    fn error(&mut self, path: &str, message: impl fmt::Display) {
        self.errors.push(format!("{path}: {message}"));
    }

    fn field(&mut self, name: &str, path: &str, value: &Value, top: bool) -> Option<FieldDef> {
        let before = self.errors.len();
        let mut def = Map::new();
        let spec = match value {
            Value::String(s) => {
                let (select, attr) = split_attr(s);
                def.insert("select".into(), Value::String(select.into()));
                if let Some(a) = attr {
                    def.insert("attr".into(), Value::String(a.into()));
                }
                &def
            }
            Value::Array(items) if items.len() == 1 => {
                let mut inner = self.field(name, path, &items[0], top)?;
                inner.all = true;
                return Some(inner);
            }
            Value::Object(o) => o,
            _ => {
                self.error(path, "a field is a selector, [selector] or an object");
                return None;
            }
        };
        for key in spec.keys() {
            if !KEYS.contains(&key.as_str()) {
                self.error(path, format!("unknown key {key:?}"));
            }
        }
        let flag = |k: &str| spec.get(k).and_then(Value::as_bool).unwrap_or(false);
        let string = |k: &str| spec.get(k).and_then(Value::as_str);

        let scoped = !top;
        let selector = match spec.get("select") {
            None | Some(Value::Null) => {
                if top {
                    self.error(path, "a top-level field needs a selector");
                }
                None
            }
            Some(Value::String(s)) => match Selector::new(s, scoped) {
                Ok(sel) => {
                    self.siblings |= sel.needs_siblings();
                    self.types |= sel.needs_types();
                    Some(sel)
                }
                Err(e) => {
                    self.error(path, format!("selector: {e}"));
                    None
                }
            },
            Some(_) => {
                self.error(path, "select is a string");
                None
            }
        };

        let read = if flag("exists") {
            Read::Exists
        } else if flag("html") {
            Read::Html
        } else if let Some(a) = string("attr") {
            Read::Attr(a.to_ascii_lowercase())
        } else {
            match string("text") {
                None | Some("visible") => Read::Visible,
                Some("raw") => Read::Raw,
                Some("own") => Read::Own,
                Some(other) => {
                    self.error(path, format!("text is \"visible\", \"raw\" or \"own\", not {other:?}"));
                    Read::Visible
                }
            }
        };
        let url = match spec.get("url") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => None,
            Some(Value::Bool(true)) => Some(UrlMode::Resolve),
            Some(Value::String(s)) if s == "http" => Some(UrlMode::Http),
            Some(_) => {
                self.error(path, "url is true or \"http\"");
                None
            }
        };
        let pattern = string("pattern").and_then(|p| {
            let source = if p.starts_with("(?") {
                p.to_owned()
            } else {
                format!("(?i){p}")
            };
            match Regex::new(&source) {
                Ok(re) => Some(re),
                Err(e) => {
                    self.error(path, format!("pattern: {e}"));
                    None
                }
            }
        });
        let then = if flag("number") {
            Then::Number
        } else if flag("json") {
            Then::Json
        } else if let Some(p) = spec.get("parse") {
            self.parse_field(path, p)
        } else {
            Then::Nothing
        };
        let collapse = spec
            .get("collapse")
            .and_then(Value::as_bool)
            .unwrap_or(matches!(read, Read::Visible | Read::Own | Read::Attr(_)));
        let emit = flag("emit");
        if emit && !(top && flag("all")) {
            self.error(path, "emit is for top-level fields with all");
        }
        if self.errors.len() > before {
            return None;
        }
        let object = match spec.get("fields") {
            Some(Value::Object(fields)) => {
                let at = self.objects.len();
                self.objects.push(ObjectDef::new(Vec::new()));
                let inner = self.object(fields, &format!("{path}{}", if flag("all") { "[]" } else { "" }), false);
                self.objects[at] = inner;
                Some(at)
            }
            Some(_) => {
                self.error(path, "fields is an object");
                None
            }
            None => None,
        };
        let limit = spec
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(usize::MAX, |n| n as usize);
        let max = spec.get("max").and_then(Value::as_u64).map(|n| n as usize);
        Some(FieldDef {
            name: name.to_owned(),
            path: path.to_owned(),
            selector,
            all: flag("all"),
            limit,
            read,
            url,
            collapse,
            max,
            pattern,
            then,
            object,
            default: spec.get("default").cloned(),
            emit,
        })
    }

    #[cfg(feature = "grammar")]
    fn parse_field(&mut self, path: &str, p: &Value) -> Then {
        match self.parse_def(path, p) {
            Some(d) => Then::Parse(Box::new(d)),
            None => Then::Nothing,
        }
    }

    #[cfg(not(feature = "grammar"))]
    fn parse_field(&mut self, path: &str, _: &Value) -> Then {
        let _ = self.grammars;
        self.error(path, "parse needs pestle-html's `grammar` feature");
        Then::Nothing
    }

    #[cfg(feature = "grammar")]
    fn parse_def(&mut self, path: &str, p: &Value) -> Option<ParseDef> {
        let empty = Map::new();
        let (reference, o) = match p {
            Value::String(s) => (s.as_str(), &empty),
            Value::Object(o) => match o.get("grammar").and_then(Value::as_str) {
                Some(g) => (g, o),
                None => {
                    self.error(path, "parse names a grammar");
                    return None;
                }
            },
            _ => {
                self.error(path, "parse is a grammar's name or an object");
                return None;
            }
        };
        // `name.rule`, unless the whole is a grammar's name.
        let (name, mut rule) = match (self.grammars.contains_key(reference), reference.split_once('.')) {
            (false, Some((g, r))) => (g, Some(r.to_owned())),
            _ => (reference, None),
        };
        if let Some(r) = o.get("rule").and_then(Value::as_str) {
            rule = Some(r.to_owned());
        }
        let Some(grammar) = self.grammars.get(name).cloned() else {
            self.error(path, format!("no grammar named {name:?}"));
            return None;
        };
        if let Some(r) = &rule {
            if grammar.rule_id(r).is_none() {
                self.error(path, format!("grammar {name:?} has no rule {r:?}"));
                return None;
            }
        } else if grammar.default_rule().is_none() {
            self.error(path, format!("grammar {name:?} defines no rule"));
            return None;
        }
        let set = |k: &str| -> HashSet<String> {
            o.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(ParseDef {
            grammar,
            name: name.to_owned(),
            rule,
            find: flag("find") || flag("all"),
            all: flag("all"),
            limit: o.get("limit").and_then(Value::as_u64).map_or(10_000, |n| n as usize),
            shape: ShapeOptions {
                lists: set("lists"),
                numbers: set("numbers"),
                by_rule: flag("byRule"),
                trim: flag("trim"),
            },
            tree: flag("tree"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::split_attr;

    #[test]
    fn attribute_shorthand() {
        assert_eq!(
            split_attr("meta[property='og:image']@content"),
            ("meta[property='og:image']", Some("content"))
        );
        assert_eq!(split_attr("a[href^='mailto:x@y']"), ("a[href^='mailto:x@y']", None));
        assert_eq!(
            split_attr("a[href^='mailto:x@y']@href"),
            ("a[href^='mailto:x@y']", Some("href"))
        );
        assert_eq!(split_attr("h1"), ("h1", None));
    }
}
