//! A page's template as a set of element paths from `<body>`
//! (`main.event-detail>div.tickets>span.price`): tag, id and sorted
//! classes per step, digits folded (`post-1234` → `post-#`), no text, no
//! other attributes; elements that never show content (scripts, forms,
//! media, `<head>`) left out with all inside them. Twelve cards or thirty
//! give the same set, so its hash changes with the template, not the
//! content (festimap's `skeletonPaths` / `skeletonHash`).

use std::collections::{BTreeSet, HashSet};
use std::hash::{DefaultHasher, Hasher};

use smallvec::SmallVec;

use sha2::{Digest, Sha256};

use crate::tree::{Handler, Text, Tree};

/// Options of the skeleton pass.
#[derive(Clone, Debug)]
pub struct SkeletonOptions {
    /// How deep below `<body>` paths go (festimap: 14).
    pub depth: usize,
    /// Elements left out with their content.
    pub ignore: Vec<String>,
    /// Also return the paths, not only their hash.
    pub paths: bool,
}

/// What festimap leaves out of the skeleton and the learn prompt.
pub const DEFAULT_IGNORE: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "iframe", "object", "canvas", "link", "base", "input", "select",
    "option", "textarea", "button", "form", "video", "audio", "source", "picture", "map", "head",
];

impl Default for SkeletonOptions {
    fn default() -> Self {
        SkeletonOptions {
            depth: 14,
            ignore: DEFAULT_IGNORE.iter().map(|s| s.to_string()).collect(),
            paths: false,
        }
    }
}

/// A page's skeleton.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skeleton {
    /// sha256 (hex) of the paths, sorted, one a line.
    pub hash: String,
    pub paths: Option<Vec<String>>,
}

/// The paths of open elements, from the document and from the first
/// `<body>`, kept as 64-bit hashes chained parent to child: a path's text
/// is built only the first time it is seen (a page repeats a few hundred
/// paths over thousands of elements).
pub(crate) struct SkeletonPass {
    options: SkeletonOptions,
    /// Per frame: its path's hash from the document's top, and from the body (None: left out).
    from_top: Vec<Option<u64>>,
    from_body: Vec<Option<u64>>,
    body: Option<usize>,
    body_done: bool,
    seen_top: HashSet<u64>,
    seen_body: HashSet<u64>,
    top_paths: BTreeSet<String>,
    body_paths: BTreeSet<String>,
}

fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut digits = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            out.push(c);
            digits = false;
        }
    }
    out
}

fn signature(name: &str, id: Option<&str>, class: Option<&str>) -> String {
    let mut classes: Vec<String> = class.unwrap_or("").split_ascii_whitespace().map(fold).collect();
    classes.sort();
    classes.dedup();
    let mut s = name.to_owned();
    if let Some(id) = id.filter(|i| !i.is_empty()) {
        s.push('#');
        s.push_str(&fold(id));
    }
    if !classes.is_empty() {
        s.push('.');
        s.push_str(&classes.join("."));
    }
    s
}

/// A string's hash with digit runs folded to `#`, as [`fold`] writes it.
fn fold_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    let mut digits = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            if !digits {
                h.write_u8(b'#');
            }
            digits = true;
        } else {
            h.write_u8(b);
            digits = false;
        }
    }
    h.write_u8(0xff);
    h.finish()
}

/// The hash of [`signature`]'s text, without building it.
fn signature_hash(name: &str, id: Option<&str>, class: Option<&str>) -> u64 {
    let mut classes: SmallVec<[u64; 8]> = class.unwrap_or("").split_ascii_whitespace().map(fold_hash).collect();
    classes.sort_unstable();
    classes.dedup();
    let mut h = DefaultHasher::new();
    h.write(name.as_bytes());
    h.write_u8(0xff);
    if let Some(id) = id.filter(|i| !i.is_empty()) {
        h.write_u64(fold_hash(id));
    }
    h.write_u8(0xfe);
    for c in classes {
        h.write_u64(c);
    }
    h.finish()
}

fn chain(parent: u64, sig: u64) -> u64 {
    // splitmix64 of the pair.
    let mut z = parent.rotate_left(29) ^ sig.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const ROOT: u64 = 0x5EED_5EED_5EED_5EED;

impl SkeletonPass {
    pub(crate) fn new(options: SkeletonOptions) -> SkeletonPass {
        SkeletonPass {
            options,
            from_top: Vec::new(),
            from_body: Vec::new(),
            body: None,
            body_done: false,
            seen_top: HashSet::new(),
            seen_body: HashSet::new(),
            top_paths: BTreeSet::new(),
            body_paths: BTreeSet::new(),
        }
    }

    pub(crate) fn finish(self) -> Skeleton {
        let paths = if self.body.is_some() || self.body_done {
            self.body_paths
        } else {
            self.top_paths
        };
        let joined = paths.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
        let hash = Sha256::digest(joined.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Skeleton {
            hash,
            paths: self.options.paths.then(|| paths.into_iter().collect()),
        }
    }

    /// The text of the path of the element at `frame`, from frame `from` down.
    fn text(tree: &Tree, from: usize, frame: usize) -> String {
        let mut out = String::new();
        for f in &tree.frames[from..=frame] {
            if !out.is_empty() {
                out.push('>');
            }
            out.push_str(&signature(f.el.name(), f.el.attr("id"), f.el.attr("class")));
        }
        out
    }
}

/// The skeleton of a template several pages share: the paths at least half
/// of them have, hashed (festimap's `templateHash`).
pub fn template_hash(pages: &[Vec<String>]) -> Option<String> {
    if pages.is_empty() {
        return None;
    }
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for page in pages {
        let unique: BTreeSet<&str> = page.iter().map(String::as_str).collect();
        for p in unique {
            *counts.entry(p).or_default() += 1;
        }
    }
    let quorum = pages.len().div_ceil(2);
    let shared: Vec<&str> = counts
        .into_iter()
        .filter(|&(_, n)| n >= quorum)
        .map(|(p, _)| p)
        .collect();
    Some(
        Sha256::digest(shared.join("\n").as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

impl Handler for SkeletonPass {
    fn open(&mut self, tree: &Tree) {
        let d = tree.frames.len() - 1;
        let el = &tree.frames[d].el;
        let ignored = self.options.ignore.iter().any(|n| n == el.name());
        let parent_top = match d.checked_sub(1) {
            None => Some(ROOT),
            Some(p) => self.from_top[p],
        };
        let wanted = |depth: usize, parent: Option<u64>| !ignored && depth <= self.options.depth && parent.is_some();
        let body_parent = match self.body {
            Some(b) if d > b => Some(if d - 1 == b { Some(ROOT) } else { self.from_body[d - 1] }),
            _ => None,
        };
        // Once there is a body, paths from the top are never used.
        let need_top = self.body.is_none() && !self.body_done && wanted(d, parent_top);
        let need_body = body_parent.is_some_and(|p| wanted(d - self.body.unwrap_or(0) - 1, p));
        let sig = if need_top || need_body {
            signature_hash(el.name(), el.attr("id"), el.attr("class"))
        } else {
            0
        };
        let top = need_top.then(|| chain(parent_top.unwrap_or(ROOT), sig));
        if let Some(h) = top {
            if self.seen_top.insert(h) {
                self.top_paths.insert(Self::text(tree, 0, d));
            }
        }
        self.from_top.push(top);
        let body = if need_body {
            body_parent.flatten().map(|p| chain(p, sig))
        } else {
            None
        };
        if let (Some(h), Some(b)) = (body, self.body) {
            if self.seen_body.insert(h) {
                self.body_paths.insert(Self::text(tree, b + 1, d));
            }
        }
        self.from_body.push(body);
        if self.body.is_none() && !self.body_done && el.kind == crate::tags::BODY {
            self.body = Some(d);
            self.top_paths.clear();
            self.seen_top.clear();
        }
    }

    fn close(&mut self, tree: &Tree) {
        let d = tree.frames.len() - 1;
        if self.body == Some(d) {
            self.body = None;
            self.body_done = true;
        }
        self.from_top.pop();
        self.from_body.pop();
    }

    fn text(&mut self, _: &Tree, _: &Text<'_>) {}
    fn markup(&mut self, _: &Tree, _: &[u8]) {}
}
