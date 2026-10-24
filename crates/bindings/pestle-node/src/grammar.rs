//! pest grammars compiled at run time (the `grammar` feature).

use std::sync::Arc;

use napi::bindgen_prelude::Uint32Array;
use napi::{Error, Result, Status};
use napi_derive::napi;
use pestle_grammar::{Grammar, ParseError, Recovery, ShapeOptions, Tree};

use crate::{coded, utf16_at};

// ── Grammars ─────────────────────────────────────────────────────────────

#[napi(object)]
pub struct NativeRule {
    pub name: String,
    pub kind: String,
}

#[napi(object)]
pub struct NativeError {
    pub message: String,
    pub line: u32,
    pub col: u32,
    /// UTF-16 offsets.
    pub offset: u32,
    pub end: Option<u32>,
    pub expected: Vec<String>,
    pub unexpected: Vec<String>,
}

pub(crate) fn native_error(e: ParseError, input: &str) -> NativeError {
    NativeError {
        message: e.message,
        line: e.line as u32,
        col: e.col as u32,
        offset: utf16_at(input, e.offset),
        end: e.end.map(|end| utf16_at(input, end)),
        expected: e.expected,
        unexpected: e.unexpected,
    }
}

pub(crate) fn error_json(e: &NativeError) -> serde_json::Value {
    serde_json::json!({
        "message": e.message, "line": e.line, "col": e.col, "offset": e.offset,
        "expected": e.expected, "unexpected": e.unexpected,
    })
}

/// A parse's pairs: `[rule, tag, start, end, size]` per pair (tag `0xFFFFFFFF`: none).
#[napi(object)]
pub struct NativeTree {
    pub nodes: Uint32Array,
    pub end: u32,
}

fn native_tree(tree: &Tree, input: &str) -> NativeTree {
    let spans = tree.utf16_spans(input);
    let mut flat = Vec::with_capacity(tree.nodes.len() * 5);
    for (n, (start, end)) in tree.nodes.iter().zip(spans) {
        flat.extend_from_slice(&[n.rule, n.tag, start, end, n.size]);
    }
    NativeTree {
        nodes: Uint32Array::new(flat),
        end: tree.utf16_end(input),
    }
}

#[napi(object)]
pub struct NativeParse {
    pub tree: Option<NativeTree>,
    pub error: Option<NativeError>,
}

#[napi(object)]
pub struct NativeRecovered {
    pub rule: String,
    /// UTF-16 offsets: where the rule started, where it broke, where the skip ended.
    pub start: u32,
    pub at: u32,
    pub end: u32,
    pub error: NativeError,
}

#[napi(object)]
pub struct NativeRecovery {
    pub rules: Vec<String>,
    pub sync: Vec<String>,
    pub consume: Option<bool>,
}

#[napi(object)]
pub struct NativeParseRecovering {
    pub tree: Option<NativeTree>,
    pub recovered: Vec<NativeRecovered>,
    pub error: Option<NativeError>,
}

#[napi(object)]
pub struct NativeShape {
    pub value: serde_json::Value,
    pub error: Option<NativeError>,
}

#[napi(object)]
pub struct NativeFound {
    pub start: u32,
    pub end: u32,
    pub tree: NativeTree,
}

#[napi(object)]
pub struct NativeShapeOptions {
    pub rule: Option<String>,
    pub lists: Option<Vec<String>>,
    pub numbers: Option<Vec<String>>,
    pub by_rule: Option<bool>,
    pub trim: Option<bool>,
}

/// A compiled grammar.
#[napi]
pub struct NativeGrammar {
    inner: Arc<Grammar>,
}

#[napi]
impl NativeGrammar {
    /// Throws `GRAMMAR:[errors]` when it does not compile.
    #[napi(constructor, catch_unwind)]
    pub fn new(source: String) -> Result<Self> {
        match Grammar::new(&source) {
            Ok(g) => Ok(NativeGrammar { inner: Arc::new(g) }),
            Err(e) => {
                let errors: Vec<serde_json::Value> = e
                    .errors
                    .into_iter()
                    .map(|e| error_json(&native_error(e, &source)))
                    .collect();
                Err(coded("GRAMMAR", serde_json::Value::Array(errors)))
            }
        }
    }

    #[napi(getter, catch_unwind)]
    pub fn rules(&self) -> Vec<NativeRule> {
        self.inner
            .rules()
            .iter()
            .map(|r| NativeRule {
                name: r.name.clone(),
                kind: r.kind.as_str().into(),
            })
            .collect()
    }

    /// Every rule name a pair can carry: the rules, then `EOI`.
    #[napi(getter, catch_unwind)]
    pub fn names(&self) -> Vec<String> {
        self.inner.names().to_vec()
    }

    #[napi(getter, catch_unwind)]
    pub fn tags(&self) -> Vec<String> {
        self.inner.tags().to_vec()
    }

    #[napi(getter, catch_unwind)]
    pub fn default_rule(&self) -> Option<String> {
        self.inner.default_rule().map(str::to_owned)
    }

    #[napi(catch_unwind)]
    pub fn parse(&self, input: String, rule: Option<String>) -> NativeParse {
        match self.inner.parse(rule.as_deref(), &input) {
            Ok(tree) => NativeParse {
                tree: Some(native_tree(&tree, &input)),
                error: None,
            },
            Err(e) => NativeParse {
                tree: None,
                error: Some(native_error(e, &input)),
            },
        }
    }

    /// Parses, recovering where `recovery` allows (see `Grammar::parse_recovering`).
    #[napi(catch_unwind)]
    pub fn parse_recovering(
        &self,
        input: String,
        rule: Option<String>,
        recovery: NativeRecovery,
    ) -> NativeParseRecovering {
        let recovery = Recovery {
            rules: recovery.rules,
            sync: recovery.sync,
            consume: recovery.consume.unwrap_or(false),
        };
        match self.inner.parse_recovering(rule.as_deref(), &input, &recovery) {
            Ok((tree, recovered)) => NativeParseRecovering {
                tree: Some(native_tree(&tree, &input)),
                recovered: recovered
                    .into_iter()
                    .map(|r| NativeRecovered {
                        rule: r.rule,
                        start: utf16_at(&input, r.start),
                        at: utf16_at(&input, r.at),
                        end: utf16_at(&input, r.end),
                        error: native_error(r.error, &input),
                    })
                    .collect(),
                error: None,
            },
            Err(e) => NativeParseRecovering {
                tree: None,
                recovered: Vec::new(),
                error: Some(native_error(e, &input)),
            },
        }
    }

    #[napi(catch_unwind)]
    pub fn shape(&self, input: String, options: Option<NativeShapeOptions>) -> NativeShape {
        let o = options.unwrap_or(NativeShapeOptions {
            rule: None,
            lists: None,
            numbers: None,
            by_rule: None,
            trim: None,
        });
        let shape = ShapeOptions {
            lists: o.lists.unwrap_or_default().into_iter().collect(),
            numbers: o.numbers.unwrap_or_default().into_iter().collect(),
            by_rule: o.by_rule.unwrap_or(false),
            trim: o.trim.unwrap_or(false),
        };
        match self.inner.shape(o.rule.as_deref(), &input, &shape) {
            Ok(value) => NativeShape { value, error: None },
            Err(e) => NativeShape {
                value: serde_json::Value::Null,
                error: Some(native_error(e, &input)),
            },
        }
    }

    #[napi(catch_unwind)]
    pub fn shaper(&self, options: Option<NativeShapeOptions>) -> NativeShaper {
        let o = options.unwrap_or(NativeShapeOptions {
            rule: None,
            lists: None,
            numbers: None,
            by_rule: None,
            trim: None,
        });
        NativeShaper {
            grammar: self.inner.clone(),
            rule: o.rule,
            options: ShapeOptions {
                lists: o.lists.unwrap_or_default().into_iter().collect(),
                numbers: o.numbers.unwrap_or_default().into_iter().collect(),
                by_rule: o.by_rule.unwrap_or(false),
                trim: o.trim.unwrap_or(false),
            },
        }
    }

    #[napi(catch_unwind)]
    pub fn find(&self, input: String, rule: Option<String>) -> Result<Option<NativeFound>> {
        let found = self
            .inner
            .find(rule.as_deref(), &input)
            .map_err(|e| Error::new(Status::InvalidArg, e.message))?;
        Ok(found.map(|f| NativeFound {
            start: utf16_at(&input, f.start),
            end: utf16_at(&input, f.end),
            tree: native_tree(&f.tree, &input),
        }))
    }

    #[napi(catch_unwind)]
    pub fn find_all(&self, input: String, rule: Option<String>, limit: Option<u32>) -> Result<Vec<NativeFound>> {
        let limit = limit.map_or(usize::MAX, |n| n as usize);
        let found = self
            .inner
            .find_all(rule.as_deref(), &input, limit)
            .map_err(|e| Error::new(Status::InvalidArg, e.message))?;
        Ok(found
            .into_iter()
            .map(|f| NativeFound {
                start: utf16_at(&input, f.start),
                end: utf16_at(&input, f.end),
                tree: native_tree(&f.tree, &input),
            })
            .collect())
    }
}

/// A grammar with its shape options compiled: what `Grammar.shape` calls.
#[napi]
pub struct NativeShaper {
    grammar: Arc<Grammar>,
    rule: Option<String>,
    options: ShapeOptions,
}

#[napi]
impl NativeShaper {
    /// The shaped value as JSON text; throws `PARSE:{error}` when `input` does not parse.
    #[napi(catch_unwind)]
    pub fn json(&self, input: String) -> Result<String> {
        let mut out = String::with_capacity(input.len() * 2 + 16);
        match self
            .grammar
            .shape_json(self.rule.as_deref(), &input, &self.options, &mut out)
        {
            Ok(()) => Ok(out),
            Err(e) => Err(coded("PARSE", error_json(&native_error(e, &input)))),
        }
    }

    /// Many inputs at once: a JSON array of their values, `null` for each that does not parse.
    #[napi(catch_unwind)]
    pub fn json_all(&self, inputs: Vec<String>) -> String {
        let mut out = String::with_capacity(inputs.iter().map(|i| i.len() * 2 + 4).sum::<usize>() + 2);
        out.push('[');
        for (i, input) in inputs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let mark = out.len();
            if self
                .grammar
                .shape_json(self.rule.as_deref(), input, &self.options, &mut out)
                .is_err()
            {
                out.truncate(mark);
                out.push_str("null");
            }
        }
        out.push(']');
        out
    }
}

/// Caps every parse at this many parser calls (none: no cap).
#[napi(catch_unwind)]
pub fn set_call_limit(limit: Option<u32>) {
    pestle_grammar::set_call_limit(limit.map(|n| n as usize));
}
