//! pestle's own engine for a compiled grammar: pest's semantics (the same
//! pairs, tags, atomicity, implicit whitespace and stack), without pest's
//! cost on the way to a success.
//!
//! pest's `ParserState` tracks every attempt for its error messages, boxes
//! its state through every combinator, and asks the OS how much stack is
//! left on every call. A parse that succeeds needs none of that, so here:
//! bytes are matched in place, single-character alternatives are fused into
//! byte classes, a choice skips an alternative whose first byte cannot
//! match, implicit whitespace and repeated classes run as tight loops,
//! silent rules are inlined, the PUSH/POP stack is journaled only when the
//! grammar uses it, and the stack is checked every few levels of recursion.
//! A parse that fails is run again on pest's `ParserState`
//! ([`crate::vm`]), so errors are pest's own, word for word.
//!
//! It also recovers, when asked ([`Recovery`]): a named rule that breaks
//! after matching something skips to a synchronising string, leaves an
//! `ERROR` pair over what it skipped, and the parse goes on.

use std::collections::HashMap;

use crate::tree::{Node as Pair, Tree, NO_TAG};
use crate::vm::{Builtin, CompiledRule, Expr, Program, RuleKind};

/// 256 bits: the bytes a class matches.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ByteSet([u64; 4]);

impl ByteSet {
    fn add(&mut self, b: u8) {
        self.0[(b >> 6) as usize] |= 1 << (b & 63);
    }
    fn add_range(&mut self, a: u8, b: u8) {
        for x in a..=b {
            self.add(x);
        }
    }
    #[inline(always)]
    fn has(&self, b: u8) -> bool {
        self.0[(b >> 6) as usize] & (1 << (b & 63)) != 0
    }
    fn union(&mut self, o: &ByteSet) {
        for i in 0..4 {
            self.0[i] |= o.0[i];
        }
    }
    fn all() -> ByteSet {
        ByteSet([u64::MAX; 4])
    }
}

/// What can start a match of an expression.
#[derive(Clone, Copy)]
struct First {
    bytes: ByteSet,
    /// It can match nothing, or what it needs is unknown (stack, lookbehind…): never skip it.
    open: bool,
}

impl First {
    const OPEN: First = First {
        bytes: ByteSet([u64::MAX; 4]),
        open: true,
    };
    fn bytes(bytes: ByteSet) -> First {
        First { bytes, open: false }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Atomicity {
    Atomic,
    Compound,
    NonAtomic,
}

/// How a rule sets atomicity and whether it makes a pair, as pest_vm runs it.
#[derive(Clone, Copy)]
struct Shape {
    /// Atomicity set before the pair is opened.
    outer: Option<Atomicity>,
    pair: bool,
    /// Atomicity set inside the pair.
    inner: Option<Atomicity>,
}

enum Op {
    Str(Box<[u8]>),
    Insens(Box<[u8]>),
    Class(ByteSet),
    Range(char, char),
    Any,
    Soi,
    Eoi,
    Rule(u32),
    Unicode(crate::vm::Property),
    PeekSlice(i32, Option<i32>, bool),
    Pop,
    PopAll,
    Drop,
    PosPred(Box<Op>),
    NegPred(Box<Op>),
    Seq(Box<[Op]>),
    Choice(Box<[(First, Op)]>),
    Opt(Box<Op>),
    Rep(Box<Op>),
    RepOnce(Box<Op>),
    /// `class*` / `class+` (`once`), and with `negated` `(!class ~ ANY)*` / `+`: a scan, when
    /// no implicit whitespace can run between; else `general`.
    RepClass {
        set: ByteSet,
        negated: bool,
        once: bool,
        general: Box<Op>,
    },
    /// A sequence of literals, classes, `class?`, `class*` and `class+`: one greedy scan
    /// (exact for PEG, which never backtracks into them) when no implicit whitespace can run
    /// between; else `general`.
    Pattern {
        units: Box<[Unit]>,
        general: Box<Op>,
    },
    /// `!class ~ ANY`: one character that does not start with a byte of `set` (ASCII only).
    NotClass {
        set: ByteSet,
        general: Box<Op>,
    },
    Skip(Box<[Box<[u8]>]>),
    Push(Box<Op>),
    PushLiteral(Box<str>),
    Tag(Box<Op>, u32),
    RestoreOnErr(Box<Op>),
}

enum Unit {
    Lit(Box<[u8]>),
    Class(ByteSet),
    Opt(ByteSet),
    Star(ByteSet, bool),
}

struct FastRule {
    shape: Shape,
    op: Op,
}

/// How implicit whitespace is skipped.
enum Skipper {
    None,
    /// `WHITESPACE` is a silent class and there is no `COMMENT`: a loop.
    Class(ByteSet),
    /// The rules, as pest runs them.
    Rules(Option<u32>, Option<u32>),
}

/// The compiled program.
pub(crate) struct Fast {
    rules: Vec<FastRule>,
    skipper: Skipper,
    eoi: u32,
    error_rule: u32,
    uses_stack: bool,
}

/// Panic-mode recovery: which rules may recover, and where they pick up again.
#[derive(Clone, Debug, Default)]
pub struct Recovery {
    /// Rules that may recover (by name).
    pub rules: Vec<String>,
    /// Strings to resynchronise on: the skip ends at the first one found.
    pub sync: Vec<String>,
    /// The synchronising string is skipped too (default: it is left for the parse).
    pub consume: bool,
}

/// One place a parse recovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recovered {
    pub(crate) rule: u32,
    /// Where the rule started, where it broke (the furthest it matched), and where the skip ended.
    pub(crate) start: usize,
    pub(crate) at: usize,
    pub(crate) end: usize,
}

pub(crate) struct RecoverPlan {
    rules: Vec<bool>,
    sync: Vec<Box<[u8]>>,
    consume: bool,
}

impl Fast {
    pub(crate) fn compile(program: &Program, tag_ids: &HashMap<String, u32>) -> Fast {
        let n = program.rules.len();
        let mut c = Compiler {
            program,
            tag_ids,
            firsts: vec![None; n],
            visiting: vec![false; n],
            inline: vec![false; n],
        };
        // Inline silent, non-recursive, non-implicit rules.
        for i in 0..n {
            let r = &program.rules[i];
            c.inline[i] = r.kind == RuleKind::Silent
                && !r.implicit
                && !refers_to(&r.expr, i as u32, program, &mut vec![false; n]);
        }
        let rules = program
            .rules
            .iter()
            .map(|r| FastRule {
                shape: shape(r),
                op: c.op(&r.expr, 0),
            })
            .collect();
        let skipper = match (program.whitespace, program.comment) {
            (None, None) => Skipper::None,
            (Some(ws), None) => {
                let r = &program.rules[ws as usize];
                match (r.kind, class_of(&r.expr, program)) {
                    (RuleKind::Silent, Some(set)) => Skipper::Class(set),
                    _ => Skipper::Rules(Some(ws), None),
                }
            }
            (ws, comment) => Skipper::Rules(ws, comment),
        };
        let uses_stack = program.rules.iter().any(|r| touches_stack(&r.expr));
        Fast {
            rules,
            skipper,
            eoi: program.eoi,
            error_rule: program.eoi + 1,
            uses_stack,
        }
    }

    pub(crate) fn plan(&self, recovery: &Recovery, by_name: &HashMap<String, u32>) -> Result<RecoverPlan, String> {
        let mut rules = vec![false; self.rules.len()];
        for name in &recovery.rules {
            let id = by_name
                .get(name)
                .ok_or_else(|| format!("no rule named {name:?} to recover"))?;
            rules[*id as usize] = true;
        }
        Ok(RecoverPlan {
            rules,
            sync: recovery.sync.iter().map(|s| s.as_bytes().into()).collect(),
            consume: recovery.consume,
        })
    }

    /// Runs `rule` on `input`. None when it fails (or a limit is hit): run pest for the error.
    pub(crate) fn run(&self, rule: u32, input: &str, recover: Option<&RecoverPlan>) -> Option<(Tree, Vec<Recovered>)> {
        let mut r = Run {
            fast: self,
            input,
            bytes: input.as_bytes(),
            pos: 0,
            nodes: Vec::with_capacity((input.len() / 4).clamp(8, 1 << 16)),
            last_end: None,
            atom: Atomicity::NonAtomic,
            look: 0,
            stack: Vec::new(),
            journal: Vec::new(),
            snapshots: 0,
            depth: 0,
            calls: 0,
            limit: crate::call_limit(),
            failed_hard: false,
            furthest: 0,
            recover,
            recovered: Vec::new(),
        };
        let ok = r.rule(rule);
        if !ok || r.failed_hard {
            return None;
        }
        Some((
            Tree {
                nodes: r.nodes,
                end: r.pos,
            },
            r.recovered,
        ))
    }
}

fn shape(r: &CompiledRule) -> Shape {
    use Atomicity::*;
    let s = |outer, pair, inner| Shape { outer, pair, inner };
    if r.implicit {
        return match r.kind {
            RuleKind::Normal | RuleKind::Atomic => s(None, true, Some(Atomic)),
            RuleKind::Silent => s(Some(Atomic), false, None),
            RuleKind::CompoundAtomic => s(Some(Compound), true, None),
            // pest_vm sets Atomic before the pair: so none is made.
            RuleKind::NonAtomic => s(Some(Atomic), true, None),
        };
    }
    match r.kind {
        RuleKind::Normal => s(None, true, None),
        RuleKind::Silent => s(None, false, None),
        RuleKind::Atomic => s(None, true, Some(Atomic)),
        RuleKind::CompoundAtomic => s(Some(Compound), true, None),
        RuleKind::NonAtomic => s(Some(NonAtomic), true, None),
    }
}

fn touches_stack(e: &Expr) -> bool {
    match e {
        Expr::PeekSlice(..) | Expr::Push(_) | Expr::PushLiteral(_) => true,
        Expr::Builtin(b) => matches!(
            b,
            Builtin::Peek | Builtin::PeekAll | Builtin::Pop | Builtin::PopAll | Builtin::Drop
        ),
        Expr::PosPred(e)
        | Expr::NegPred(e)
        | Expr::Opt(e)
        | Expr::Rep(e)
        | Expr::RepOnce(e)
        | Expr::Tag(e, _)
        | Expr::RestoreOnErr(e) => touches_stack(e),
        Expr::Seq(items) | Expr::Choice(items) => items.iter().any(touches_stack),
        _ => false,
    }
}

fn refers_to(e: &Expr, target: u32, p: &Program, seen: &mut Vec<bool>) -> bool {
    match e {
        Expr::Rule(id) => {
            if *id == target {
                return true;
            }
            if seen[*id as usize] {
                return false;
            }
            seen[*id as usize] = true;
            refers_to(&p.rules[*id as usize].expr, target, p, seen)
        }
        Expr::PosPred(e)
        | Expr::NegPred(e)
        | Expr::Opt(e)
        | Expr::Rep(e)
        | Expr::RepOnce(e)
        | Expr::Push(e)
        | Expr::Tag(e, _)
        | Expr::RestoreOnErr(e) => refers_to(e, target, p, seen),
        Expr::Seq(items) | Expr::Choice(items) => items.iter().any(|i| refers_to(i, target, p, seen)),
        _ => false,
    }
}

/// The byte class an expression is, if it matches exactly one ASCII byte from a set.
fn class_of(e: &Expr, p: &Program) -> Option<ByteSet> {
    let mut set = ByteSet::default();
    match e {
        Expr::Str(s) if s.len() == 1 && s.as_bytes()[0] < 0x80 => set.add(s.as_bytes()[0]),
        Expr::Insens(s) if s.len() == 1 && s.as_bytes()[0] < 0x80 => {
            let b = s.as_bytes()[0];
            set.add(b.to_ascii_lowercase());
            set.add(b.to_ascii_uppercase());
        }
        Expr::Range(a, b) if (*a as u32) < 0x80 && (*b as u32) < 0x80 => {
            if a <= b {
                set.add_range(*a as u8, *b as u8)
            }
        }
        Expr::Builtin(b) => match b {
            Builtin::AsciiDigit => set.add_range(b'0', b'9'),
            Builtin::AsciiNonzeroDigit => set.add_range(b'1', b'9'),
            Builtin::AsciiBinDigit => set.add_range(b'0', b'1'),
            Builtin::AsciiOctDigit => set.add_range(b'0', b'7'),
            Builtin::AsciiHexDigit => {
                set.add_range(b'0', b'9');
                set.add_range(b'a', b'f');
                set.add_range(b'A', b'F');
            }
            Builtin::AsciiAlphaLower => set.add_range(b'a', b'z'),
            Builtin::AsciiAlphaUpper => set.add_range(b'A', b'Z'),
            Builtin::AsciiAlpha => {
                set.add_range(b'a', b'z');
                set.add_range(b'A', b'Z');
            }
            Builtin::AsciiAlphanumeric => {
                set.add_range(b'a', b'z');
                set.add_range(b'A', b'Z');
                set.add_range(b'0', b'9');
            }
            Builtin::Ascii => set.add_range(0, 0x7f),
            _ => return None,
        },
        Expr::Choice(items) => {
            for i in items {
                set.union(&class_of(i, p)?);
            }
        }
        // A silent, non-implicit rule is just its expression.
        Expr::Rule(id) => {
            let r = &p.rules[*id as usize];
            if r.kind != RuleKind::Silent || r.implicit {
                return None;
            }
            return class_of(&r.expr, p);
        }
        _ => return None,
    }
    Some(set)
}

/// `!class ~ ANY`, as its class.
fn not_class_of(e: &Expr, p: &Program) -> Option<ByteSet> {
    match e {
        Expr::Seq(items) if items.len() == 2 && matches!(items[1], Expr::Builtin(Builtin::Any)) => match &items[0] {
            Expr::NegPred(x) => class_of(x, p),
            _ => None,
        },
        _ => None,
    }
}

struct Compiler<'p> {
    program: &'p Program,
    tag_ids: &'p HashMap<String, u32>,
    firsts: Vec<Option<First>>,
    visiting: Vec<bool>,
    inline: Vec<bool>,
}

impl Compiler<'_> {
    fn op(&mut self, e: &Expr, depth: usize) -> Op {
        if let Some(set) = class_of(e, self.program) {
            // A rule reference that is a class is inlined as one: it makes no pair.
            return Op::Class(set);
        }
        match e {
            Expr::Str(s) => Op::Str(s.as_bytes().into()),
            Expr::Insens(s) => Op::Insens(s.as_bytes().into()),
            Expr::Range(a, b) => Op::Range(*a, *b),
            Expr::Rule(id) if self.inline[*id as usize] && depth < 8 => {
                let expr = &self.program.rules[*id as usize].expr;
                self.op(expr, depth + 1)
            }
            Expr::Rule(id) => Op::Rule(*id),
            Expr::Eoi => Op::Eoi,
            Expr::Builtin(b) => match b {
                Builtin::Any => Op::Any,
                Builtin::Soi => Op::Soi,
                Builtin::Peek => Op::PeekSlice(-1, None, false),
                Builtin::PeekAll => Op::PeekSlice(0, None, true),
                Builtin::Pop => Op::Pop,
                Builtin::PopAll => Op::PopAll,
                Builtin::Drop => Op::Drop,
                Builtin::Newline => Op::Choice(
                    [&b"\n"[..], b"\r\n", b"\r"]
                        .iter()
                        .map(|s| {
                            (
                                First::bytes({
                                    let mut b = ByteSet::default();
                                    b.add(s[0]);
                                    b
                                }),
                                Op::Str((*s).into()),
                            )
                        })
                        .collect(),
                ),
                // The class builtins were taken by `class_of`.
                _ => unreachable!("class builtin"),
            },
            Expr::Unicode(p) => Op::Unicode(p.clone()),
            Expr::PeekSlice(a, b) => Op::PeekSlice(*a, *b, false),
            Expr::PosPred(e) => Op::PosPred(Box::new(self.op(e, depth))),
            Expr::NegPred(e) => Op::NegPred(Box::new(self.op(e, depth))),
            Expr::Seq(items) => {
                let general = Op::Seq(items.iter().map(|i| self.op(i, depth)).collect());
                if let Some(set) = not_class_of(e, self.program) {
                    return Op::NotClass {
                        set,
                        general: Box::new(general),
                    };
                }
                match items.iter().map(|i| self.unit(i)).collect::<Option<Vec<Unit>>>() {
                    Some(units) => Op::Pattern {
                        units: units.into(),
                        general: Box::new(general),
                    },
                    None => general,
                }
            }
            Expr::Choice(items) => Op::Choice(items.iter().map(|i| (self.first(i), self.op(i, depth))).collect()),
            Expr::Opt(e) => Op::Opt(Box::new(self.op(e, depth))),
            Expr::Rep(inner) | Expr::RepOnce(inner) => {
                let once = matches!(e, Expr::RepOnce(_));
                let one = self.op(inner, depth);
                let found = class_of(inner, self.program)
                    .map(|s| (s, false))
                    .or_else(|| not_class_of(inner, self.program).map(|s| (s, true)));
                let general = if once {
                    Op::RepOnce(Box::new(one))
                } else {
                    Op::Rep(Box::new(one))
                };
                match found {
                    Some((set, negated)) => Op::RepClass {
                        set,
                        negated,
                        once,
                        general: Box::new(general),
                    },
                    None => general,
                }
            }
            Expr::Skip(strings) => Op::Skip(strings.iter().map(|s| s.as_bytes().into()).collect()),
            Expr::Push(e) => Op::Push(Box::new(self.op(e, depth))),
            Expr::PushLiteral(s) => Op::PushLiteral(s.as_str().into()),
            Expr::Tag(e, tag) => Op::Tag(Box::new(self.op(e, depth)), self.tag_id(tag)),
            Expr::RestoreOnErr(e) => Op::RestoreOnErr(Box::new(self.op(e, depth))),
        }
    }

    /// An expression as a unit of a byte pattern, if it is one.
    fn unit(&self, e: &Expr) -> Option<Unit> {
        if let Some(set) = class_of(e, self.program) {
            return Some(Unit::Class(set));
        }
        match e {
            Expr::Str(s) if !s.is_empty() => Some(Unit::Lit(s.as_bytes().into())),
            Expr::Opt(x) => class_of(x, self.program).map(Unit::Opt),
            Expr::Rep(x) => class_of(x, self.program).map(|s| Unit::Star(s, false)),
            Expr::RepOnce(x) => class_of(x, self.program).map(|s| Unit::Star(s, true)),
            _ => None,
        }
    }

    fn tag_id(&self, tag: &str) -> u32 {
        self.tag_ids.get(tag).copied().unwrap_or(NO_TAG)
    }

    /// What can start a match (a necessary condition for it to succeed).
    fn first(&mut self, e: &Expr) -> First {
        if let Some(set) = class_of(e, self.program) {
            return First::bytes(set);
        }
        match e {
            Expr::Str(s) => match s.as_bytes().first() {
                Some(&b) => First::bytes({
                    let mut x = ByteSet::default();
                    x.add(b);
                    x
                }),
                None => First::OPEN,
            },
            Expr::Insens(s) => match s.as_bytes().first() {
                Some(&b) => First::bytes({
                    let mut x = ByteSet::default();
                    x.add(b.to_ascii_lowercase());
                    x.add(b.to_ascii_uppercase());
                    x
                }),
                None => First::OPEN,
            },
            Expr::Range(a, b) => {
                let mut x = ByteSet::default();
                if (*a as u32) < 0x80 {
                    x.add_range(*a as u8, (*b as u32).min(0x7f) as u8);
                }
                if (*b as u32) >= 0x80 {
                    x.add_range(0x80, 0xff);
                }
                First::bytes(x)
            }
            Expr::Builtin(Builtin::Any) | Expr::Unicode(_) => First::bytes(ByteSet::all()),
            Expr::Builtin(Builtin::Newline) => First::bytes({
                let mut x = ByteSet::default();
                x.add(b'\n');
                x.add(b'\r');
                x
            }),
            Expr::Rule(id) => {
                let id = *id as usize;
                if let Some(f) = self.firsts[id] {
                    return f;
                }
                if self.visiting[id] {
                    return First::OPEN;
                }
                self.visiting[id] = true;
                let expr = &self.program.rules[id].expr;
                let f = if touches_stack(expr) {
                    First::OPEN
                } else {
                    self.first(expr)
                };
                self.visiting[id] = false;
                self.firsts[id] = Some(f);
                f
            }
            Expr::Seq(items) => {
                // The first element that consumes decides; a negative lookahead before it is passed over,
                // unless implicit whitespace may run between them.
                let mut negated = false;
                for item in items {
                    if let Expr::NegPred(_) = item {
                        negated = true;
                        continue;
                    }
                    if negated && (self.program.whitespace.is_some() || self.program.comment.is_some()) {
                        return First::OPEN;
                    }
                    return self.first(item);
                }
                First::OPEN
            }
            Expr::Choice(items) => {
                let mut x = ByteSet::default();
                for i in items {
                    let f = self.first(i);
                    if f.open {
                        return First::OPEN;
                    }
                    x.union(&f.bytes);
                }
                First::bytes(x)
            }
            Expr::PosPred(e) | Expr::RepOnce(e) | Expr::Tag(e, _) => self.first(e),
            Expr::RestoreOnErr(e) if !touches_stack(e) => self.first(e),
            _ => First::OPEN,
        }
    }
}

#[derive(Clone)]
enum StackItem {
    Span(usize, usize),
    Literal(Box<str>),
}

enum Undo {
    Pushed,
    Popped(StackItem),
}

#[derive(Clone, Copy)]
struct Checkpoint {
    pos: usize,
    nodes: usize,
    last_end: Option<u32>,
    journal: usize,
}

struct Run<'a> {
    fast: &'a Fast,
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    nodes: Vec<Pair>,
    /// The pair whose end was the last token (what a tag tags).
    last_end: Option<u32>,
    atom: Atomicity,
    look: u32,
    stack: Vec<StackItem>,
    journal: Vec<Undo>,
    snapshots: u32,
    depth: u32,
    calls: usize,
    limit: usize,
    /// A limit was hit: the parse fails, whatever backtracking would do.
    failed_hard: bool,
    furthest: usize,
    recover: Option<&'a RecoverPlan>,
    recovered: Vec<Recovered>,
}

#[inline(always)]
fn utf8_len(b: u8) -> usize {
    match b {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

const STACK_CHECK_EVERY: u32 = 16;
const STACK_RESERVE: usize = 128 * 1024;

impl Run<'_> {
    #[inline]
    fn checkpoint(&mut self) -> Checkpoint {
        if self.fast.uses_stack {
            self.snapshots += 1;
        }
        Checkpoint {
            pos: self.pos,
            nodes: self.nodes.len(),
            last_end: self.last_end,
            journal: self.journal.len(),
        }
    }

    #[inline]
    fn commit(&mut self, _cp: Checkpoint) {
        if self.fast.uses_stack {
            self.snapshots -= 1;
            if self.snapshots == 0 {
                self.journal.clear();
            }
        }
    }

    #[inline]
    fn restore(&mut self, cp: Checkpoint) {
        self.pos = cp.pos;
        self.nodes.truncate(cp.nodes);
        self.last_end = cp.last_end;
        if self.fast.uses_stack {
            self.undo_to(cp.journal);
            self.snapshots -= 1;
            if self.snapshots == 0 {
                self.journal.clear();
            }
        }
    }

    fn undo_to(&mut self, mark: usize) {
        while self.journal.len() > mark {
            match self.journal.pop() {
                Some(Undo::Pushed) => {
                    self.stack.pop();
                }
                Some(Undo::Popped(item)) => self.stack.push(item),
                None => break,
            }
        }
    }

    fn push_stack(&mut self, item: StackItem) {
        self.stack.push(item);
        if self.snapshots > 0 {
            self.journal.push(Undo::Pushed);
        }
    }

    fn pop_stack(&mut self) -> Option<StackItem> {
        let item = self.stack.pop()?;
        if self.snapshots > 0 {
            self.journal.push(Undo::Popped(item.clone()));
        }
        Some(item)
    }

    fn item_bytes<'s>(&'s self, item: &'s StackItem) -> &'s [u8] {
        match item {
            StackItem::Span(a, b) => &self.bytes[*a..*b],
            StackItem::Literal(s) => s.as_bytes(),
        }
    }

    #[inline(always)]
    fn advance(&mut self, to: usize) {
        self.pos = to;
        // How far a recovering rule got; a plain parse does not look.
        if self.recover.is_some() && to > self.furthest {
            self.furthest = to;
        }
    }

    /// A failed scan got as far as `p`: what a recovering rule needs to know.
    #[inline(always)]
    fn reached(&mut self, p: usize) {
        if self.recover.is_some() && p > self.furthest {
            self.furthest = p;
        }
    }

    #[inline(always)]
    fn match_bytes(&mut self, s: &[u8]) -> bool {
        let end = self.pos + s.len();
        if self.bytes.get(self.pos..end) == Some(s) {
            self.advance(end);
            true
        } else {
            false
        }
    }

    /// No implicit whitespace can run here.
    #[inline(always)]
    fn no_skip(&self) -> bool {
        self.atom != Atomicity::NonAtomic || matches!(self.fast.skipper, Skipper::None)
    }

    fn skip(&mut self) {
        if self.atom != Atomicity::NonAtomic {
            return;
        }
        match &self.fast.skipper {
            Skipper::None => {}
            Skipper::Class(set) => {
                let mut p = self.pos;
                while p < self.bytes.len() && set.has(self.bytes[p]) {
                    p += 1;
                }
                self.pos = p;
            }
            &Skipper::Rules(ws, comment) => match (ws, comment) {
                (Some(ws), None) => while self.rule(ws) {},
                (None, Some(c)) => while self.rule(c) {},
                (Some(ws), Some(c)) => {
                    while self.rule(ws) {}
                    loop {
                        let cp = self.checkpoint();
                        if self.rule(c) {
                            while self.rule(ws) {}
                            self.commit(cp);
                        } else {
                            self.restore(cp);
                            break;
                        }
                    }
                }
                (None, None) => {}
            },
        }
    }

    #[inline]
    fn rule(&mut self, id: u32) -> bool {
        if self.recover.is_some() {
            return self.rule_recovering(id);
        }
        self.calls += 1;
        if self.calls > self.limit {
            self.failed_hard = true;
            return false;
        }
        self.depth += 1;
        if self.depth.is_multiple_of(STACK_CHECK_EVERY) && stacker::remaining_stack().is_some_and(|r| r < STACK_RESERVE)
        {
            self.failed_hard = true;
            self.depth -= 1;
            return false;
        }
        let fast = self.fast;
        let r = &fast.rules[id as usize];
        let saved_atom = self.atom;
        if let Some(a) = r.shape.outer {
            self.atom = a;
        }
        let pair = r.shape.pair && self.look == 0 && self.atom != Atomicity::Atomic;
        let index = self.nodes.len();
        let saved_last = self.last_end;
        if pair {
            self.nodes.push(Pair {
                rule: id,
                tag: NO_TAG,
                start: self.pos as u32,
                end: 0,
                size: 0,
            });
            self.last_end = None;
        }
        if let Some(a) = r.shape.inner {
            self.atom = a;
        }
        let ok = self.op(&r.op);
        self.atom = saved_atom;
        if pair {
            if ok {
                let size = (self.nodes.len() - index - 1) as u32;
                let n = &mut self.nodes[index];
                n.end = self.pos as u32;
                n.size = size;
                self.last_end = Some(index as u32);
            } else {
                self.nodes.truncate(index);
                self.last_end = saved_last;
            }
        }
        self.depth -= 1;
        ok
    }

    /// [`Run::rule`] with recovery: a recovering rule that breaks after matching something skips and goes on.
    #[inline(never)]
    fn rule_recovering(&mut self, id: u32) -> bool {
        if self.failed_hard {
            return false;
        }
        self.calls += 1;
        if self.calls > self.limit {
            self.failed_hard = true;
            return false;
        }
        self.depth += 1;
        if self.depth.is_multiple_of(STACK_CHECK_EVERY) && stacker::remaining_stack().is_some_and(|r| r < STACK_RESERVE)
        {
            self.failed_hard = true;
            self.depth -= 1;
            return false;
        }
        let fast = self.fast;
        let r = &fast.rules[id as usize];
        let start = self.pos;
        let saved_atom = self.atom;
        if let Some(a) = r.shape.outer {
            self.atom = a;
        }
        let pair = r.shape.pair && self.look == 0 && self.atom != Atomicity::Atomic;
        let index = self.nodes.len();
        let saved_last = self.last_end;
        if pair {
            self.nodes.push(Pair {
                rule: id,
                tag: NO_TAG,
                start: start as u32,
                end: 0,
                size: 0,
            });
            self.last_end = None;
        }
        let atom_in_pair = self.atom;
        if let Some(a) = r.shape.inner {
            self.atom = a;
        }
        let recovering = self.recover.is_some_and(|p| p.rules[id as usize]) && self.look == 0;
        let saved_furthest = self.furthest;
        if recovering {
            self.furthest = start;
        }
        let mut ok = self.op(&r.op);
        if !ok && recovering && !self.failed_hard && self.furthest > start {
            ok = self.recover_at(id, start, index, pair, saved_last, atom_in_pair);
        }
        if recovering {
            self.furthest = self.furthest.max(saved_furthest);
        }
        self.atom = saved_atom;
        if pair && ok && self.nodes.get(index).is_some_and(|n| n.rule == id) {
            let size = (self.nodes.len() - index - 1) as u32;
            let n = &mut self.nodes[index];
            n.end = self.pos as u32;
            n.size = size;
            self.last_end = Some(index as u32);
        } else if pair && !ok {
            self.nodes.truncate(index);
            self.last_end = saved_last;
        }
        self.depth -= 1;
        ok
    }

    /// A recovering rule broke at `self.furthest`: skip to the next synchronising string and stand in an `ERROR` pair.
    fn recover_at(
        &mut self,
        id: u32,
        start: usize,
        index: usize,
        pair: bool,
        saved_last: Option<u32>,
        _atom: Atomicity,
    ) -> bool {
        let plan = self.recover.expect("recovering");
        let at = self.furthest;
        let from = at.max(start + 1).min(self.bytes.len());
        let mut end = self.bytes.len();
        for s in &plan.sync {
            if s.is_empty() {
                continue;
            }
            if let Some(i) = memchr::memmem::find(&self.bytes[from..], s) {
                let e = from + i + if plan.consume { s.len() } else { 0 };
                end = end.min(e);
            }
        }
        // At least one character, on a character boundary.
        let mut end = end.max(start + 1).min(self.bytes.len());
        while end < self.bytes.len() && !self.input.is_char_boundary(end) {
            end += 1;
        }
        if end <= start {
            return false;
        }
        self.nodes.truncate(if pair { index } else { self.nodes.len() });
        if self.look == 0 && self.atom != Atomicity::Atomic {
            let error = self.fast.error_rule;
            self.nodes.push(Pair {
                rule: error,
                tag: NO_TAG,
                start: start as u32,
                end: end as u32,
                size: 0,
            });
            self.last_end = Some(self.nodes.len() as u32 - 1);
        } else {
            self.last_end = saved_last;
        }
        self.pos = end;
        self.recovered.push(Recovered {
            rule: id,
            start,
            at,
            end,
        });
        true
    }

    #[inline]
    fn op(&mut self, op: &Op) -> bool {
        match op {
            Op::Str(s) => self.match_bytes(s),
            Op::Pattern { units, general } => {
                if self.no_skip() {
                    self.pattern(units)
                } else {
                    self.op(general)
                }
            }
            Op::Insens(..) => self.op_insens(op),
            Op::Class(set) => match self.bytes.get(self.pos) {
                Some(&b) if set.has(b) => {
                    self.advance(self.pos + 1);
                    true
                }
                _ => false,
            },
            Op::Range(..) => self.op_range(op),
            Op::Any => self.op_any(op),
            Op::Soi => self.pos == 0,
            Op::Eoi => self.op_eoi(op),
            Op::Rule(id) => self.rule(*id),
            Op::Unicode(..) => self.op_unicode(op),
            Op::PeekSlice(..) => self.op_peek_slice(op),
            Op::Pop => self.op_pop(op),
            Op::PopAll => self.op_pop_all(op),
            Op::Drop => self.op_drop(op),
            Op::PosPred(..) | Op::NegPred(..) => self.op_pos_pred(op),
            Op::Seq(..) => self.op_seq(op),
            Op::Choice(..) => self.op_choice(op),
            Op::Opt(..) => self.op_opt(op),
            Op::Rep(..) => self.op_rep(op),
            Op::RepOnce(..) => self.op_rep_once(op),
            Op::RepClass { .. } => self.op_rep_class(op),
            Op::NotClass { .. } => self.op_not_class(op),
            Op::Skip(..) => self.op_skip(op),
            Op::Push(..) => self.op_push(op),
            Op::PushLiteral(..) => self.op_push_literal(op),
            Op::Tag(..) => self.op_tag(op),
            Op::RestoreOnErr(..) => self.op_restore_on_err(op),
        }
    }

    #[inline(never)]
    fn op_insens(&mut self, op: &Op) -> bool {
        let Op::Insens(s) = op else { unreachable!() };
        {
            let end = self.pos + s.len();
            match self.bytes.get(self.pos..end) {
                Some(slice) if slice.eq_ignore_ascii_case(s) => {
                    self.advance(end);
                    true
                }
                _ => false,
            }
        }
    }

    #[inline(never)]
    fn op_range(&mut self, op: &Op) -> bool {
        let Op::Range(a, b) = op else { unreachable!() };
        {
            match self.input[self.pos..].chars().next() {
                Some(c) if *a <= c && c <= *b => {
                    self.advance(self.pos + c.len_utf8());
                    true
                }
                _ => false,
            }
        }
    }

    #[inline(never)]
    fn op_any(&mut self, op: &Op) -> bool {
        let Op::Any = op else { unreachable!() };
        {
            match self.input[self.pos..].chars().next() {
                Some(c) => {
                    self.advance(self.pos + c.len_utf8());
                    true
                }
                None => false,
            }
        }
    }

    #[inline(never)]
    fn op_eoi(&mut self, op: &Op) -> bool {
        let Op::Eoi = op else { unreachable!() };
        {
            // `EOI` is a rule: a pair, unless atomic or in a lookahead.
            if self.pos != self.bytes.len() {
                return false;
            }
            if self.look == 0 && self.atom != Atomicity::Atomic {
                let p = self.pos as u32;
                self.nodes.push(Pair {
                    rule: self.fast.eoi,
                    tag: NO_TAG,
                    start: p,
                    end: p,
                    size: 0,
                });
                self.last_end = Some(self.nodes.len() as u32 - 1);
            }
            true
        }
    }

    #[inline(never)]
    fn op_unicode(&mut self, op: &Op) -> bool {
        let Op::Unicode(p) = op else { unreachable!() };
        {
            match self.input[self.pos..].chars().next() {
                Some(c) if p.test(c) => {
                    self.advance(self.pos + c.len_utf8());
                    true
                }
                _ => false,
            }
        }
    }

    #[inline(never)]
    fn op_peek_slice(&mut self, op: &Op) -> bool {
        let Op::PeekSlice(start, end, top_down) = op else {
            unreachable!()
        };
        {
            self.peek_slice(*start, *end, *top_down)
        }
    }

    #[inline(never)]
    fn op_pop(&mut self, op: &Op) -> bool {
        let Op::Pop = op else { unreachable!() };
        {
            match self.pop_stack() {
                Some(item) => {
                    let s: Vec<u8> = self.item_bytes(&item).to_vec();
                    self.match_bytes(&s)
                }
                None => false,
            }
        }
    }

    #[inline(never)]
    fn op_pop_all(&mut self, op: &Op) -> bool {
        let Op::PopAll = op else { unreachable!() };
        {
            let mut p = self.pos;
            let mut ok = true;
            while let Some(item) = self.pop_stack() {
                let s = self.item_bytes(&item);
                if self.bytes.get(p..p + s.len()) == Some(s) {
                    p += s.len();
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                self.advance(p);
            }
            ok
        }
    }

    #[inline(never)]
    fn op_drop(&mut self, op: &Op) -> bool {
        let Op::Drop = op else { unreachable!() };
        {
            self.pop_stack().is_some()
        }
    }

    #[inline(never)]
    fn op_pos_pred(&mut self, op: &Op) -> bool {
        let (Op::PosPred(e) | Op::NegPred(e)) = op else {
            unreachable!()
        };
        {
            let positive = matches!(op, Op::PosPred(_));
            self.look += 1;
            let cp = self.checkpoint();
            let ok = self.op(e);
            self.restore(cp);
            self.look -= 1;
            if self.failed_hard {
                return false;
            }
            ok == positive
        }
    }

    #[inline(never)]
    fn op_seq(&mut self, op: &Op) -> bool {
        let Op::Seq(items) = op else { unreachable!() };
        {
            let cp = self.checkpoint();
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.skip();
                }
                if !self.op(item) {
                    self.restore(cp);
                    return false;
                }
            }
            self.commit(cp);
            true
        }
    }

    #[inline(never)]
    fn op_choice(&mut self, op: &Op) -> bool {
        let Op::Choice(alts) = op else { unreachable!() };
        {
            let next = self.bytes.get(self.pos).copied();
            for (first, alt) in alts.iter() {
                if !first.open && !next.is_some_and(|b| first.bytes.has(b)) {
                    continue;
                }
                if self.op(alt) {
                    return true;
                }
                if self.failed_hard {
                    return false;
                }
            }
            false
        }
    }

    #[inline(never)]
    fn op_opt(&mut self, op: &Op) -> bool {
        let Op::Opt(e) = op else { unreachable!() };
        {
            self.op(e);
            !self.failed_hard
        }
    }

    #[inline(never)]
    fn op_rep(&mut self, op: &Op) -> bool {
        let Op::Rep(e) = op else { unreachable!() };
        {
            if self.op(e) {
                self.repeat(e);
            }
            !self.failed_hard
        }
    }

    #[inline(never)]
    fn op_rep_once(&mut self, op: &Op) -> bool {
        let Op::RepOnce(e) = op else { unreachable!() };
        {
            if !self.op(e) {
                return false;
            }
            self.repeat(e);
            !self.failed_hard
        }
    }

    #[inline(never)]
    fn op_rep_class(&mut self, op: &Op) -> bool {
        let Op::RepClass {
            set,
            negated,
            once,
            general,
        } = op
        else {
            unreachable!()
        };
        {
            if !self.no_skip() {
                return self.op(general);
            }
            let start = self.pos;
            let mut p = start;
            let n = self.bytes.len();
            // A negated class holds only ASCII: a scan stops on character boundaries.
            if *negated {
                while p < n && !set.has(self.bytes[p]) {
                    p += 1;
                }
            } else {
                while p < n && set.has(self.bytes[p]) {
                    p += 1;
                }
            }
            if *once && p == start {
                return false;
            }
            self.advance(p);
            true
        }
    }

    #[inline(never)]
    fn op_not_class(&mut self, op: &Op) -> bool {
        let Op::NotClass { set, general } = op else {
            unreachable!()
        };
        {
            if !self.no_skip() {
                return self.op(general);
            }
            match self.bytes.get(self.pos) {
                Some(&b) if !set.has(b) => {
                    let len = utf8_len(b);
                    self.advance((self.pos + len).min(self.bytes.len()));
                    true
                }
                _ => false,
            }
        }
    }

    #[inline(never)]
    fn op_skip(&mut self, op: &Op) -> bool {
        let Op::Skip(strings) = op else { unreachable!() };
        {
            let rest = &self.bytes[self.pos..];
            let mut best: Option<usize> = None;
            for s in strings.iter() {
                if let Some(i) = memchr::memmem::find(rest, s) {
                    best = Some(best.map_or(i, |b| b.min(i)));
                }
            }
            self.pos += best.unwrap_or(rest.len());
            true
        }
    }

    #[inline(never)]
    fn op_push(&mut self, op: &Op) -> bool {
        let Op::Push(e) = op else { unreachable!() };
        {
            let start = self.pos;
            if self.op(e) {
                self.push_stack(StackItem::Span(start, self.pos));
                true
            } else {
                false
            }
        }
    }

    #[inline(never)]
    fn op_push_literal(&mut self, op: &Op) -> bool {
        let Op::PushLiteral(s) = op else { unreachable!() };
        {
            self.push_stack(StackItem::Literal(s.clone()));
            true
        }
    }

    #[inline(never)]
    fn op_tag(&mut self, op: &Op) -> bool {
        let Op::Tag(e, tag) = op else { unreachable!() };
        {
            if !self.op(e) {
                return false;
            }
            if self.look == 0 {
                if let Some(i) = self.last_end {
                    self.nodes[i as usize].tag = *tag;
                }
            }
            true
        }
    }

    #[inline(never)]
    fn op_restore_on_err(&mut self, op: &Op) -> bool {
        let Op::RestoreOnErr(e) = op else { unreachable!() };
        {
            let mark = self.journal.len();
            if self.fast.uses_stack {
                self.snapshots += 1;
            }
            let ok = self.op(e);
            if self.fast.uses_stack {
                if !ok {
                    self.undo_to(mark);
                }
                self.snapshots -= 1;
                if self.snapshots == 0 {
                    self.journal.clear();
                }
            }
            ok
        }
    }

    #[inline(never)]
    fn pattern(&mut self, units: &[Unit]) -> bool {
        let b = self.bytes;
        let mut p = self.pos;
        for u in units {
            match u {
                Unit::Lit(s) => {
                    if b.get(p..p + s.len()) != Some(&s[..]) {
                        {
                            self.reached(p);
                            return false;
                        }
                    }
                    p += s.len();
                }
                Unit::Class(set) => match b.get(p) {
                    Some(&x) if set.has(x) => p += 1,
                    _ => {
                        self.reached(p);
                        return false;
                    }
                },
                Unit::Opt(set) => {
                    if b.get(p).is_some_and(|&x| set.has(x)) {
                        p += 1;
                    }
                }
                Unit::Star(set, once) => {
                    let start = p;
                    while p < b.len() && set.has(b[p]) {
                        p += 1;
                    }
                    if *once && p == start {
                        {
                            self.reached(p);
                            return false;
                        }
                    }
                }
            }
        }
        self.advance(p);
        true
    }

    /// `repeat(sequence(skip ~ e))`.
    fn repeat(&mut self, e: &Op) {
        loop {
            let cp = self.checkpoint();
            let before = (self.pos, self.nodes.len());
            self.skip();
            if !self.op(e) || self.failed_hard {
                self.restore(cp);
                return;
            }
            self.commit(cp);
            // pest would loop for ever on a match of nothing; its validator refuses most such grammars.
            if (self.pos, self.nodes.len()) == before {
                return;
            }
        }
    }

    fn peek_slice(&mut self, start: i32, end: Option<i32>, top_down: bool) -> bool {
        let len = self.stack.len();
        let norm = |i: i32| -> Option<usize> {
            if i > len as i32 {
                None
            } else if i >= 0 {
                Some(i as usize)
            } else {
                let r = len as i32 + i;
                (r >= 0).then_some(r as usize)
            }
        };
        let Some(a) = norm(start) else { return false };
        let Some(b) = end.map_or(Some(len), norm) else {
            return false;
        };
        if b <= a {
            return true;
        }
        let mut p = self.pos;
        let items: Vec<StackItem> = if top_down {
            self.stack[a..b].iter().rev().cloned().collect()
        } else {
            self.stack[a..b].to_vec()
        };
        for item in &items {
            let s = self.item_bytes(item);
            if self.bytes.get(p..p + s.len()) != Some(s) {
                return false;
            }
            p += s.len();
        }
        self.advance(p);
        true
    }
}
