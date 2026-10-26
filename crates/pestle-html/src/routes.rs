//! A site's routes, inferred from its URLs (a sitemap's, or the links of its
//! pages): a tree of patterns as Next.js writes them, `/agenda/[slug]`,
//! `/news/[year]/[month]/[slug]`, `/[locale]/products/[category]/[slug]`,
//! with how many URLs each holds, and which ones hold the site's items: one
//! event, one product, one article a page.
//!
//! Siblings become a parameter when they are values, not sections: years,
//! dates, numbers, UUIDs and hash ids from two of them; slugs of three words
//! or more (`nora-live-in-paradiso`) from two; shorter ones from
//! [`RouteOptions::min_siblings`]; single words from four times that, or
//! from two under a section that names what it holds (`events`, `agenda`,
//! `producten`). Words a site keeps for its structure (`page`, `tag`,
//! `archive`, `contact`) stay as they are. Counts are of the URLs given, so
//! each URL counts once, at the route [`Routes::find`] gives it.
//!
//! ```
//! use pestle_html::routes::{RouteBuilder, RouteKind};
//! let mut b = RouteBuilder::new(Default::default());
//! for slug in ["nora-live", "jazz-night", "open-air-2026"] {
//!     b.add(&format!("https://venue.example/agenda/{slug}"));
//! }
//! b.add("https://venue.example/contact");
//! let routes = b.finish();
//! let item = routes.find("https://venue.example/agenda/new-show").unwrap();
//! assert_eq!(item.pattern, "/agenda/[slug]");
//! assert_eq!(item.kind, RouteKind::Item);
//! assert_eq!(item.pages, 3);
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Serialize;
use url::Url;

use crate::sitemap::{Entry, Kind as EntryKind};

/// How routes are inferred.
#[derive(Clone, Copy, Debug)]
pub struct RouteOptions {
    /// Siblings that are short slugs (`jazz-night`) become a parameter from this many; single words
    /// from four times as many (or from two under a section such as `events`).
    pub min_siblings: usize,
    /// Example URLs kept for each route.
    pub max_examples: usize,
    /// At most this many URLs are read; the rest are counted in a warning.
    pub max_urls: usize,
    /// Path segments past this many are left out.
    pub max_depth: usize,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            min_siblings: 5,
            max_examples: 3,
            max_urls: 1_000_000,
            max_depth: 32,
        }
    }
}

/// What a parameter's values are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    /// Text: `nora-live`, `rock`.
    Slug,
    /// Digits: `123`.
    Number,
    /// `2026`.
    Year,
    /// `01`–`12` under a year.
    Month,
    /// `01`–`31` under a month.
    Day,
    /// `2026-11-01`, `20261101`, `2026-11`.
    Date,
    /// `0e8f…-…`.
    Uuid,
    /// An opaque id: `a1b2c3d4e5`, `xK9fQ2mZ`.
    Hash,
    /// `en`, `nl-be`, first in the path.
    Locale,
}

/// A route's dynamic segment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Param {
    /// `slug`, `id`, `page`, `year`, `category`, … (unique along a route, as Next.js wants).
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ParamType,
}

/// What a route's pages most likely are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteKind {
    /// The site's `/`.
    Root,
    /// One page: `/contact`.
    Page,
    /// Pages that group others, without items of their own.
    Section,
    /// A section whose child is a route of items: `/agenda` above `/agenda/[slug]`.
    Listing,
    /// One item a page: an event, a product, an article.
    Item,
    /// A page per category, tag, genre or author: `/tag/[slug]`.
    Taxonomy,
    /// A page per year, month or day: `/news/[year]/[month]`.
    Archive,
    /// `/agenda/page/[page]`.
    Pagination,
}

/// One route of a site.
#[derive(Clone, Debug, Serialize)]
pub struct Route {
    /// As written in the path (`agenda`), or the parameter in brackets (`[slug]`).
    pub segment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<Param>,
    /// From the site's root: `/agenda/[slug]`.
    pub pattern: String,
    /// URLs that are this route.
    pub pages: usize,
    /// URLs at this route or below it.
    pub total: usize,
    pub kind: RouteKind,
    /// What its pages are about, from the words of its path: `event`, `product`, `article`,
    /// `person`, `venue`, `job`, `recipe`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<&'static str>,
    /// For an item route, how sure it is that each page is one item, 0–1; 0 for other routes.
    pub score: f64,
    /// The first URLs given that are this route.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// The latest `<lastmod>` of its pages (sitemap entries only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastmod: Option<String>,
    /// How many of its pages have images (sitemap entries only).
    #[serde(skip_serializing_if = "is_zero")]
    pub images: usize,
    /// Most URLs first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Route>,
    /// Pages whose last segment is a value (a long slug, a number, a date, an id).
    #[serde(skip)]
    valued: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The routes of one site (a host, `www.` or not).
#[derive(Clone, Debug, Serialize)]
pub struct Site {
    /// `https://venue.example`, as its first URL has it.
    pub origin: String,
    /// The URLs read for it.
    pub urls: usize,
    pub root: Route,
}

/// Every site's routes.
#[derive(Clone, Debug, Serialize)]
pub struct Routes {
    /// Most URLs first.
    pub sites: Vec<Site>,
    /// What was left out (`3 URLs that are not http(s)`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

struct Input {
    site: usize,
    url: String,
    lastmod: Option<String>,
    images: bool,
}

/// URLs read one by one, routes inferred at the end.
pub struct RouteBuilder {
    opts: RouteOptions,
    /// Origin of each site, and the index of each host key.
    origins: Vec<String>,
    hosts: HashMap<String, usize>,
    inputs: Vec<Input>,
    skipped: usize,
    over: usize,
}

impl RouteBuilder {
    pub fn new(opts: RouteOptions) -> RouteBuilder {
        RouteBuilder {
            opts,
            origins: Vec::new(),
            hosts: HashMap::new(),
            inputs: Vec::new(),
            skipped: 0,
            over: 0,
        }
    }

    /// An absolute http(s) URL; anything else is counted in a warning.
    pub fn add(&mut self, url: &str) {
        self.push(url, None, false);
    }

    /// A sitemap entry: its `<lastmod>` and images go into its route's figures. Entries that name
    /// another sitemap are not pages, and are left out.
    pub fn add_entry(&mut self, entry: &Entry) {
        if entry.kind == EntryKind::Url {
            self.push(&entry.url, entry.lastmod.clone(), !entry.images.is_empty());
        }
    }

    /// [`RouteBuilder::add`] with what a sitemap says of the page.
    pub fn add_with(&mut self, url: &str, lastmod: Option<String>, images: bool) {
        self.push(url, lastmod, images);
    }

    fn push(&mut self, url: &str, lastmod: Option<String>, images: bool) {
        if self.inputs.len() >= self.opts.max_urls {
            self.over += 1;
            return;
        }
        let Ok(mut u) = Url::parse(url.trim()) else {
            self.skipped += 1;
            return;
        };
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
            self.skipped += 1;
            return;
        }
        u.set_fragment(None);
        let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
        let key = match u.port() {
            Some(p) => format!("{}:{p}", host.strip_prefix("www.").unwrap_or(&host)),
            None => host.strip_prefix("www.").unwrap_or(&host).to_owned(),
        };
        let site = match self.hosts.get(&key) {
            Some(&s) => s,
            None => {
                let s = self.origins.len();
                self.origins.push(u.origin().ascii_serialization());
                self.hosts.insert(key, s);
                s
            }
        };
        self.inputs.push(Input {
            site,
            url: u.into(),
            lastmod,
            images,
        });
    }

    /// The routes of every site the URLs were on.
    pub fn finish(self) -> Routes {
        let opts = self.opts;
        let mut tries: Vec<Node> = (0..self.origins.len()).map(|_| Node::default()).collect();
        for input in &self.inputs {
            let segs = segments(&input.url, opts.max_depth);
            tries[input.site].insert(&segs);
        }
        let mut routes: Vec<Route> = tries
            .into_iter()
            .map(|mut trie| {
                collapse(&mut trie, &Context::root(), &opts);
                to_route(&trie, String::new(), None, String::new())
            })
            .collect();
        let mut unmatched = 0;
        for input in self.inputs {
            let segs = segments(&input.url, opts.max_depth);
            if !tally(&mut routes[input.site], &segs, &input, &opts) {
                unmatched += 1;
            }
        }
        let mut sites: Vec<Site> = routes
            .into_iter()
            .zip(self.origins)
            .map(|(mut root, origin)| {
                prune(&mut root);
                label(&mut root, None, None, true);
                Site {
                    origin,
                    urls: root.total,
                    root,
                }
            })
            .collect();
        sites.sort_by(|a, b| b.urls.cmp(&a.urls).then_with(|| a.origin.cmp(&b.origin)));
        let mut warnings = Vec::new();
        if self.skipped > 0 {
            warnings.push(format!(
                "routes: {} URLs that are not absolute http(s) were left out",
                self.skipped
            ));
        }
        if self.over > 0 {
            warnings.push(format!("routes: {} URLs past max_urls were left out", self.over));
        }
        if unmatched > 0 {
            warnings.push(format!("routes: {unmatched} URLs matched no route"));
        }
        Routes { sites, warnings }
    }
}

/// Routes inferred from `urls` at once.
pub fn infer<'a>(urls: impl IntoIterator<Item = &'a str>, opts: RouteOptions) -> Routes {
    let mut b = RouteBuilder::new(opts);
    for u in urls {
        b.add(u);
    }
    b.finish()
}

impl Routes {
    /// The route `url` is (absolute http(s)), when its site and path have one: static segments
    /// first, then a parameter whose values it fits.
    pub fn find(&self, url: &str) -> Option<&Route> {
        self.locate(url).map(|(_, r)| r)
    }

    /// [`Routes::find`], with the site the route is of.
    pub fn locate(&self, url: &str) -> Option<(&Site, &Route)> {
        let u = Url::parse(url.trim()).ok()?;
        let host = u.host_str()?.to_ascii_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host);
        let site = self.sites.iter().find(|s| {
            Url::parse(&s.origin).ok().is_some_and(|o| {
                let h = o.host_str().unwrap_or_default();
                h.strip_prefix("www.").unwrap_or(h) == host && o.port() == u.port()
            })
        })?;
        let segs = segments(u.as_str(), usize::MAX);
        find_in(&site.root, &segs).map(|r| (site, r))
    }

    /// Every route of items, the likeliest first: where a site's events, products or articles are.
    pub fn items(&self) -> Vec<&Route> {
        fn walk<'a>(r: &'a Route, out: &mut Vec<&'a Route>) {
            if r.kind == RouteKind::Item {
                out.push(r);
            }
            for c in &r.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        for s in &self.sites {
            walk(&s.root, &mut out);
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| b.pages.cmp(&a.pages)));
        out
    }

    /// The tree as text, a line a route, with its pages, the URLs at it or below, its kind,
    /// topic and score: for a person or a model to read. A static segment with no pages of its
    /// own and one child is joined to it (`page/[page]`).
    pub fn outline(&self) -> String {
        let mut out = String::new();
        for (k, s) in self.sites.iter().enumerate() {
            if k > 0 {
                out.push('\n');
            }
            let mut lines = Vec::new();
            line(&s.root, "/".to_owned(), String::new(), &mut lines);
            outline_children(&s.root, "", &mut lines);
            let width = lines.iter().map(|l| l.0.chars().count()).max().unwrap_or(0).max(10);
            let _ = writeln!(out, "{} ({} URLs)", s.origin, thousands(s.urls));
            let _ = writeln!(out, "{:width$}  {:>7}  {:>7}", "", "pages", "in all");
            for (label, pages, total, notes) in lines {
                let pad = width - label.chars().count();
                let _ = writeln!(
                    out,
                    "{label}{:pad$}  {:>7}  {:>7}  {notes}",
                    "",
                    thousands(pages),
                    thousands(total)
                );
            }
        }
        out
    }
}

const MAX_OUTLINE_CHILDREN: usize = 25;
const MAX_LABEL: usize = 48;

type Line = (String, usize, usize, String);

fn outline_children(r: &Route, prefix: &str, lines: &mut Vec<Line>) {
    let shown = r.children.len().min(MAX_OUTLINE_CHILDREN);
    let rest = &r.children[shown..];
    for (k, c) in r.children[..shown].iter().enumerate() {
        let last = k + 1 == shown && rest.is_empty();
        let (branch, next) = if last {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };
        // `page` with no pages and one child: `page/[page]`.
        let mut node = c;
        let mut name = node.segment.clone();
        while node.pages == 0 && node.children.len() == 1 && node.param.is_none() {
            node = &node.children[0];
            name = format!("{name}/{}", node.segment);
        }
        if name.chars().count() > MAX_LABEL {
            name = format!("{}…", name.chars().take(MAX_LABEL - 1).collect::<String>());
        }
        line(node, name, format!("{prefix}{branch}"), lines);
        outline_children(node, &format!("{prefix}{next}"), lines);
    }
    if !rest.is_empty() {
        let total: usize = rest.iter().map(|c| c.total).sum();
        lines.push((
            format!("{prefix}└── … {} more", rest.len()),
            rest.iter().map(|c| c.pages).sum(),
            total,
            String::new(),
        ));
    }
}

fn line(r: &Route, name: String, prefix: String, lines: &mut Vec<Line>) {
    let mut notes = format!("{:?}", r.kind).to_lowercase();
    if let Some(t) = r.topic {
        let _ = write!(notes, " · {t}");
    }
    if r.kind == RouteKind::Item {
        let _ = write!(notes, " · {:.2}", r.score);
    }
    if r.param.is_some() {
        if let Some(e) = r.examples.first() {
            let path = Url::parse(e).map_or_else(|_| e.clone(), |u| u.path().to_owned());
            let _ = write!(notes, "  e.g. {path}");
        }
    }
    lines.push((format!("{prefix}{name}"), r.pages, r.total, notes));
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ── Inference ────────────────────────────────────────────────────────────

/// A trie of path segments; after [`collapse`], its parameters too.
#[derive(Default)]
struct Node {
    statics: Statics,
    params: Vec<(Param, Node)>,
    pages: usize,
    total: usize,
    /// How many nodes were merged into this one: under a parameter, a name from several of its
    /// values is a page each value has (`/artists/[slug]/bio`), one from a single value a value.
    sources: usize,
}

impl Node {
    fn insert(&mut self, segs: &[&str]) {
        self.total += 1;
        self.sources = 1;
        match segs.split_first() {
            None => self.pages += 1,
            Some((first, rest)) => self.statics.slot(first).insert(rest),
        }
    }

    fn merge(&mut self, mut other: Node) {
        self.pages += other.pages;
        self.total += other.total;
        self.sources += other.sources;
        for (k, v) in other.statics.take() {
            self.statics.add(k, v);
        }
        self.params.extend(other.params);
    }

    fn is_leaf(&self) -> bool {
        self.statics.list.is_empty() && self.params.is_empty()
    }
}

/// A node's static children: a list, and an index once there are more than a few. Most nodes have
/// none or one, and a map for each would cost far more than the URLs.
#[derive(Default)]
struct Statics {
    list: Vec<(Box<str>, Node)>,
    index: HashMap<Box<str>, usize>,
}

const INDEXED: usize = 8;

impl Statics {
    fn position(&self, key: &str) -> Option<usize> {
        if self.index.is_empty() {
            self.list.iter().position(|(k, _)| &**k == key)
        } else {
            self.index.get(key).copied()
        }
    }

    fn push(&mut self, key: Box<str>, node: Node) -> usize {
        if self.list.is_empty() {
            self.list.reserve_exact(1);
        }
        let k = self.list.len();
        if !self.index.is_empty() {
            self.index.insert(key.clone(), k);
        }
        self.list.push((key, node));
        if self.index.is_empty() && self.list.len() > INDEXED {
            self.index = self.list.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect();
        }
        k
    }

    fn slot(&mut self, key: &str) -> &mut Node {
        let k = match self.position(key) {
            Some(k) => k,
            None => self.push(key.into(), Node::default()),
        };
        &mut self.list[k].1
    }

    fn add(&mut self, key: Box<str>, node: Node) {
        match self.position(&key) {
            Some(k) => self.list[k].1.merge(node),
            None => {
                self.push(key, node);
            }
        }
    }

    fn take(&mut self) -> Vec<(Box<str>, Node)> {
        self.index = HashMap::new();
        std::mem::take(&mut self.list)
    }

    /// In name order, so what is inferred does not depend on the order URLs came in.
    fn sort(&mut self) {
        self.list.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        if !self.index.is_empty() {
            self.index = self.list.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect();
        }
    }
}

/// Where a node is: how deep, under which static segment or parameter, and the parameter names
/// above it.
struct Context<'a> {
    depth: usize,
    parent_static: Option<&'a str>,
    parent_param: Option<ParamType>,
    names: Vec<String>,
}

impl Context<'_> {
    fn root() -> Context<'static> {
        Context {
            depth: 0,
            parent_static: None,
            parent_param: None,
            names: Vec::new(),
        }
    }
}

fn collapse(node: &mut Node, cx: &Context<'_>, opts: &RouteOptions) {
    let min = opts.min_siblings.max(2);
    // At the top of a site, or of a language version of it, names are its sections and pages.
    let top = cx.depth == 0 || cx.parent_param == Some(ParamType::Locale);
    // Under a section that says what it holds (`events`, `tag`), or under a value, names are values.
    let topical = cx
        .parent_static
        .is_some_and(|s| topic_of(s).is_some() || is_taxonomy_word(s))
        || (cx.parent_param.is_some() && !top);
    let paging = cx.parent_static.is_some_and(is_page_word);
    node.statics.sort();
    let list = &node.statics.list;
    let mut typed: Vec<(ParamType, Vec<usize>)> = Vec::new();
    let (mut strong, mut weak, mut words) = (Vec::new(), Vec::new(), Vec::new());
    // Under a parameter, a few names that come from several of its values are pages each value
    // has (`/artists/[slug]/bio`); many are values shared, as translations share their slugs.
    let repeated: Vec<usize> = match cx.parent_param {
        Some(t) if t != ParamType::Locale => (0..list.len()).filter(|&i| list[i].1.sources >= 2).collect(),
        _ => Vec::new(),
    };
    let repeated = if repeated.len() < min { repeated } else { Vec::new() };
    for (i, (name, _)) in list.iter().enumerate() {
        if is_structural(name) {
            continue;
        }
        let t = match shape(name, cx.depth) {
            Shape::Year => Some(ParamType::Year),
            Shape::Number(n) => Some(match cx.parent_param {
                Some(ParamType::Year) if (1..=12).contains(&n) => ParamType::Month,
                Some(ParamType::Month) if (1..=31).contains(&n) => ParamType::Day,
                _ => ParamType::Number,
            }),
            Shape::Date => Some(ParamType::Date),
            Shape::Uuid => Some(ParamType::Uuid),
            Shape::Hash => Some(ParamType::Hash),
            Shape::Locale => Some(ParamType::Locale),
            _ if repeated.contains(&i) => None,
            Shape::Slug => {
                strong.push(i);
                None
            }
            Shape::ShortSlug => {
                weak.push(i);
                None
            }
            Shape::Word => {
                words.push(i);
                None
            }
        };
        if let Some(t) = t {
            match typed.iter_mut().find(|(k, _)| *k == t) {
                Some((_, v)) => v.push(i),
                None => typed.push((t, vec![i])),
            }
        }
    }
    // Years, months and days among other numbers are numbers.
    if let Some(k) = typed.iter().position(|(t, _)| *t == ParamType::Number) {
        let mut folded = std::mem::take(&mut typed[k].1);
        typed.retain_mut(|(t, v)| {
            let fold = matches!(t, ParamType::Year | ParamType::Month | ParamType::Day);
            if fold {
                folded.append(v);
            }
            !fold && *t != ParamType::Number
        });
        typed.push((ParamType::Number, folded));
    }
    let mut groups: Vec<(ParamType, Vec<usize>)> = typed
        .into_iter()
        .filter(|(t, v)| {
            v.len() >= 2 || matches!(t, ParamType::Month | ParamType::Day) || (*t == ParamType::Number && paging)
        })
        .collect();
    // Text: long slugs from two, short ones from `min`, single words from `4 × min` (never at the
    // root, where a site keeps its pages, unless there are very many) — or anything from two
    // under a section that says what it holds.
    let mut text: Vec<usize> = Vec::new();
    let untyped = strong.len() + weak.len() + words.len();
    if topical && untyped >= 2 {
        text.extend(strong);
        text.extend(weak);
        text.extend(words);
    } else {
        let short = strong.len() + weak.len() >= min;
        let many = if top { untyped >= 25 * min } else { untyped >= 4 * min };
        if strong.len() >= if top { min } else { 2 } || short || many {
            text.extend(strong);
        }
        if short || many {
            text.extend(weak);
        }
        if many {
            text.extend(words);
        }
    }
    if text.len() == 1 && !topical {
        text.clear();
    }
    // Among values that are mostly single pages, one with pages of its own below stays a section.
    if text.len() >= 2 {
        let leaves = text.iter().filter(|&&i| list[i].1.is_leaf()).count();
        if leaves * 4 >= text.len() * 3 {
            text.retain(|&i| list[i].1.is_leaf() || list[i].1.total < min);
        }
    }
    if !text.is_empty() {
        groups.push((ParamType::Slug, text));
    }
    let mut assign = vec![usize::MAX; list.len()];
    for (g, (_, members)) in groups.iter().enumerate() {
        for &i in members {
            assign[i] = g;
        }
    }
    let mut merged: Vec<Node> = groups.iter().map(|_| Node::default()).collect();
    for (i, (k, child)) in node.statics.take().into_iter().enumerate() {
        match assign[i] {
            usize::MAX => {
                node.statics.push(k, child);
            }
            g => merged[g].merge(child),
        }
    }
    for ((kind, _), merged) in groups.into_iter().zip(merged) {
        // A value with pages of its own below (`/artists/[slug]/bio`) is still one thing; one with
        // values below it groups them.
        let children = &merged.statics.list;
        let shared = children.iter().filter(|(_, c)| c.sources >= 2).count();
        let few = children.len() < min && shared * 2 >= children.len();
        // `/programma/[slug]/[id]`: one URL below each value is the same thing, its slug and its id.
        let single = merged.total - merged.pages <= merged.sources + merged.sources / 2;
        let one = merged.pages * 2 >= merged.total || (merged.pages > 0 && few) || single;
        let base = match kind {
            ParamType::Slug if one => "slug",
            ParamType::Slug => "category",
            ParamType::Number if paging => "page",
            ParamType::Number | ParamType::Uuid | ParamType::Hash => "id",
            ParamType::Year => "year",
            ParamType::Month => "month",
            ParamType::Day => "day",
            ParamType::Date => "date",
            ParamType::Locale => "locale",
        };
        let mut name = base.to_owned();
        let mut k = 2;
        while cx.names.contains(&name) || node.params.iter().any(|(p, _)| p.name == name) {
            name = format!("{base}{k}");
            k += 1;
        }
        node.params.push((Param { name, kind }, merged));
    }
    // Typed parameters before `[slug]`, which takes any value.
    node.params.sort_by_key(|(p, _)| p.kind == ParamType::Slug);
    for (name, child) in &mut node.statics.list {
        let cx = Context {
            depth: cx.depth + 1,
            parent_static: Some(name),
            parent_param: None,
            names: cx.names.clone(),
        };
        collapse(child, &cx, opts);
    }
    for (param, child) in &mut node.params {
        let mut names = cx.names.clone();
        names.push(param.name.clone());
        let cx = Context {
            depth: cx.depth + 1,
            parent_static: None,
            parent_param: Some(param.kind),
            names,
        };
        collapse(child, &cx, opts);
    }
}

fn to_route(node: &Node, segment: String, param: Option<Param>, pattern: String) -> Route {
    let mut children = Vec::new();
    for (name, child) in &node.statics.list {
        children.push(to_route(child, name.to_string(), None, format!("{pattern}/{name}")));
    }
    for (p, child) in &node.params {
        let seg = format!("[{}]", p.name);
        children.push(to_route(
            child,
            seg.clone(),
            Some(p.clone()),
            format!("{pattern}/{seg}"),
        ));
    }
    Route {
        segment,
        param,
        pattern: if pattern.is_empty() { "/".to_owned() } else { pattern },
        pages: 0,
        total: 0,
        kind: RouteKind::Section,
        topic: None,
        score: 0.0,
        examples: Vec::new(),
        lastmod: None,
        images: 0,
        children,
        valued: 0,
    }
}

/// Routes no URL is (a parameter another one, tried first, took every value of) are left out.
fn prune(r: &mut Route) {
    r.children.retain(|c| c.total > 0);
    for c in &mut r.children {
        prune(c);
    }
}

/// Counts `input` at the route its path is, and at every route above it.
fn tally(r: &mut Route, segs: &[&str], input: &Input, opts: &RouteOptions) -> bool {
    let Some((first, rest)) = segs.split_first() else {
        r.pages += 1;
        r.total += 1;
        if r.examples.len() < opts.max_examples {
            r.examples.push(input.url.clone());
        }
        if let Some(m) = &input.lastmod {
            // W3C dates compare as text; a feed's RFC 822 dates are left out.
            if m.starts_with(|c: char| c.is_ascii_digit()) && r.lastmod.as_ref().is_none_or(|l| m > l) {
                r.lastmod = Some(m.clone());
            }
        }
        if input.images {
            r.images += 1;
        }
        return true;
    };
    let statics = r.children.iter().position(|c| c.param.is_none() && c.segment == *first);
    let found = statics.is_some_and(|k| tally(&mut r.children[k], rest, input, opts))
        || (0..r.children.len()).any(|k| {
            let fits = r.children[k].param.as_ref().is_some_and(|p| fits(p.kind, first));
            if fits && tally(&mut r.children[k], rest, input, opts) {
                if rest.is_empty() && is_value(first) {
                    r.children[k].valued += 1;
                }
                return true;
            }
            false
        });
    if found {
        r.total += 1;
    }
    found
}

fn find_in<'a>(r: &'a Route, segs: &[&str]) -> Option<&'a Route> {
    let Some((first, rest)) = segs.split_first() else {
        return Some(r);
    };
    if let Some(found) = r
        .children
        .iter()
        .filter(|c| c.param.is_none() && c.segment == *first)
        .find_map(|c| find_in(c, rest))
    {
        return Some(found);
    }
    r.children
        .iter()
        .filter(|c| c.param.as_ref().is_some_and(|p| fits(p.kind, first)))
        .find_map(|c| find_in(c, rest))
}

/// Kinds, topics and scores, from the counts.
fn label(r: &mut Route, parent_static: Option<&str>, topic: Option<&'static str>, root: bool) {
    let own_topic = if r.param.is_none() { topic_of(&r.segment) } else { None };
    r.topic = own_topic.or(topic);
    let next_static = if r.param.is_none() && !root {
        Some(r.segment.clone())
    } else {
        None
    };
    for c in &mut r.children {
        label(c, next_static.as_deref(), r.topic, false);
    }
    r.children
        .sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.segment.cmp(&b.segment)));
    r.kind = if root {
        RouteKind::Root
    } else if let Some(p) = &r.param {
        let taxonomy = parent_static.is_some_and(is_taxonomy_word);
        match p.kind {
            _ if p.name.starts_with("page") || parent_static.is_some_and(is_page_word) => RouteKind::Pagination,
            ParamType::Year | ParamType::Month | ParamType::Day => RouteKind::Archive,
            ParamType::Locale => RouteKind::Section,
            _ if taxonomy => RouteKind::Taxonomy,
            _ if r.pages > 0 && (r.pages * 2 >= r.total || r.children.iter().all(|c| c.param.is_none())) => {
                RouteKind::Item
            }
            _ if r.pages > 0
                && r.children
                    .iter()
                    .any(|c| c.kind == RouteKind::Item && c.param.is_some()) =>
            {
                RouteKind::Listing
            }
            _ => RouteKind::Section,
        }
    } else if r.pages > 0
        && r.children
            .iter()
            .any(|c| c.kind == RouteKind::Item && c.param.is_some())
    {
        RouteKind::Listing
    } else if r.children.is_empty() {
        RouteKind::Page
    } else {
        RouteKind::Section
    };
    r.score = if r.kind == RouteKind::Item {
        let pages = r.pages as f64;
        let s = 0.4
            + 0.25 * ((pages + 1.0).log10() / 3.0).min(1.0)
            + 0.15 * (r.valued as f64 / pages)
            + 0.1 * (pages / r.total as f64)
            + if r.topic.is_some() { 0.1 } else { 0.0 };
        (s * 100.0).round() / 100.0
    } else {
        0.0
    };
}

/// The path segments of `url` (after the host, before `?`), an `index.html` at the end left out.
fn segments(url: &str, max: usize) -> Vec<&str> {
    let after = url.find("://").map_or(url, |k| &url[k + 3..]);
    let path = after.find('/').map_or("", |k| &after[k..]);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).take(max).collect();
    if segs.last().is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "index.html" | "index.htm" | "index.php"
        )
    }) {
        segs.pop();
    }
    segs
}

// ── Segments ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Year,
    Number(u64),
    Date,
    Uuid,
    Hash,
    Locale,
    /// Three words or more, letters with digits, or long: a value.
    Slug,
    /// Two words: a value or a page (`about-us`).
    ShortSlug,
    /// One word.
    Word,
}

/// The segment without a file extension, lowercased (a copy only when it was not).
fn stem(seg: &str) -> Cow<'_, str> {
    fn bare(s: &str) -> &str {
        [".html", ".htm", ".php", ".aspx", ".asp", ".jsp"]
            .iter()
            .find_map(|ext| {
                let cut = s.len().checked_sub(ext.len())?;
                (s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(ext)).then(|| &s[..cut])
            })
            .unwrap_or(s)
    }
    if seg.bytes().all(|b| b.is_ascii() && !b.is_ascii_uppercase()) {
        Cow::Borrowed(bare(seg))
    } else {
        let lower = seg.to_lowercase();
        Cow::Owned(bare(&lower).to_owned())
    }
}

fn shape(seg: &str, depth: usize) -> Shape {
    let s = stem(seg);
    let b = s.as_bytes();
    let digits = !b.is_empty() && b.iter().all(u8::is_ascii_digit);
    if digits && b.len() == 4 && (1900..=2100).contains(&s.parse::<u32>().unwrap_or(0)) {
        return Shape::Year;
    }
    if digits && b.len() == 8 && is_date(&format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..])) {
        return Shape::Date;
    }
    if digits {
        return Shape::Number(s.parse().unwrap_or(u64::MAX));
    }
    if is_date(&s) || (b.len() == 7 && is_date(&format!("{s}-01"))) {
        return Shape::Date;
    }
    if is_uuid(b) {
        return Shape::Uuid;
    }
    if depth == 0 && is_locale(&s) {
        return Shape::Locale;
    }
    let words = s.split(['-', '_', '+', '.', ' ']).filter(|w| !w.is_empty()).count();
    let letters = s.chars().any(char::is_alphabetic);
    let has_digit = b.iter().any(u8::is_ascii_digit);
    if words == 1 && has_digit && letters && is_hash(seg) {
        return Shape::Hash;
    }
    if words >= 3 || (has_digit && letters) || s.chars().count() >= 24 {
        Shape::Slug
    } else if words == 2 || !letters {
        Shape::ShortSlug
    } else {
        Shape::Word
    }
}

/// Whether `seg` fits a parameter of type `t`.
fn fits(t: ParamType, seg: &str) -> bool {
    match t {
        ParamType::Slug => true,
        _ => {
            let shape = shape(seg, if t == ParamType::Locale { 0 } else { 1 });
            match t {
                ParamType::Year => shape == Shape::Year,
                ParamType::Number => matches!(shape, Shape::Number(_) | Shape::Year),
                ParamType::Month => matches!(shape, Shape::Number(1..=12)),
                ParamType::Day => matches!(shape, Shape::Number(1..=31)),
                ParamType::Date => shape == Shape::Date,
                ParamType::Uuid => shape == Shape::Uuid,
                ParamType::Hash => shape == Shape::Hash,
                ParamType::Locale => shape == Shape::Locale,
                ParamType::Slug => true,
            }
        }
    }
}

/// A last segment that names one thing: a long slug, a number, a date, an id.
fn is_value(seg: &str) -> bool {
    matches!(
        shape(seg, 1),
        Shape::Slug | Shape::Number(_) | Shape::Date | Shape::Uuid | Shape::Hash
    )
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [&b[..4], &b[5..7], &b[8..]]
            .iter()
            .all(|p| p.iter().all(u8::is_ascii_digit))
        && (1..=12).contains(&s[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&s[8..].parse::<u8>().unwrap_or(0))
}

fn is_uuid(b: &[u8]) -> bool {
    b.len() == 36
        && b.iter().enumerate().all(|(k, &c)| {
            if matches!(k, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// An opaque id: hex of 8 or more, or 8 or more letters and digits in both cases.
fn is_hash(seg: &str) -> bool {
    let b = seg.as_bytes();
    if b.len() < 8 || !b.iter().all(u8::is_ascii_alphanumeric) {
        return false;
    }
    let hex = b.iter().all(u8::is_ascii_hexdigit);
    let upper = b.iter().any(u8::is_ascii_uppercase);
    let lower = b.iter().any(u8::is_ascii_lowercase);
    hex || (upper && lower)
}

const LANGUAGES: &[&str] = &[
    "ar", "bg", "ca", "cs", "cy", "da", "de", "el", "en", "es", "et", "eu", "fi", "fr", "ga", "gl", "he", "hi", "hr",
    "hu", "id", "is", "it", "ja", "ko", "lt", "lv", "ms", "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sk", "sl",
    "sr", "sv", "th", "tr", "uk", "vi", "zh",
];

fn is_locale(s: &str) -> bool {
    let (lang, region) = match s.split_once(['-', '_']) {
        Some((l, r)) => (l, Some(r)),
        None => (s, None),
    };
    LANGUAGES.contains(&lang) && region.is_none_or(|r| r.len() == 2 && r.bytes().all(|c| c.is_ascii_alphabetic()))
}

/// Words a site keeps for its structure: never a parameter's value.
fn is_structural(seg: &str) -> bool {
    let s = stem(seg);
    is_page_word(&s)
        || is_taxonomy_word(&s)
        || matches!(
            &*s,
            "archive"
                | "archief"
                | "archiv"
                | "archives"
                | "search"
                | "zoeken"
                | "suche"
                | "recherche"
                | "buscar"
                | "feed"
                | "rss"
                | "atom"
                | "all"
                | "alle"
                | "about"
                | "about-us"
                | "over"
                | "over-ons"
                | "contact"
                | "kontakt"
                | "privacy"
                | "cookies"
                | "terms"
                | "voorwaarden"
                | "login"
                | "account"
                | "cart"
                | "winkelwagen"
                | "checkout"
                | "faq"
                | "help"
                | "home"
                | "sitemap"
                | "impressum"
                | "disclaimer"
        )
}

fn is_page_word(seg: &str) -> bool {
    matches!(&*stem(seg), "page" | "pagina" | "seite" | "pg")
}

fn is_taxonomy_word(seg: &str) -> bool {
    matches!(
        &*stem(seg),
        "category"
            | "categories"
            | "categorie"
            | "categorieen"
            | "cat"
            | "kategorie"
            | "kategorien"
            | "tag"
            | "tags"
            | "genre"
            | "genres"
            | "type"
            | "types"
            | "topic"
            | "topics"
            | "onderwerp"
            | "onderwerpen"
            | "thema"
            | "themas"
            | "rubriek"
            | "author"
            | "authors"
            | "auteur"
            | "collection"
            | "collections"
            | "brand"
            | "brands"
            | "merk"
            | "merken"
    )
}

/// What a section's pages are, from its name (English, Dutch, German, French, Spanish).
fn topic_of(seg: &str) -> Option<&'static str> {
    Some(match &*stem(seg) {
        "event" | "events" | "agenda" | "calendar" | "kalender" | "programma" | "programme" | "program"
        | "evenement" | "evenementen" | "evenements" | "voorstelling" | "voorstellingen" | "concert" | "concerts"
        | "concerten" | "show" | "shows" | "gig" | "gigs" | "festival" | "festivals" | "veranstaltung"
        | "veranstaltungen" | "termine" | "evento" | "eventos" | "whats-on" | "whatson" | "activiteit"
        | "activiteiten" | "uitagenda" | "performances" | "ticket" | "tickets" => "event",
        "product" | "products" | "producten" | "produkt" | "produkte" | "produit" | "produits" | "producto"
        | "productos" | "shop" | "store" | "winkel" | "item" | "items" => "product",
        "news" | "nieuws" | "blog" | "blogs" | "post" | "posts" | "article" | "articles" | "artikelen"
        | "nachrichten" | "actualites" | "actualiteiten" | "noticias" | "magazine" | "stories" | "story"
        | "journal" | "updates" | "verhalen" => "article",
        "artist" | "artists" | "artiest" | "artiesten" | "act" | "acts" | "performer" | "performers" | "lineup"
        | "line-up" | "people" | "team" | "speaker" | "speakers" | "spreker" | "sprekers" | "kunstenaar"
        | "kunstenaars" | "kuenstler" => "person",
        "venue" | "venues" | "location" | "locations" | "locatie" | "locaties" | "zaal" | "zalen" | "place"
        | "places" | "podium" | "podia" => "venue",
        "job" | "jobs" | "vacature" | "vacatures" | "vacancy" | "vacancies" | "career" | "careers" | "werken-bij" => {
            "job"
        }
        "recipe" | "recipes" | "recept" | "recepten" => "recipe",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(urls: &[String]) -> Routes {
        infer(urls.iter().map(String::as_str), RouteOptions::default())
    }

    fn patterns(r: &Routes) -> Vec<(String, usize, usize, RouteKind)> {
        fn walk(r: &Route, out: &mut Vec<(String, usize, usize, RouteKind)>) {
            out.push((r.pattern.clone(), r.pages, r.total, r.kind));
            for c in &r.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(&r.sites[0].root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn site(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| format!("https://venue.example{p}")).collect()
    }

    #[test]
    fn a_venue() {
        let mut paths: Vec<String> = ["/", "/agenda", "/contact", "/over-ons", "/nieuws", "/agenda/archief"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        for s in [
            "nora-live-in-paradiso",
            "jazz-night",
            "open-air-2026",
            "de-dijk",
            "kensington",
        ] {
            paths.push(format!("/agenda/{s}"));
        }
        for n in 2..=4 {
            paths.push(format!("/agenda/page/{n}"));
        }
        for (y, m, s) in [
            ("2025", "11", "a-new-hall"),
            ("2026", "01", "season-two-opens"),
            ("2026", "02", "hello"),
        ] {
            paths.push(format!("/nieuws/{y}/{m}/{s}"));
        }
        let urls: Vec<String> = paths.iter().map(|p| format!("https://www.venue.example{p}")).collect();
        let r = routes(&urls);
        assert_eq!(r.sites.len(), 1);
        assert_eq!(r.sites[0].urls, urls.len());
        assert_eq!(
            patterns(&r),
            [
                ("/".into(), 1, 17, RouteKind::Root),
                ("/agenda".into(), 1, 10, RouteKind::Listing),
                ("/agenda/[slug]".into(), 5, 5, RouteKind::Item),
                ("/agenda/archief".into(), 1, 1, RouteKind::Page),
                ("/agenda/page".into(), 0, 3, RouteKind::Section),
                ("/agenda/page/[page]".into(), 3, 3, RouteKind::Pagination),
                ("/contact".into(), 1, 1, RouteKind::Page),
                ("/nieuws".into(), 1, 4, RouteKind::Section),
                ("/nieuws/[year]".into(), 0, 3, RouteKind::Archive),
                ("/nieuws/[year]/[month]".into(), 0, 3, RouteKind::Archive),
                ("/nieuws/[year]/[month]/[slug]".into(), 3, 3, RouteKind::Item),
                ("/over-ons".into(), 1, 1, RouteKind::Page),
            ]
        );
        let mut items: Vec<_> = r.items().iter().map(|r| (r.pattern.as_str(), r.topic)).collect();
        items.sort();
        assert_eq!(
            items,
            [
                ("/agenda/[slug]", Some("event")),
                ("/nieuws/[year]/[month]/[slug]", Some("article"))
            ]
        );
        let found = r.find("https://venue.example/agenda/someone-new").unwrap();
        assert_eq!(found.pattern, "/agenda/[slug]");
        assert_eq!(
            r.find("https://venue.example/agenda/page/9").unwrap().pattern,
            "/agenda/page/[page]"
        );
        assert_eq!(
            r.find("https://venue.example/agenda/archief").unwrap().pattern,
            "/agenda/archief"
        );
        assert!(r.find("https://venue.example/agenda/a/b/c").is_none());
        assert!(r.find("https://other.example/agenda").is_none());
        let outline = r.outline();
        assert!(outline.contains("page/[page]"), "{outline}");
        assert!(outline.contains("[slug]"), "{outline}");
    }

    #[test]
    fn a_shop_with_locales_and_ids() {
        let mut paths = Vec::new();
        for l in ["en", "nl"] {
            for c in ["shoes", "bags", "hats", "coats", "socks", "belts"] {
                paths.push(format!("/{l}/products/{c}"));
                for p in 0..4 {
                    paths.push(format!("/{l}/products/{c}/{c}-model-{p}"));
                }
            }
        }
        for id in [101, 202, 303] {
            paths.push(format!("/p/{id}"));
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p = patterns(&r);
        assert!(
            p.contains(&("/[locale]/products/[category]".into(), 12, 60, RouteKind::Listing)),
            "{p:#?}"
        );
        assert!(
            p.contains(&("/[locale]/products/[category]/[slug]".into(), 48, 48, RouteKind::Item)),
            "{p:#?}"
        );
        assert!(p.contains(&("/p/[id]".into(), 3, 3, RouteKind::Item)), "{p:#?}");
        assert_eq!(r.items()[0].pattern, "/[locale]/products/[category]/[slug]");
        assert_eq!(r.items()[0].topic, Some("product"));
    }

    #[test]
    fn pages_at_the_root_stay_pages() {
        let r = routes(&site(&[
            "/",
            "/about",
            "/contact",
            "/tickets",
            "/route",
            "/faq",
            "/partners",
            "/press",
        ]));
        assert!(
            patterns(&r).iter().all(|(p, ..)| !p.contains('[')),
            "{:?}",
            patterns(&r)
        );
        // A blog at the root: its posts' slugs become a parameter, the pages stay.
        let r = routes(&site(&[
            "/about",
            "/contact",
            "/my-first-post-ever",
            "/why-we-love-jazz",
            "/a-night-at-the-opera",
            "/five-records-to-hear",
            "/the-hall-turns-forty",
        ]));
        let p = patterns(&r);
        assert!(p.contains(&("/[slug]".into(), 5, 5, RouteKind::Item)), "{p:?}");
        assert!(p.contains(&("/about".into(), 1, 1, RouteKind::Page)), "{p:?}");
    }

    #[test]
    fn taxonomies_and_uuids() {
        let mut paths = Vec::new();
        for t in ["rock", "jazz", "pop", "techno", "folk", "metal"] {
            paths.push(format!("/tag/{t}"));
        }
        for u in [
            "0e8f7a52-1c2b-4c1e-9a6b-1f2e3d4c5b6a",
            "9b1d2c3e-4f50-6172-8394-a5b6c7d8e9f0",
        ] {
            paths.push(format!("/e/{u}"));
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p = patterns(&r);
        assert!(p.contains(&("/tag/[slug]".into(), 6, 6, RouteKind::Taxonomy)), "{p:?}");
        assert!(p.contains(&("/e/[id]".into(), 2, 2, RouteKind::Item)), "{p:?}");
    }

    #[test]
    fn sitemap_entries_and_sites() {
        let mut b = RouteBuilder::new(RouteOptions::default());
        for (k, s) in ["a-b-c", "d-e-f", "g-h-i"].iter().enumerate() {
            b.add_entry(&Entry {
                kind: EntryKind::Url,
                url: format!("https://venue.example/events/{s}"),
                lastmod: Some(format!("2026-10-0{}", k + 1)),
                changefreq: None,
                priority: None,
                images: if k == 0 {
                    vec!["https://venue.example/i.jpg".into()]
                } else {
                    Vec::new()
                },
                alternates: Vec::new(),
                title: None,
            });
        }
        b.add("https://other.example/");
        b.add("mailto:x@venue.example");
        b.add("/relative");
        let r = b.finish();
        assert_eq!(r.sites.len(), 2);
        assert_eq!(r.sites[0].origin, "https://venue.example");
        let item = r.find("https://venue.example/events/x-y-z").unwrap();
        assert_eq!(item.lastmod.as_deref(), Some("2026-10-03"));
        assert_eq!(item.images, 1);
        assert_eq!(item.examples.len(), 3);
        assert_eq!(
            r.warnings,
            ["routes: 2 URLs that are not absolute http(s) were left out"]
        );
    }

    #[test]
    fn names_are_unique_along_a_route() {
        let mut paths = Vec::new();
        for a in ["first-one-here", "second-one-here"] {
            for b in ["third-one-here", "fourth-one-here", "fifth-one-here"] {
                paths.push(format!("/x/{a}/{b}-{a}"));
            }
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        assert!(
            patterns(&r).iter().any(|(p, ..)| p == "/x/[category]/[slug]"),
            "{:?}",
            patterns(&r)
        );
        let mut paths = Vec::new();
        for a in ["first-one-here", "second-one-here", "third-one-here"] {
            paths.push(format!("/x/{a}"));
        }
        for b in ["fourth-one-here", "fifth-one-here"] {
            paths.push(format!("/x/first-one-here/{b}"));
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p = patterns(&r);
        assert!(p.iter().any(|(p, ..)| p == "/x/[slug]/[slug2]"), "{p:?}");
    }

    #[test]
    fn hostile_input_stays_bounded() {
        // Deep, wide and repetitive paths: linear work, no recursion past max_depth.
        let deep = format!("https://venue.example/{}", "a/".repeat(10_000));
        let mut urls = vec![deep];
        for k in 0..50_000 {
            urls.push(format!("https://venue.example/w/{k:x}-slug-{k}"));
        }
        let r = routes(&urls);
        assert_eq!(r.sites[0].urls, urls.len());
        assert!(r.find("https://venue.example/w/new-slug-here").is_some());
        let opts = RouteOptions {
            max_urls: 10,
            ..RouteOptions::default()
        };
        let r = infer(urls.iter().map(String::as_str), opts);
        assert_eq!(r.sites[0].urls, 10);
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn pages_each_value_has_stay_pages() {
        let mut paths = Vec::new();
        for a in ["nora", "bob-dylan", "kensington", "de-staat", "golden-earring", "anouk"] {
            paths.push(format!("/artists/{a}"));
            paths.push(format!("/artists/{a}/bio"));
            paths.push(format!("/artists/{a}/events"));
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p = patterns(&r);
        assert!(p.contains(&("/artists/[slug]".into(), 6, 18, RouteKind::Item)), "{p:?}");
        assert!(
            p.contains(&("/artists/[slug]/bio".into(), 6, 6, RouteKind::Page)),
            "{p:?}"
        );
        assert_eq!(
            r.find("https://venue.example/artists/someone/events").unwrap().pattern,
            "/artists/[slug]/events"
        );
    }

    #[test]
    fn sections_under_a_locale_stay_sections() {
        let mut paths = Vec::new();
        for l in ["en", "nl"] {
            paths.push(format!("/{l}/agenda"));
            paths.push(format!("/{l}/nieuws"));
            paths.push(format!("/{l}/contact"));
            for s in ["3x3-eyes-1991", "nora-live-tonight", "a-night-of-jazz"] {
                paths.push(format!("/{l}/agenda/{s}-{l}"));
            }
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p = patterns(&r);
        assert!(
            p.contains(&("/[locale]/agenda/[slug]".into(), 6, 6, RouteKind::Item)),
            "{p:?}"
        );
        assert!(p.contains(&("/[locale]/nieuws".into(), 2, 2, RouteKind::Page)), "{p:?}");
        assert_eq!(r.items()[0].topic, Some("event"));
    }

    #[test]
    fn numbers_and_dates_under_values() {
        let mut paths = Vec::new();
        for k in (1990..2010).chain(5000..5003) {
            paths.push(format!("/blog/{k}/post-title-{k}"));
        }
        for y in 2020..2024 {
            for m in [2, 6, 10] {
                paths.push(format!("/agenda/{y}/{m}/event-{y}-{m}-night"));
            }
        }
        let r = routes(&site(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
        let p: Vec<_> = patterns(&r).into_iter().map(|p| p.0).collect();
        assert_eq!(
            p,
            [
                "/",
                "/agenda",
                "/agenda/[year]",
                "/agenda/[year]/[month]",
                "/agenda/[year]/[month]/[slug]",
                "/blog",
                "/blog/[id]",
                "/blog/[id]/[slug]",
            ]
        );
    }
}
