//! A parse's pairs, flat: one [`Node`] per pair in document order (a pair
//! before the pairs inside it), each knowing how many pairs it holds, so a
//! node's children are found without pointers and the whole tree is one
//! allocation, cheap to hand across a language boundary.

use std::collections::HashMap;

use pest::iterators::{Pair, Pairs};

use crate::RuleId;

/// [`Node::tag`] of an untagged pair.
pub const NO_TAG: u32 = u32::MAX;

/// One pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    /// Its rule ([`crate::Grammar::names`]).
    pub rule: u32,
    /// Its node tag ([`crate::Grammar::tags`]), or [`NO_TAG`].
    pub tag: u32,
    /// Its span in the input, in bytes.
    pub start: u32,
    pub end: u32,
    /// How many pairs it holds, at any depth: its next sibling is at `index + 1 + size`.
    pub size: u32,
}

/// The pairs of one parse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// How far the parse read, in bytes (a rule need not read the whole input).
    pub end: usize,
}

impl Tree {
    pub(crate) fn from_pairs(pairs: Pairs<'_, RuleId>, end: usize, tags: &HashMap<String, u32>) -> Tree {
        let mut nodes = Vec::new();
        for pair in pairs {
            push(pair, &mut nodes, tags);
        }
        Tree { nodes, end }
    }

    /// The offsets moved by `by` bytes (a match found past the input's start).
    pub(crate) fn shift(&mut self, by: usize) {
        let by32 = by as u32;
        for n in &mut self.nodes {
            n.start += by32;
            n.end += by32;
        }
        self.end += by;
    }

    /// The top-level pairs' indices.
    pub fn roots(&self) -> Children<'_> {
        Children {
            tree: self,
            at: 0,
            until: self.nodes.len(),
        }
    }

    /// A node's children's indices.
    pub fn children(&self, index: usize) -> Children<'_> {
        let n = &self.nodes[index];
        Children {
            tree: self,
            at: index + 1,
            until: index + 1 + n.size as usize,
        }
    }

    /// A node's text.
    pub fn text<'i>(&self, index: usize, input: &'i str) -> &'i str {
        let n = &self.nodes[index];
        &input[n.start as usize..n.end as usize]
    }

    /// Every start and end as a UTF-16 offset (how JavaScript counts), in node order.
    pub fn utf16_spans(&self, input: &str) -> Vec<(u32, u32)> {
        if input.is_ascii() {
            return self.nodes.iter().map(|n| (n.start, n.end)).collect();
        }
        let mut offsets: Vec<u32> = self.nodes.iter().flat_map(|n| [n.start, n.end]).collect();
        offsets.push(self.end as u32);
        offsets.sort_unstable();
        offsets.dedup();
        let mut utf16 = Vec::with_capacity(offsets.len());
        let (mut byte, mut unit) = (0usize, 0u32);
        for &target in &offsets {
            for c in input[byte..target as usize].chars() {
                unit += c.len_utf16() as u32;
            }
            byte = target as usize;
            utf16.push(unit);
        }
        let at = |b: u32| utf16[offsets.binary_search(&b).expect("offset collected")];
        self.nodes.iter().map(|n| (at(n.start), at(n.end))).collect()
    }

    /// [`Tree::end`] as a UTF-16 offset.
    pub fn utf16_end(&self, input: &str) -> u32 {
        utf16_len(&input[..self.end])
    }
}

/// A string's length in UTF-16 units.
pub fn utf16_len(s: &str) -> u32 {
    if s.is_ascii() {
        s.len() as u32
    } else {
        s.chars().map(|c| c.len_utf16() as u32).sum()
    }
}

fn push(pair: Pair<'_, RuleId>, nodes: &mut Vec<Node>, tags: &HashMap<String, u32>) {
    let at = nodes.len();
    let span = pair.as_span();
    let tag = pair.as_node_tag().and_then(|t| tags.get(t).copied()).unwrap_or(NO_TAG);
    nodes.push(Node {
        rule: pair.as_rule().0,
        tag,
        start: span.start() as u32,
        end: span.end() as u32,
        size: 0,
    });
    for inner in pair.into_inner() {
        push(inner, nodes, tags);
    }
    nodes[at].size = (nodes.len() - at - 1) as u32;
}

/// Sibling indices.
pub struct Children<'t> {
    tree: &'t Tree,
    at: usize,
    until: usize,
}

impl Iterator for Children<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.at >= self.until {
            return None;
        }
        let i = self.at;
        self.at += 1 + self.tree.nodes[i].size as usize;
        Some(i)
    }
}
