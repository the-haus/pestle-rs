// Derived from pest_vm (https://github.com/pest-parser/pest/tree/master/vm),
// Copyright (c) 2018 Dragoș Tiselice, licensed under the Apache License,
// Version 2.0 or the MIT license, at your option.
//
// Changes: rules are resolved to indices when the grammar is compiled
// (pest_vm looks every rule up by name, on every call, and panics on one it
// does not know); sequences and choices are flattened the way pest_derive
// generates them; built-in rules and Unicode properties are resolved once.

//! Runs a compiled grammar on pest's own [`ParserState`], so a grammar means
//! here exactly what it means to `pest_derive`.

use pest::{Atomicity, MatchDir, ParseResult, ParserState};

/// A rule of a compiled grammar: its index in [`crate::Grammar::names`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId(pub u32);

pub(crate) type State<'a> = Box<ParserState<'a, RuleId>>;

/// A rule's kind, as its definition's modifier says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// `a = { … }`
    Normal,
    /// `a = _{ … }`: matches, makes no pair.
    Silent,
    /// `a = @{ … }`: no implicit whitespace, no inner pairs.
    Atomic,
    /// `a = ${ … }`: no implicit whitespace, inner pairs kept.
    CompoundAtomic,
    /// `a = !{ … }`: implicit whitespace again inside an atomic rule.
    NonAtomic,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Normal => "normal",
            RuleKind::Silent => "silent",
            RuleKind::Atomic => "atomic",
            RuleKind::CompoundAtomic => "compound",
            RuleKind::NonAtomic => "nonAtomic",
        }
    }
}

/// The built-in rules pest defines.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Builtin {
    Any,
    Soi,
    Peek,
    PeekAll,
    Pop,
    PopAll,
    Drop,
    AsciiDigit,
    AsciiNonzeroDigit,
    AsciiBinDigit,
    AsciiOctDigit,
    AsciiHexDigit,
    AsciiAlphaLower,
    AsciiAlphaUpper,
    AsciiAlpha,
    AsciiAlphanumeric,
    Ascii,
    Newline,
}

impl Builtin {
    pub(crate) fn by_name(name: &str) -> Option<Builtin> {
        Some(match name {
            "ANY" => Builtin::Any,
            "SOI" => Builtin::Soi,
            "PEEK" => Builtin::Peek,
            "PEEK_ALL" => Builtin::PeekAll,
            "POP" => Builtin::Pop,
            "POP_ALL" => Builtin::PopAll,
            "DROP" => Builtin::Drop,
            "ASCII_DIGIT" => Builtin::AsciiDigit,
            "ASCII_NONZERO_DIGIT" => Builtin::AsciiNonzeroDigit,
            "ASCII_BIN_DIGIT" => Builtin::AsciiBinDigit,
            "ASCII_OCT_DIGIT" => Builtin::AsciiOctDigit,
            "ASCII_HEX_DIGIT" => Builtin::AsciiHexDigit,
            "ASCII_ALPHA_LOWER" => Builtin::AsciiAlphaLower,
            "ASCII_ALPHA_UPPER" => Builtin::AsciiAlphaUpper,
            "ASCII_ALPHA" => Builtin::AsciiAlpha,
            "ASCII_ALPHANUMERIC" => Builtin::AsciiAlphanumeric,
            "ASCII" => Builtin::Ascii,
            "NEWLINE" => Builtin::Newline,
            _ => return None,
        })
    }
}

/// A Unicode property test (`LETTER`, `XID_START`, …) from `pest::unicode`.
#[derive(Clone)]
pub(crate) struct Property(std::sync::Arc<dyn Fn(char) -> bool>);

// SAFETY: `pest::unicode::by_name` returns closures that capture only a
// `&'static` table entry; they hold no state and are safe to share.
unsafe impl Send for Property {}
unsafe impl Sync for Property {}

impl Property {
    pub(crate) fn by_name(name: &str) -> Option<Property> {
        pest::unicode::by_name(name).map(|f| Property(f.into()))
    }

    #[inline]
    pub(crate) fn test(&self, c: char) -> bool {
        (self.0)(c)
    }
}

/// An expression with every name resolved.
pub(crate) enum Expr {
    Str(String),
    Insens(String),
    Range(char, char),
    Rule(u32),
    Eoi,
    Builtin(Builtin),
    Unicode(Property),
    PeekSlice(i32, Option<i32>),
    PosPred(Box<Expr>),
    NegPred(Box<Expr>),
    Seq(Vec<Expr>),
    Choice(Vec<Expr>),
    Opt(Box<Expr>),
    Rep(Box<Expr>),
    RepOnce(Box<Expr>),
    Skip(Vec<String>),
    Push(Box<Expr>),
    PushLiteral(String),
    Tag(Box<Expr>, String),
    RestoreOnErr(Box<Expr>),
}

pub(crate) struct CompiledRule {
    pub(crate) kind: RuleKind,
    /// `WHITESPACE` or `COMMENT`: always atomic inside.
    pub(crate) implicit: bool,
    pub(crate) expr: Expr,
}

/// A grammar ready to run.
pub(crate) struct Program {
    pub(crate) rules: Vec<CompiledRule>,
    /// The id `EOI`'s pairs carry (one past the grammar's own rules).
    pub(crate) eoi: u32,
    pub(crate) whitespace: Option<u32>,
    pub(crate) comment: Option<u32>,
}

impl Program {
    pub(crate) fn rule<'a>(&'a self, id: u32, state: State<'a>) -> ParseResult<State<'a>> {
        let rule = &self.rules[id as usize];
        // pest's `rule` checks the stack and counts the call; a silent rule makes no pair, so no `rule`.
        let state = if rule.kind == RuleKind::Silent {
            state.check_stack_limit()?
        } else {
            state
        };
        let r = RuleId(id);
        let expr = &rule.expr;
        if rule.implicit {
            return match rule.kind {
                RuleKind::Normal | RuleKind::Atomic => {
                    state.rule(r, |s| s.atomic(Atomicity::Atomic, |s| self.expr(expr, s)))
                }
                RuleKind::Silent => state.atomic(Atomicity::Atomic, |s| self.expr(expr, s)),
                RuleKind::CompoundAtomic => {
                    state.atomic(Atomicity::CompoundAtomic, |s| s.rule(r, |s| self.expr(expr, s)))
                }
                RuleKind::NonAtomic => state.atomic(Atomicity::Atomic, |s| s.rule(r, |s| self.expr(expr, s))),
            };
        }
        match rule.kind {
            RuleKind::Normal => state.rule(r, |s| self.expr(expr, s)),
            RuleKind::Silent => self.expr(expr, state),
            RuleKind::Atomic => state.rule(r, |s| s.atomic(Atomicity::Atomic, |s| self.expr(expr, s))),
            RuleKind::CompoundAtomic => state.atomic(Atomicity::CompoundAtomic, |s| s.rule(r, |s| self.expr(expr, s))),
            RuleKind::NonAtomic => state.atomic(Atomicity::NonAtomic, |s| s.rule(r, |s| self.expr(expr, s))),
        }
    }

    // No stack check here: recursion only goes through rules, and every rule checks (pest's
    // `rule` again, as it counts calls); an expression nests no deeper than the grammar's text.
    fn expr<'a>(&'a self, expr: &'a Expr, state: State<'a>) -> ParseResult<State<'a>> {
        match expr {
            Expr::Str(s) => state.match_string(s),
            Expr::Insens(s) => state.match_insensitive(s),
            Expr::Range(a, b) => state.match_range(*a..*b),
            Expr::Rule(id) => self.rule(*id, state),
            Expr::Eoi => state.rule(RuleId(self.eoi), |s| s.end_of_input()),
            Expr::Builtin(b) => builtin(*b, state),
            Expr::Unicode(p) => state.match_char_by(|c| (p.0)(c)),
            Expr::PeekSlice(start, end) => state.stack_match_peek_slice(*start, *end, MatchDir::BottomToTop),
            Expr::PosPred(e) => state.lookahead(true, |s| self.expr(e, s)),
            Expr::NegPred(e) => state.lookahead(false, |s| self.expr(e, s)),
            Expr::Seq(items) => state.sequence(|mut s| {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        s = self.skip(s)?;
                    }
                    s = self.expr(item, s)?;
                }
                Ok(s)
            }),
            Expr::Choice(items) => {
                let mut s = state;
                for item in items {
                    match self.expr(item, s) {
                        Ok(s) => return Ok(s),
                        Err(back) => s = back,
                    }
                }
                Err(s)
            }
            Expr::Opt(e) => state.optional(|s| self.expr(e, s)),
            Expr::Rep(e) => state.sequence(|s| {
                s.optional(|s| {
                    self.expr(e, s)
                        .and_then(|s| s.repeat(|s| s.sequence(|s| self.skip(s).and_then(|s| self.expr(e, s)))))
                })
            }),
            Expr::RepOnce(e) => state.sequence(|s| {
                self.expr(e, s)
                    .and_then(|s| s.repeat(|s| s.sequence(|s| self.skip(s).and_then(|s| self.expr(e, s)))))
            }),
            Expr::Skip(strings) => {
                let strings: Vec<&str> = strings.iter().map(String::as_str).collect();
                state.skip_until(&strings)
            }
            Expr::Push(e) => state.stack_push(|s| self.expr(e, s)),
            Expr::PushLiteral(s) => state.stack_push_literal(s.clone()),
            Expr::Tag(e, tag) => self.expr(e, state).and_then(|s| s.tag_node(tag)),
            Expr::RestoreOnErr(e) => state.restore_on_err(|s| self.expr(e, s)),
        }
    }

    /// Implicit whitespace and comments between the parts of a non-atomic sequence.
    fn skip<'a>(&'a self, state: State<'a>) -> ParseResult<State<'a>> {
        if state.atomicity() != Atomicity::NonAtomic {
            return Ok(state);
        }
        match (self.whitespace, self.comment) {
            (None, None) => Ok(state),
            (Some(ws), None) => state.repeat(|s| self.rule(ws, s)),
            (None, Some(c)) => state.repeat(|s| self.rule(c, s)),
            (Some(ws), Some(c)) => state.sequence(|s| {
                s.repeat(|s| self.rule(ws, s)).and_then(|s| {
                    s.repeat(|s| s.sequence(|s| self.rule(c, s).and_then(|s| s.repeat(|s| self.rule(ws, s)))))
                })
            }),
        }
    }
}

// pest's `match_range` includes both ends: `'0'..'9'` is every digit.
#[allow(clippy::almost_complete_range)]
fn builtin(b: Builtin, state: State<'_>) -> ParseResult<State<'_>> {
    match b {
        Builtin::Any => state.skip(1),
        Builtin::Soi => state.start_of_input(),
        Builtin::Peek => state.stack_peek(),
        Builtin::PeekAll => state.stack_match_peek(),
        Builtin::Pop => state.stack_pop(),
        Builtin::PopAll => state.stack_match_pop(),
        Builtin::Drop => state.stack_drop(),
        Builtin::AsciiDigit => state.match_range('0'..'9'),
        Builtin::AsciiNonzeroDigit => state.match_range('1'..'9'),
        Builtin::AsciiBinDigit => state.match_range('0'..'1'),
        Builtin::AsciiOctDigit => state.match_range('0'..'7'),
        Builtin::AsciiHexDigit => state
            .match_range('0'..'9')
            .or_else(|s| s.match_range('a'..'f'))
            .or_else(|s| s.match_range('A'..'F')),
        Builtin::AsciiAlphaLower => state.match_range('a'..'z'),
        Builtin::AsciiAlphaUpper => state.match_range('A'..'Z'),
        Builtin::AsciiAlpha => state.match_range('a'..'z').or_else(|s| s.match_range('A'..'Z')),
        Builtin::AsciiAlphanumeric => state
            .match_range('a'..'z')
            .or_else(|s| s.match_range('A'..'Z'))
            .or_else(|s| s.match_range('0'..'9')),
        Builtin::Ascii => state.match_range('\x00'..'\x7f'),
        Builtin::Newline => state
            .match_string("\n")
            .or_else(|s| s.match_string("\r\n"))
            .or_else(|s| s.match_string("\r")),
    }
}
