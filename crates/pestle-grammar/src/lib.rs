//! pest grammars compiled at run time.
//!
//! A [`Grammar`] is any grammar `pest_derive` accepts (parsed, validated and
//! optimised by `pest_meta`, node tags and the other `grammar-extras`
//! included), compiled so that every rule, built-in and Unicode property is
//! resolved once, and run on pest's own `ParserState`: a grammar means here
//! exactly what it means to `pest_derive`, without a build step.
//!
//! A parse gives a [`Tree`] (the pairs, flat, in document order), or a value
//! [shaped](Grammar::shape) by the grammar's node tags:
//!
//! ```
//! use pestle_grammar::{Grammar, ShapeOptions};
//!
//! let g = Grammar::new(r#"
//!     time = { #hour = num ~ ":" ~ #minute = num }
//!     num  = @{ ASCII_DIGIT{2} }
//! "#).unwrap();
//! let v = g.shape(None, "20:30", &ShapeOptions::default()).unwrap();
//! assert_eq!(v, serde_json::json!({ "hour": "20", "minute": "30" }));
//! ```

mod fast;
mod number;
mod shape;
mod tree;
mod vm;

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroUsize;

use pest::error::{Error as PestError, ErrorVariant, InputLocation, LineColLocation};
use pest_meta::ast::RuleType as MetaRuleType;
use pest_meta::optimizer::{OptimizedExpr, OptimizedRule};

pub use fast::Recovery;
pub use number::lenient_number;
pub use shape::ShapeOptions;
pub use tree::{Node, Tree, NO_TAG};
pub use vm::{RuleId, RuleKind};

use vm::{Builtin, CompiledRule, Expr, Program, Property};

/// A rule of a grammar: its name and kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleInfo {
    pub name: String,
    pub kind: RuleKind,
}

/// A failed parse, or one of a grammar's errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// pest's report: the message and the line it points at.
    pub message: String,
    /// 1-based line and column (in characters) where it failed.
    pub line: usize,
    pub col: usize,
    /// Where it failed, in bytes; a span's end, when it is one.
    pub offset: usize,
    pub end: Option<usize>,
    /// The rules that would have matched there, and those that must not have.
    pub expected: Vec<String>,
    pub unexpected: Vec<String>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    fn from_pest<R: pest::RuleType>(e: PestError<R>, name: impl Fn(&R) -> String) -> ParseError {
        let (expected, unexpected) = match &e.variant {
            ErrorVariant::ParsingError { positives, negatives } => (
                positives.iter().map(&name).collect(),
                negatives.iter().map(&name).collect(),
            ),
            ErrorVariant::CustomError { .. } => (Vec::new(), Vec::new()),
        };
        let (line, col) = match e.line_col {
            LineColLocation::Pos(p) | LineColLocation::Span(p, _) => p,
        };
        let (offset, end) = match e.location {
            InputLocation::Pos(p) => (p, None),
            InputLocation::Span((s, e)) => (s, Some(e)),
        };
        let message = e.renamed_rules(|r| name(r)).to_string();
        ParseError {
            message,
            line,
            col,
            offset,
            end,
            expected,
            unexpected,
        }
    }
}

/// A grammar that does not compile: every error pest_meta found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrammarError {
    pub errors: Vec<ParseError>,
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str("\n\n")?;
            }
            f.write_str(&e.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for GrammarError {}

/// Caps every parse, process-wide, at this many parser calls (none by default).
///
/// A parse past it fails with "call limit reached" instead of backtracking
/// for ever on a pathological grammar or input. Deep recursion is always
/// guarded: a parse that would run out of stack fails, it does not crash.
pub fn set_call_limit(limit: Option<usize>) {
    pest::set_call_limit(limit.and_then(NonZeroUsize::new));
    CALL_LIMIT.store(limit.unwrap_or(usize::MAX), std::sync::atomic::Ordering::Relaxed);
}

static CALL_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

pub(crate) fn call_limit() -> usize {
    CALL_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}

/// One place a recovering parse ([`Grammar::parse_recovering`]) skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    /// The rule that broke.
    pub rule: String,
    /// Where it started, where it broke (how far it matched), where the skip ended (bytes);
    /// the tree holds an `ERROR` pair from `start` to `end`.
    pub start: usize,
    pub at: usize,
    pub end: usize,
    /// What pest says the rule expected there.
    pub error: ParseError,
}

/// A compiled grammar.
pub struct Grammar {
    program: Program,
    fast: fast::Fast,
    /// Every rule's name by id; `EOI` last.
    names: Vec<String>,
    rules: Vec<RuleInfo>,
    by_name: HashMap<String, u32>,
    tags: Vec<String>,
    tag_ids: HashMap<String, u32>,
    default_rule: Option<u32>,
}

impl fmt::Debug for Grammar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grammar").field("rules", &self.rules).finish()
    }
}

/// One match of [`Grammar::find`]: where it starts and ends (bytes), and its tree.
#[derive(Clone, Debug)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub tree: Tree,
}

impl Grammar {
    /// Parses, validates and compiles a grammar.
    pub fn new(source: &str) -> Result<Grammar, GrammarError> {
        let (_, optimized) = pest_meta::parse_and_optimize(source).map_err(|errors| GrammarError {
            errors: errors
                .into_iter()
                .map(|e| ParseError::from_pest(e, pest_meta::parser::rename_meta_rule))
                .collect(),
        })?;
        Self::compile(optimized)
    }

    fn compile(optimized: Vec<OptimizedRule>) -> Result<Grammar, GrammarError> {
        let by_name: HashMap<String, u32> = optimized
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.clone(), i as u32))
            .collect();
        let eoi = optimized.len() as u32;
        let mut tags = Vec::new();
        let mut tag_ids = HashMap::new();
        let mut rules = Vec::with_capacity(optimized.len());
        let mut compiled = Vec::with_capacity(optimized.len());
        for r in &optimized {
            let kind = match r.ty {
                MetaRuleType::Normal => RuleKind::Normal,
                MetaRuleType::Silent => RuleKind::Silent,
                MetaRuleType::Atomic => RuleKind::Atomic,
                MetaRuleType::CompoundAtomic => RuleKind::CompoundAtomic,
                MetaRuleType::NonAtomic => RuleKind::NonAtomic,
            };
            let mut cx = Compiler {
                by_name: &by_name,
                tags: &mut tags,
                tag_ids: &mut tag_ids,
            };
            let expr = cx.expr(&r.expr).map_err(|message| GrammarError {
                errors: vec![ParseError {
                    message: format!("rule {}: {message}", r.name),
                    line: 1,
                    col: 1,
                    offset: 0,
                    end: None,
                    expected: Vec::new(),
                    unexpected: Vec::new(),
                }],
            })?;
            let implicit = r.name == "WHITESPACE" || r.name == "COMMENT";
            compiled.push(CompiledRule { kind, implicit, expr });
            rules.push(RuleInfo {
                name: r.name.clone(),
                kind,
            });
        }
        let mut names: Vec<String> = optimized.iter().map(|r| r.name.clone()).collect();
        names.push("EOI".to_owned());
        names.push("ERROR".to_owned());
        let default_rule = rules
            .iter()
            .position(|r| r.name != "WHITESPACE" && r.name != "COMMENT")
            .map(|i| i as u32);
        let program = Program {
            rules: compiled,
            eoi,
            whitespace: by_name.get("WHITESPACE").copied(),
            comment: by_name.get("COMMENT").copied(),
        };
        let fast = fast::Fast::compile(&program, &tag_ids);
        Ok(Grammar {
            program,
            fast,
            names,
            rules,
            by_name,
            tags,
            tag_ids,
            default_rule,
        })
    }

    /// The grammar's rules, in the order it defines them.
    pub fn rules(&self) -> &[RuleInfo] {
        &self.rules
    }

    /// Every name a [`Node::rule`] can refer to: the grammar's rules, then `EOI`, then `ERROR` (a recovered skip).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Every node tag the grammar uses; a [`Node::tag`] indexes it.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// A rule's id by name.
    pub fn rule_id(&self, name: &str) -> Option<RuleId> {
        self.by_name.get(name).map(|&i| RuleId(i))
    }

    /// The rule a parse starts from when none is named: the first the grammar defines.
    pub fn default_rule(&self) -> Option<&str> {
        self.default_rule.map(|i| self.names[i as usize].as_str())
    }

    fn resolve(&self, rule: Option<&str>) -> Result<u32, ParseError> {
        let found = match rule {
            Some(name) => self.by_name.get(name).copied(),
            None => self.default_rule,
        };
        found.ok_or_else(|| {
            let message = match rule {
                Some(name) => format!("no rule named {name:?}"),
                None => "the grammar defines no rule".to_owned(),
            };
            ParseError {
                message,
                line: 1,
                col: 1,
                offset: 0,
                end: None,
                expected: Vec::new(),
                unexpected: Vec::new(),
            }
        })
    }

    /// Parses `input` from its start with `rule` (the default rule when
    /// `None`). Like pest, a rule need not consume the whole input unless it
    /// says so (`… ~ EOI`); [`Tree::end`] says how far it read.
    pub fn parse(&self, rule: Option<&str>, input: &str) -> Result<Tree, ParseError> {
        let id = self.resolve(rule)?;
        self.run(id, input)
    }

    fn run(&self, id: u32, input: &str) -> Result<Tree, ParseError> {
        if let Some((tree, _)) = self.fast.run(id, input, None) {
            return Ok(tree);
        }
        self.run_pest(id, input)
    }

    /// The parse on pest's own `ParserState`: what a failure is reported by,
    /// and the answer should the fast engine ever disagree.
    fn run_pest(&self, id: u32, input: &str) -> Result<Tree, ParseError> {
        let end = Cell::new(0);
        let result = pest::state(input, |s| {
            let r = self.program.rule(id, s);
            if let Ok(s) = &r {
                end.set(s.position().pos());
            }
            r
        });
        match result {
            Ok(pairs) => Ok(Tree::from_pairs(pairs, end.get(), &self.tag_ids)),
            Err(e) => Err(ParseError::from_pest(e, |r| self.names[r.0 as usize].clone())),
        }
    }

    /// Whether `rule` matches at the start of `input`.
    pub fn matches(&self, rule: Option<&str>, input: &str) -> Result<bool, ParseError> {
        let id = self.resolve(rule)?;
        Ok(self.fast.run(id, input, None).is_some() || self.run_pest(id, input).is_ok())
    }

    /// Parses `input`, recovering where [`Recovery`] allows: a named rule
    /// that breaks after matching something skips to the next synchronising
    /// string, an `ERROR` pair stands for what it skipped, and the parse goes
    /// on. Fails only when the parse cannot complete even so.
    pub fn parse_recovering(
        &self,
        rule: Option<&str>,
        input: &str,
        recovery: &Recovery,
    ) -> Result<(Tree, Vec<Recovered>), ParseError> {
        let id = self.resolve(rule)?;
        let plan = self.fast.plan(recovery, &self.by_name).map_err(|message| ParseError {
            message,
            line: 1,
            col: 1,
            offset: 0,
            end: None,
            expected: Vec::new(),
            unexpected: Vec::new(),
        })?;
        let Some((tree, recovered)) = self.fast.run(id, input, Some(&plan)) else {
            return self.run_pest(id, input).map(|t| (t, Vec::new()));
        };
        let recovered = recovered
            .into_iter()
            .map(|r| Recovered {
                rule: self.names[r.rule as usize].clone(),
                start: r.start,
                at: r.at,
                end: r.end,
                error: self.error_at(r.rule, input, r.start),
            })
            .collect();
        Ok((tree, recovered))
    }

    /// Why `rule` fails at `start`, positioned in the whole input.
    fn error_at(&self, rule: u32, input: &str, start: usize) -> ParseError {
        let mut e = match self.run_pest(rule, &input[start..]) {
            Err(e) => e,
            Ok(_) => ParseError {
                message: String::new(),
                line: 0,
                col: 0,
                offset: 0,
                end: None,
                expected: Vec::new(),
                unexpected: Vec::new(),
            },
        };
        e.offset += start;
        e.end = e.end.map(|x| x + start);
        let before = &input[..e.offset.min(input.len())];
        e.line = before.matches('\n').count() + 1;
        e.col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        let what = if e.expected.is_empty() {
            "something else".to_owned()
        } else {
            e.expected.join(" or ")
        };
        e.message = format!(
            "{}:{}: {} broke here, expected {what}",
            e.line, e.col, self.names[rule as usize]
        );
        e
    }

    /// The first place in `input` where `rule` matches and reads at least
    /// one character, trying each character's start in turn (a search, the
    /// way a regex finds); its tree's offsets are the input's.
    pub fn find(&self, rule: Option<&str>, input: &str) -> Result<Option<Found>, ParseError> {
        let id = self.resolve(rule)?;
        Ok(self.find_from(id, input, 0))
    }

    /// Every match of `rule` in `input`, left to right, none overlapping.
    pub fn find_all(&self, rule: Option<&str>, input: &str, limit: usize) -> Result<Vec<Found>, ParseError> {
        let id = self.resolve(rule)?;
        let mut out = Vec::new();
        let mut at = 0;
        while out.len() < limit {
            match self.find_from(id, input, at) {
                Some(found) => {
                    at = found.end;
                    out.push(found);
                }
                None => break,
            }
        }
        Ok(out)
    }

    fn find_from(&self, id: u32, input: &str, from: usize) -> Option<Found> {
        let mut at = from;
        while at < input.len() {
            if let Some((mut tree, _)) = self.fast.run(id, &input[at..], None) {
                if tree.end > 0 {
                    tree.shift(at);
                    return Some(Found {
                        start: at,
                        end: tree.end,
                        tree,
                    });
                }
            }
            at += input[at..].chars().next().map_or(1, char::len_utf8);
        }
        None
    }

    /// Parses `input` and shapes the result by its node tags
    /// ([`ShapeOptions`]): a pair with tagged pairs beneath it is an object of
    /// them, by tag (a tag met twice an array); one without is its text.
    pub fn shape(
        &self,
        rule: Option<&str>,
        input: &str,
        options: &ShapeOptions,
    ) -> Result<serde_json::Value, ParseError> {
        let tree = self.parse(rule, input)?;
        Ok(self.shape_tree(&tree, input, options))
    }

    /// [`Grammar::shape`] as JSON text, appended to `out` (no value tree is built).
    pub fn shape_json(
        &self,
        rule: Option<&str>,
        input: &str,
        options: &ShapeOptions,
        out: &mut String,
    ) -> Result<(), ParseError> {
        let tree = self.parse(rule, input)?;
        shape::shape_json(&tree, input, &self.tags, &self.names, options, out);
        Ok(())
    }

    /// A tree already parsed from `input`, shaped.
    pub fn shape_tree(&self, tree: &Tree, input: &str, options: &ShapeOptions) -> serde_json::Value {
        shape::shape(tree, input, &self.tags, &self.names, options)
    }
}

struct Compiler<'g> {
    by_name: &'g HashMap<String, u32>,
    tags: &'g mut Vec<String>,
    tag_ids: &'g mut HashMap<String, u32>,
}

impl Compiler<'_> {
    fn expr(&mut self, e: &OptimizedExpr) -> Result<Expr, String> {
        Ok(match e {
            OptimizedExpr::Str(s) => Expr::Str(s.clone()),
            OptimizedExpr::Insens(s) => Expr::Insens(s.clone()),
            OptimizedExpr::Range(a, b) => {
                let first = |s: &str| {
                    s.chars()
                        .next()
                        .ok_or_else(|| "an empty character in a range".to_owned())
                };
                Expr::Range(first(a)?, first(b)?)
            }
            OptimizedExpr::Ident(name) => self.ident(name)?,
            OptimizedExpr::PeekSlice(start, end) => Expr::PeekSlice(*start, *end),
            OptimizedExpr::PosPred(e) => Expr::PosPred(Box::new(self.expr(e)?)),
            OptimizedExpr::NegPred(e) => Expr::NegPred(Box::new(self.expr(e)?)),
            OptimizedExpr::Seq(..) => {
                let mut items = Vec::new();
                self.flatten(e, true, &mut items)?;
                Expr::Seq(items)
            }
            OptimizedExpr::Choice(..) => {
                let mut items = Vec::new();
                self.flatten(e, false, &mut items)?;
                Expr::Choice(items)
            }
            OptimizedExpr::Opt(e) => Expr::Opt(Box::new(self.expr(e)?)),
            OptimizedExpr::Rep(e) => Expr::Rep(Box::new(self.expr(e)?)),
            OptimizedExpr::RepOnce(e) => Expr::RepOnce(Box::new(self.expr(e)?)),
            OptimizedExpr::Skip(strings) => Expr::Skip(strings.clone()),
            OptimizedExpr::Push(e) => Expr::Push(Box::new(self.expr(e)?)),
            OptimizedExpr::PushLiteral(s) => Expr::PushLiteral(s.clone()),
            OptimizedExpr::NodeTag(e, tag) => {
                if !self.tag_ids.contains_key(tag) {
                    self.tag_ids.insert(tag.clone(), self.tags.len() as u32);
                    self.tags.push(tag.clone());
                }
                Expr::Tag(Box::new(self.expr(e)?), tag.clone())
            }
            OptimizedExpr::RestoreOnErr(e) => Expr::RestoreOnErr(Box::new(self.expr(e)?)),
        })
    }

    /// `a ~ (b ~ c)` as `[a, b, c]` (and choices alike), as pest_derive generates them.
    fn flatten(&mut self, e: &OptimizedExpr, seq: bool, out: &mut Vec<Expr>) -> Result<(), String> {
        match (e, seq) {
            (OptimizedExpr::Seq(a, b), true) | (OptimizedExpr::Choice(a, b), false) => {
                self.flatten(a, seq, out)?;
                self.flatten(b, seq, out)
            }
            _ => {
                out.push(self.expr(e)?);
                Ok(())
            }
        }
    }

    fn ident(&self, name: &str) -> Result<Expr, String> {
        if let Some(&id) = self.by_name.get(name) {
            return Ok(Expr::Rule(id));
        }
        if name == "EOI" {
            return Ok(Expr::Eoi);
        }
        if let Some(b) = Builtin::by_name(name) {
            return Ok(Expr::Builtin(b));
        }
        if let Some(p) = Property::by_name(name) {
            return Ok(Expr::Unicode(p));
        }
        Err(format!("undefined rule {name}"))
    }
}

#[cfg(test)]
mod tests;
