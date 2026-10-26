//! What a scraper wants to know of any page, read in the same pass: title,
//! description, language, canonical URL, robots rules, a meta refresh,
//! every `<meta>` (OpenGraph, Twitter cards, `name=` and `property=`
//! alike), alternates by language, feeds and calendars, icons, pagination,
//! the links with their text, the images (lazy-loaded ones too), headings,
//! embedded frames, e-mail addresses and phone numbers, and the JSON pages
//! carry for their scripts (`__NEXT_DATA__`, `<script type="application/json">`,
//! `window.__INITIAL_STATE__ = {…}`), and on request the page's visible
//! text, a line per block.
//!
//! Every URL is absolute (against `<base href>` when there is one); links
//! and images are http(s) and without `#…`. Every list is capped and every
//! text cut; what is malformed is skipped, never an error.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Map, Value};
use url::Url;

use crate::structured::lenient;
use crate::text::{collapse, cut};
use crate::tree::{Handler, Text, Tree};

/// What the page pass reads, and how much of it.
#[derive(Clone, Debug)]
pub struct PageOptions {
    pub links: bool,
    pub images: bool,
    pub headings: bool,
    /// JSON in scripts (`__NEXT_DATA__`, `application/json`, `window.X = {…}`).
    pub embedded: bool,
    pub max_links: usize,
    pub max_images: usize,
    pub max_headings: usize,
    /// The most characters a link's or heading's text keeps.
    pub max_text: usize,
    /// At most this many embedded values, from scripts of at most this many bytes.
    pub max_embedded: usize,
    pub max_script_bytes: usize,
    /// The page's visible text, a line per block ([`Page::text`]).
    pub text: bool,
    /// The most bytes [`Page::text`] holds.
    pub max_text_bytes: usize,
}

impl Default for PageOptions {
    fn default() -> Self {
        PageOptions {
            links: true,
            images: true,
            headings: true,
            embedded: true,
            max_links: 5000,
            max_images: 2000,
            max_headings: 500,
            max_text: 240,
            max_embedded: 32,
            max_script_bytes: 8 << 20,
            text: false,
            max_text_bytes: 4 << 20,
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize)]
pub struct Robots {
    /// `noindex` (or `none`) not stated.
    pub index: bool,
    /// `nofollow` (or `none`) not stated.
    pub follow: bool,
    /// Every directive of `robots` and of a bot's own meta (`googlebot`), lowercase.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub directives: Vec<String>,
}

impl Default for Robots {
    fn default() -> Self {
        Robots {
            index: true,
            follow: true,
            directives: Vec::new(),
        }
    }
}

/// `<meta http-equiv="refresh" content="5; url=/next">`.
#[derive(Clone, Debug, Serialize)]
pub struct Refresh {
    pub seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Alternate {
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hreflang: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Icon {
    pub href: String,
    pub rel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizes: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Link {
    pub href: String,
    /// Its text, or else its `title`, `aria-label` or an image's `alt` in it.
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rel: Vec<String>,
    /// `rel="nofollow"` (or `ugc`, `sponsored`).
    #[serde(skip_serializing_if = "is_false")]
    pub nofollow: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Image {
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// JSON a page carries for its scripts.
#[derive(Clone, Debug, Serialize)]
pub struct Embedded {
    /// Where it was: a script's `id` (`__NEXT_DATA__`), the name assigned (`window.__INITIAL_STATE__`),
    /// or `application/json`.
    pub name: String,
    pub value: Value,
}

/// What a scraper wants to know of a page.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Page {
    /// The encoding the page was read in (`UTF-8`, `windows-1252`).
    pub charset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `<meta name="description">`, else OpenGraph's or Twitter's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `<html lang>`, else `content-language`, else `og:locale`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub robots: Robots,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<Refresh>,
    /// Every `<meta name|property|itemprop content>` by its key, lowercase (the first of each).
    pub meta: Map<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<Alternate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amp: Option<String>,
    /// RSS, Atom and JSON feeds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub feeds: Vec<Alternate>,
    /// iCalendar: `<link type="text/calendar">`, `webcal:` and `.ics` links.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<Icon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headings: Vec<Heading>,
    /// `<iframe src>`: maps, videos, ticket shops.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phones: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embedded: Vec<Embedded>,
    /// With [`PageOptions::text`]: the text a reader sees, a line per block (a heading, a
    /// paragraph, a list item, a table row) or `<br>`, spaces in a line collapsed, nothing of
    /// the `<head>`, scripts, styles, `<template>` and `<noscript>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A bot wall instead of the page: `cloudflare`, `datadome`, `perimeterx`, `incapsula`,
    /// `akamai` or `captcha`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    /// What was cut at a limit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Text as people read it: whitespace collapsed, soft hyphens and zero-width characters out.
fn clean(text: &str) -> String {
    const INVISIBLE: [char; 6] = ['\u{ad}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}'];
    if text.contains(INVISIBLE) {
        collapse(&text.replace(INVISIBLE, ""))
    } else {
        collapse(text)
    }
}

/// Which bot wall a script, form or frame address belongs to.
fn wall(src: &str) -> Option<&'static str> {
    let s = src.to_ascii_lowercase();
    if s.contains("/cdn-cgi/challenge-platform") || s.contains("challenges.cloudflare.com") {
        Some("cloudflare")
    } else if s.contains("captcha-delivery.com") || s.contains("datadome") {
        Some("datadome")
    } else if s.contains("px-cdn.net") || s.contains("perimeterx") || s.contains("px-captcha") {
        Some("perimeterx")
    } else if s.contains("_incapsula_resource") {
        Some("incapsula")
    } else {
        None
    }
}

enum Capture {
    Title,
    /// A link, and the text that stands in when it has none (`title`, `aria-label`, an image's `alt`).
    Link(usize, String),
    Heading(u8),
    /// A script's text: its name when it is JSON, none for JavaScript to look into.
    Script(Option<String>),
    /// A script past `max_script_bytes`: not read, and nothing more joins on.
    Dropped,
}

pub(crate) struct PagePass {
    opts: PageOptions,
    page: Page,
    url: Option<Url>,
    base: Option<Url>,
    base_seen: bool,
    captures: Vec<(usize, Capture, String)>,
    /// Links by URL (the first keeps its place; a later one with text gives it its text).
    seen_links: HashMap<String, usize>,
    seen_images: HashMap<String, ()>,
    /// The `<a>` open, for an image's `alt` to stand in for an empty text.
    open_link: Option<(usize, usize)>,
    svg_depth: Option<usize>,
    html_lang: Option<String>,
    /// The page's text so far ([`PageOptions::text`]).
    text: Option<PageText>,
}

/// The page's text as it streams by.
struct PageText {
    out: String,
    /// Inside an element whose text is never shown (its frame).
    skip: Option<usize>,
    full: bool,
}

impl PageText {
    fn push(&mut self, s: &str, max: usize) {
        if self.full {
            return;
        }
        if self.out.len() + s.len() > max {
            self.out.push_str(crate::text::cut_bytes(s, max - self.out.len()));
            self.full = true;
        } else {
            self.out.push_str(s);
        }
    }

    fn open(&mut self, depth: usize, kind: u8, max: usize) {
        if self.skip.is_some() {
            return;
        }
        if crate::tags::is_never_text(kind) || matches!(kind, crate::tags::HEAD | crate::tags::TITLE) {
            self.skip = Some(depth);
        } else {
            self.brk(kind, max);
        }
    }

    fn close(&mut self, depth: usize, kind: u8, max: usize) {
        if self.skip == Some(depth) {
            self.skip = None;
        } else if self.skip.is_none() {
            self.brk(kind, max);
        }
    }

    /// Where an element opens or closes: a line break for a block, a space for the rest but inline ones.
    fn brk(&mut self, kind: u8, max: usize) {
        match crate::tags::text_break(kind, true) {
            Some('\n') => self.push("\n", max),
            Some(_) => self.push(" ", max),
            None => {}
        }
    }
}

impl PagePass {
    pub(crate) fn new(opts: PageOptions, url: Option<Url>) -> PagePass {
        PagePass {
            page: Page::default(),
            base: url.clone(),
            url,
            base_seen: false,
            captures: Vec::new(),
            seen_links: HashMap::new(),
            seen_images: HashMap::new(),
            open_link: None,
            svg_depth: None,
            html_lang: None,
            text: opts.text.then(|| PageText {
                out: String::new(),
                skip: None,
                full: false,
            }),
            opts,
        }
    }

    pub(crate) fn wants_text(&self) -> bool {
        !self.captures.is_empty() || self.text.is_some()
    }

    /// `href` absolute, any scheme (feeds, icons, canonical).
    fn absolute(&self, href: &str) -> Option<String> {
        let href = href.trim();
        if href.is_empty() || href.starts_with("data:") || href.starts_with("javascript:") {
            return None;
        }
        let mut u = match &self.base {
            Some(b) => b.join(href).ok()?,
            None => Url::parse(href).ok()?,
        };
        u.set_fragment(None);
        Some(u.to_string())
    }

    /// `href` as a page to visit: absolute, http(s), no `#…` (`webcal:` as `https:`).
    fn http(&self, href: &str) -> Option<String> {
        match &self.base {
            Some(b) => crate::value::http_url(href, b),
            None => Url::parse(href.trim())
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https"))
                .map(|mut u| {
                    u.set_fragment(None);
                    u.to_string()
                }),
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.page.warnings.iter().any(|x| x == w) {
            self.page.warnings.push(w.to_owned());
        }
    }

    fn meta(&mut self, key: &str, content: &str) {
        let key = key.trim().to_ascii_lowercase();
        let content = collapse(content);
        if key.is_empty() || content.is_empty() || self.page.meta.len() >= 1000 {
            return;
        }
        self.page
            .meta
            .entry(key)
            .or_insert(Value::String(cut(&content, 8192).to_owned()));
    }

    fn calendar(&mut self, href: &str) {
        if let Some(u) = self.absolute(href) {
            let u = match u.get(..7) {
                Some(p) if p.eq_ignore_ascii_case("webcal:") => format!("https:{}", &u[7..]),
                _ => u,
            };
            if !self.page.calendars.contains(&u) && self.page.calendars.len() < 100 {
                self.page.calendars.push(u);
            }
        }
    }

    fn open_meta(&mut self, attr: &dyn Fn(&str) -> Option<String>) {
        let content = attr("content");
        if let (Some(equiv), Some(content)) = (attr("http-equiv"), content.as_deref()) {
            let equiv = equiv.trim().to_ascii_lowercase();
            match equiv.as_str() {
                "refresh" if self.page.refresh.is_none() => self.page.refresh = refresh(content, self),
                "content-language" if self.html_lang.is_none() && self.page.lang.is_none() => {
                    let l = content.split(',').next().unwrap_or("").trim();
                    if !l.is_empty() {
                        self.page.lang = Some(l.to_owned());
                    }
                }
                _ => {}
            }
            self.meta(&equiv, content);
        }
        let Some(content) = content else { return };
        for key in [attr("name"), attr("property"), attr("itemprop")].into_iter().flatten() {
            let k = key.trim().to_ascii_lowercase();
            if matches!(k.as_str(), "robots" | "googlebot" | "bingbot") {
                for d in content
                    .split(',')
                    .map(|d| d.trim().to_ascii_lowercase())
                    .filter(|d| !d.is_empty())
                {
                    match d.as_str() {
                        "noindex" => self.page.robots.index = false,
                        "nofollow" => self.page.robots.follow = false,
                        "none" => {
                            self.page.robots.index = false;
                            self.page.robots.follow = false;
                        }
                        _ => {}
                    }
                    if !self.page.robots.directives.contains(&d) {
                        self.page.robots.directives.push(d);
                    }
                }
            }
            self.meta(&k, &content);
        }
    }

    fn open_link_tag(&mut self, attr: &dyn Fn(&str) -> Option<String>) {
        let (Some(rel), Some(href)) = (attr("rel"), attr("href")) else {
            return;
        };
        let rels: Vec<String> = rel.split_ascii_whitespace().map(|r| r.to_ascii_lowercase()).collect();
        let kind = attr("type").map(|t| t.trim().to_ascii_lowercase());
        let Some(url) = self.absolute(&href) else { return };
        let has = |r: &str| rels.iter().any(|x| x == r);
        if has("canonical") && self.page.canonical.is_none() {
            self.page.canonical = Some(url.clone());
        }
        if has("amphtml") && self.page.amp.is_none() {
            self.page.amp = Some(url.clone());
        }
        if has("manifest") && self.page.manifest.is_none() {
            self.page.manifest = Some(url.clone());
        }
        if has("next") && self.page.next.is_none() {
            self.page.next = Some(url.clone());
        }
        if (has("prev") || has("previous")) && self.page.prev.is_none() {
            self.page.prev = Some(url.clone());
        }
        if rels.iter().any(|r| r.contains("icon")) && self.page.icons.len() < 50 {
            self.page.icons.push(Icon {
                href: url.clone(),
                rel: rels.join(" "),
                sizes: attr("sizes"),
                kind: kind.clone(),
            });
        }
        let is_calendar = kind.as_deref() == Some("text/calendar") || is_ics(&href);
        if is_calendar {
            self.calendar(&href);
        }
        if has("alternate") && !is_calendar {
            let alt = Alternate {
                href: url,
                hreflang: attr("hreflang"),
                kind: kind.clone(),
                media: attr("media"),
                title: attr("title").map(|t| collapse(&t)),
            };
            let feed = kind.as_deref().is_some_and(|k| {
                matches!(
                    k,
                    "application/rss+xml" | "application/atom+xml" | "application/feed+json" | "application/rdf+xml"
                ) || k.contains("rss")
                    || k.contains("atom")
            });
            let list = if feed {
                &mut self.page.feeds
            } else {
                &mut self.page.alternates
            };
            if list.len() < 500 && !list.iter().any(|a| a.href == alt.href && a.hreflang == alt.hreflang) {
                list.push(alt);
            }
        }
    }

    fn open_anchor(&mut self, depth: usize, attr: &dyn Fn(&str) -> Option<String>, is_a: bool) {
        let Some(href) = attr("href") else { return };
        let h = href.trim();
        let lower = h.get(..7).unwrap_or(h).to_ascii_lowercase();
        if lower.starts_with("mailto:") {
            let addr = h[7..].split('?').next().unwrap_or("").trim();
            let addr = percent_decode(addr);
            for a in addr.split([',', ';']).map(str::trim).filter(|a| a.contains('@')) {
                if !self.page.emails.iter().any(|e| e.eq_ignore_ascii_case(a)) && self.page.emails.len() < 100 {
                    self.page.emails.push(a.to_owned());
                }
            }
            return;
        }
        if lower.starts_with("tel:") || lower.starts_with("callto:") {
            let number = percent_decode(h.split_once(':').map_or("", |(_, n)| n))
                .trim()
                .to_owned();
            if number.chars().filter(char::is_ascii_digit).count() >= 3
                && !self.page.phones.contains(&number)
                && self.page.phones.len() < 100
            {
                self.page.phones.push(number);
            }
            return;
        }
        if is_ics(h) || lower.starts_with("webcal:") {
            self.calendar(h);
        }
        let rels: Vec<String> = attr("rel")
            .map(|r| r.split_ascii_whitespace().map(|r| r.to_ascii_lowercase()).collect())
            .unwrap_or_default();
        let Some(url) = self.http(h) else { return };
        if rels.iter().any(|r| r == "next") && self.page.next.is_none() {
            self.page.next = Some(url.clone());
        }
        if rels.iter().any(|r| r == "prev" || r == "previous") && self.page.prev.is_none() {
            self.page.prev = Some(url.clone());
        }
        if !self.opts.links || !is_a {
            return;
        }
        let fallback = attr("title")
            .or_else(|| attr("aria-label"))
            .map(|t| clean(&t))
            .unwrap_or_default();
        let nofollow = rels
            .iter()
            .any(|r| matches!(r.as_str(), "nofollow" | "ugc" | "sponsored"));
        let index = match self.seen_links.get(&url) {
            Some(&i) => i,
            None => {
                if self.page.links.len() >= self.opts.max_links {
                    self.warn("page: links past max_links were left out");
                    return;
                }
                self.page.links.push(Link {
                    href: url.clone(),
                    text: String::new(),
                    rel: rels,
                    nofollow,
                });
                let i = self.page.links.len() - 1;
                self.seen_links.insert(url, i);
                i
            }
        };
        self.open_link = Some((depth, index));
        self.captures
            .push((depth, Capture::Link(index, fallback), String::new()));
    }

    fn open_image(&mut self, attr: &dyn Fn(&str) -> Option<String>) {
        // Lazy loading puts the real address in a data attribute; `src` holds a placeholder.
        let src = ["data-src", "data-lazy-src", "data-original", "data-lazy", "data-url"]
            .iter()
            .find_map(|a| attr(a).filter(|v| !v.trim().is_empty()))
            .or_else(|| attr("src").filter(|s| !s.trim().starts_with("data:") && !s.trim().is_empty()))
            .or_else(|| attr("data-srcset").or_else(|| attr("srcset")).and_then(|s| largest(&s)));
        let alt = attr("alt").map(|a| clean(&a)).filter(|a| !a.is_empty());
        // An image is a link's text when the link has none.
        if let (Some((_, i)), Some(a)) = (self.open_link, &alt) {
            for (_, c, _) in self.captures.iter_mut().rev() {
                if let Capture::Link(x, fallback) = c {
                    if *x == i && fallback.is_empty() {
                        fallback.clone_from(a);
                    }
                }
            }
        }
        if !self.opts.images {
            return;
        }
        let Some(src) = src.and_then(|s| self.http(&s)) else {
            return;
        };
        if self.seen_images.contains_key(&src) {
            return;
        }
        if self.page.images.len() >= self.opts.max_images {
            self.warn("page: images past max_images were left out");
            return;
        }
        let dim = |a: &str| attr(a).and_then(|v| v.trim().trim_end_matches("px").parse::<u32>().ok());
        self.seen_images.insert(src.clone(), ());
        self.page.images.push(Image {
            src,
            alt,
            width: dim("width"),
            height: dim("height"),
        });
    }

    fn close_capture(&mut self, capture: Capture, text: String) {
        match capture {
            Capture::Title => {
                let t = clean(&text);
                if !t.is_empty() && self.page.title.is_none() {
                    self.page.title = Some(cut(&t, 1000).to_owned());
                }
            }
            Capture::Link(i, fallback) => {
                let t = clean(&text);
                let t = if t.is_empty() { fallback } else { t };
                let t = cut(&t, self.opts.max_text).to_owned();
                if let Some(l) = self.page.links.get_mut(i) {
                    if l.text.is_empty() && !t.is_empty() {
                        l.text = t;
                    }
                }
            }
            Capture::Heading(level) => {
                let t = clean(&text);
                if !t.is_empty() {
                    if self.page.headings.len() < self.opts.max_headings {
                        self.page.headings.push(Heading {
                            level,
                            text: cut(&t, self.opts.max_text).to_owned(),
                        });
                    } else {
                        self.warn("page: headings past max_headings were left out");
                    }
                }
            }
            Capture::Dropped => {}
            Capture::Script(name) => {
                if self.page.embedded.len() >= self.opts.max_embedded {
                    return;
                }
                match name {
                    Some(name) => {
                        if let Some(v) = lenient::read(&text).values.into_iter().next() {
                            if !is_empty_json(&v) {
                                self.page.embedded.push(Embedded { name, value: v });
                            }
                        }
                    }
                    None => {
                        for (name, value) in assignments(&text) {
                            if self.page.embedded.len() >= self.opts.max_embedded {
                                break;
                            }
                            self.page.embedded.push(Embedded { name, value });
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn finish(mut self, charset: &str) -> Page {
        // What was still open at the end of the page.
        while let Some((_, c, t)) = self.captures.pop() {
            self.close_capture(c, t);
        }
        if let Some(t) = self.text.take() {
            if t.full {
                self.warn("page: text was cut at max_text_bytes");
            }
            self.page.text = Some(crate::text::collapse_lines(&t.out));
        }
        let p = &mut self.page;
        p.charset = charset.to_owned();
        if p.blocked.is_none() {
            let title = p.title.as_deref().unwrap_or("").to_ascii_lowercase();
            let named = |n: &str| p.embedded.iter().any(|e| e.name.contains(n));
            p.blocked = if title == "just a moment..."
                || title.starts_with("attention required! | cloudflare")
                || named("_cf_chl_opt")
            {
                Some("cloudflare".into())
            } else if title == "access denied" && p.links.is_empty() {
                Some("akamai".into())
            } else if title.contains("captcha")
                || title.contains("are you a robot")
                || title.contains("verify you are human")
                || title.contains("bot verification")
            {
                Some("captcha".into())
            } else {
                None
            };
        }
        let m = |k: &str| p.meta.get(k).and_then(Value::as_str).map(str::to_owned);
        if p.description.is_none() {
            p.description = m("description")
                .or_else(|| m("og:description"))
                .or_else(|| m("twitter:description"));
        }
        if p.title.is_none() {
            p.title = m("og:title").or_else(|| m("twitter:title"));
        }
        if let Some(l) = self.html_lang.take() {
            p.lang = Some(l);
        } else if p.lang.is_none() {
            p.lang = m("og:locale").or_else(|| m("language")).or_else(|| m("dc.language"));
        }
        if p.canonical.is_none() {
            p.canonical = m("og:url").and_then(|u| match &self.base {
                Some(b) => b.join(&u).ok().map(|u| u.to_string()),
                None => Url::parse(&u).ok().map(|u| u.to_string()),
            });
        }
        self.page
    }
}

impl Handler for PagePass {
    fn open(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        let frame = &tree.frames[depth];
        if let Some(t) = &mut self.text {
            t.open(depth, frame.el.kind, self.opts.max_text_bytes);
        }
        if frame.templates > 0 {
            return;
        }
        let el = &frame.el;
        let name = el.name();
        let attr = |a: &str| el.attr(a).map(str::to_owned);
        // A bot wall's scripts, frames, forms and boxes.
        if self.page.blocked.is_none() && matches!(name, "script" | "iframe" | "form" | "div") {
            let by_address = [el.attr("src"), el.attr("action")].into_iter().flatten().find_map(wall);
            let by_id = el.attr("id").and_then(|id| match id {
                "challenge-running" | "cf-wrapper" | "challenge-stage" | "challenge-form" => Some("cloudflare"),
                "px-captcha" => Some("perimeterx"),
                _ => None,
            });
            self.page.blocked = by_address.or(by_id).map(str::to_owned);
        }
        match name {
            "html" => {
                if let Some(l) = el
                    .attr("lang")
                    .or_else(|| el.attr("xml:lang"))
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                {
                    self.html_lang.get_or_insert_with(|| l.to_owned());
                }
            }
            "base" if !self.base_seen => {
                if let Some(href) = el.attr("href") {
                    self.base_seen = true;
                    let b = match &self.url {
                        Some(u) => u.join(href.trim()).ok(),
                        None => Url::parse(href.trim()).ok(),
                    };
                    if let Some(b) = b {
                        self.page.base = Some(b.to_string());
                        self.base = Some(b);
                    }
                }
            }
            "svg" => {
                self.svg_depth.get_or_insert(depth);
            }
            "title" if self.svg_depth.is_none() && self.page.title.is_none() => {
                self.captures.push((depth, Capture::Title, String::new()));
            }
            "meta" => self.open_meta(&attr),
            "link" => self.open_link_tag(&attr),
            "a" | "area" => self.open_anchor(depth, &attr, name == "a"),
            "img" => self.open_image(&attr),
            "iframe" | "frame" => {
                if let Some(u) = attr("src")
                    .filter(|s| !s.trim().is_empty() && s.trim() != "about:blank")
                    .or_else(|| attr("data-src"))
                    .and_then(|s| self.http(&s))
                {
                    if !self.page.frames.contains(&u) && self.page.frames.len() < 100 {
                        self.page.frames.push(u);
                    }
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if self.opts.headings => {
                self.captures
                    .push((depth, Capture::Heading(name.as_bytes()[1] - b'0'), String::new()));
            }
            "script" if self.opts.embedded && self.page.embedded.len() < self.opts.max_embedded => {
                let kind = el
                    .attr("type")
                    .map(|t| t.trim().to_ascii_lowercase())
                    .unwrap_or_default();
                let id = el.attr("id").map(str::to_owned);
                let json = kind == "application/json"
                    || kind.ends_with("+json") && !kind.contains("ld+json")
                    || id.as_deref() == Some("__NEXT_DATA__");
                let js =
                    kind.is_empty() || kind.contains("javascript") || kind == "module" || kind.contains("ecmascript");
                if json && !kind.contains("ld+json") {
                    self.captures
                        .push((depth, Capture::Script(Some(id.unwrap_or(kind))), String::new()));
                } else if js && el.attr("src").is_none() {
                    self.captures.push((depth, Capture::Script(None), String::new()));
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        if let Some(t) = &mut self.text {
            t.close(depth, tree.frames[depth].el.kind, self.opts.max_text_bytes);
        }
        if self.svg_depth == Some(depth) {
            self.svg_depth = None;
        }
        if self.open_link.is_some_and(|(d, _)| d == depth) {
            self.open_link = None;
        }
        while self.captures.last().is_some_and(|(d, _, _)| *d >= depth) {
            if let Some((_, c, t)) = self.captures.pop() {
                self.close_capture(c, t);
            }
        }
    }

    fn text(&mut self, _: &Tree, text: &Text<'_>) {
        let t = text.get();
        if let Some(p) = self.text.as_mut().filter(|p| p.skip.is_none()) {
            let max = self.opts.max_text_bytes;
            // Its own line breaks are spaces: lines break where blocks do.
            if t.contains(['\n', '\r']) {
                p.push(&t.replace(['\n', '\r'], " "), max);
            } else {
                p.push(t, max);
            }
        }
        let max_script = self.opts.max_script_bytes;
        let max_text = self.opts.max_text * 8;
        let mut cut_script = false;
        for (_, c, s) in &mut self.captures {
            match c {
                Capture::Link(..) if s.len() < max_text => s.push_str(t),
                Capture::Script(_) => {
                    if s.len() + t.len() <= max_script {
                        s.push_str(t);
                    } else {
                        *c = Capture::Dropped;
                        *s = String::new();
                        cut_script = true;
                    }
                }
                Capture::Title | Capture::Heading(_) if s.len() < max_text => s.push_str(t),
                _ => {}
            }
        }
        if cut_script {
            self.warn("page: a script past max_script_bytes was not read");
        }
    }

    fn markup(&mut self, _: &Tree, _: &[u8]) {}
}

fn is_ics(href: &str) -> bool {
    let h = href.trim().to_ascii_lowercase();
    let path = h.split(['?', '#']).next().unwrap_or("");
    path.ends_with(".ics") || h.contains("?ical=") || h.contains("&ical=") || h.contains("?ics=") || h.contains("&ics=")
}

fn is_empty_json(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Null => true,
        _ => false,
    }
}

/// The largest candidate of a `srcset` (`a.jpg 480w, b.jpg 1080w` → `b.jpg`), read as the HTML
/// standard reads it: a URL runs to whitespace (a `data:` URL's commas are its own), then a
/// descriptor to the next comma.
fn largest(srcset: &str) -> Option<String> {
    let b = srcset.as_bytes();
    let mut best: Option<(f64, &str)> = None;
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut url = &srcset[start..i];
        // A URL directly followed by a comma has no descriptor.
        let bare = url.ends_with(',');
        url = url.trim_end_matches(',');
        let d0 = i;
        if !bare {
            while i < b.len() && b[i] != b',' {
                i += 1;
            }
        }
        let descriptor = srcset[d0..i].trim();
        let size = descriptor
            .trim_end_matches(['w', 'x', 'W', 'X', 'h', 'H'])
            .parse::<f64>()
            .unwrap_or(1.0);
        if !url.is_empty() && !url.starts_with("data:") && best.is_none_or(|(s, _)| size > s) {
            best = Some((size, url));
        }
    }
    best.map(|(_, u)| u.to_owned())
}

/// `5; url=/next`, `0;URL='x'`, `3`.
fn refresh(content: &str, pass: &PagePass) -> Option<Refresh> {
    let content = content.trim();
    let (secs, rest) = match content.find([';', ',']) {
        Some(i) => (&content[..i], &content[i + 1..]),
        None => (content, ""),
    };
    let seconds: f64 = secs
        .trim()
        .trim_end_matches('.')
        .parse()
        .ok()
        .filter(|s: &f64| s.is_finite() && *s >= 0.0)?;
    let rest = rest.trim();
    let target = match rest.get(..4) {
        Some(p) if p.eq_ignore_ascii_case("url=") => &rest[4..],
        _ => match rest.get(..3) {
            Some(p) if p.eq_ignore_ascii_case("url") => rest[3..].trim_start().trim_start_matches('='),
            _ => rest,
        },
    };
    let target = target.trim().trim_matches(['"', '\'']);
    Some(Refresh {
        seconds,
        url: (!target.is_empty()).then(|| pass.absolute(target)).flatten(),
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 {
            if let Some(v) = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// JSON a script assigns: `window.__INITIAL_STATE__ = {…}`, `window["x"] = […]`,
/// `self.__NUXT__ = {…}`, `var __APOLLO_STATE__ = {…}`, `… = JSON.parse('{…}')`.
pub fn assignments(js: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let b = js.as_bytes();
    let mut i = 0;
    while let Some(eq) = memchr::memchr(b'=', &b[i..]).map(|k| i + k) {
        i = eq + 1;
        // `==`, `=>`, `<=`, `!=` are no assignment.
        if b.get(eq + 1).is_some_and(|&c| c == b'=' || c == b'>')
            || (eq > 0
                && matches!(
                    b[eq - 1],
                    b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'|' | b'&'
                ))
        {
            continue;
        }
        let Some(name) = assigned_name(&js[..eq]) else { continue };
        let mut v = eq + 1;
        while v < b.len() && b[v].is_ascii_whitespace() {
            v += 1;
        }
        let value = if js[v..].starts_with("JSON.parse(") {
            let start = v + "JSON.parse(".len();
            js_string(&js[start..]).and_then(|s| lenient::read(&s).values.into_iter().next())
        } else if matches!(b.get(v), Some(b'{' | b'[')) {
            balanced(&js[v..]).and_then(|s| {
                let read = lenient::read(s);
                read.values.into_iter().next()
            })
        } else {
            None
        };
        if let Some(value) = value.filter(|v| !is_empty_json(v) && (v.is_object() || v.is_array())) {
            out.push((name, value));
            if out.len() >= 32 {
                break;
            }
        }
    }
    out
}

/// The name before an `=`, when it is state a page keeps: a global (`window.x`, `self["x"]`,
/// `globalThis.x`), or a variable whose name says so (`__X__`, `…State`, `…Data`, `…Props`).
fn assigned_name(before: &str) -> Option<String> {
    let t = before.trim_end();
    // `window["__X__"]`.
    if let Some(inner) = t.strip_suffix(']') {
        let open = inner.rfind('[')?;
        let key = inner[open + 1..].trim().trim_matches(['"', '\'', '`']);
        let obj = inner[..open].trim_end();
        let obj = obj
            .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
            .next()?;
        return matches!(obj, "window" | "self" | "globalThis").then(|| format!("{obj}.{key}"));
    }
    let ident_start = t
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.'))
        .map_or(0, |k| k + 1);
    let path = &t[ident_start..];
    if path.is_empty() || path.starts_with('.') || path.ends_with('.') {
        return None;
    }
    let before_path = t[..ident_start].trim_end();
    let last = path.rsplit('.').next()?;
    let global = ["window.", "self.", "globalThis."].iter().any(|g| path.starts_with(g));
    let declared = ["var", "let", "const"].iter().any(|k| before_path.ends_with(k)) && !path.contains('.');
    let stateful = (last.starts_with("__") && last.len() > 4)
        || [
            "state", "data", "props", "store", "apollo", "nuxt", "next", "redux", "initial", "preload", "config",
            "event",
        ]
        .iter()
        .any(|w| last.to_ascii_lowercase().contains(w));
    ((global && !last.is_empty()) || (declared && stateful)).then(|| path.to_owned())
}

/// The text of a balanced `{…}` or `[…]` (strings and comments skipped over), or none.
fn balanced(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A JavaScript string literal's value (`'{\"a\":1}'` → `{"a":1}`).
fn js_string(s: &str) -> Option<String> {
    let s = s.trim_start();
    let q = s.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut out = String::new();
    let mut chars = s[1..].chars();
    while let Some(c) = chars.next() {
        if c == q {
            return Some(out);
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                out.push(
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .unwrap_or('\u{fffd}'),
                );
            }
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .unwrap_or('\u{fffd}'),
                );
            }
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_state() {
        let js = r#"
            var x = 1; if (a == b) {}
            window.__INITIAL_STATE__ = {"events":[{"name":"Nora"}]};
            window["__APOLLO_STATE__"] = {ROOT_QUERY: {a: 1}};
            self.__NUXT__=(function(a){return {}})(1);
            const pageData = JSON.parse('{\"id\":7,\"t\":\"caf\\u00e9\"}');
            var handlers = {click: 1};
            let eventsConfig = [1,2,3] // trailing
        "#;
        let got: Vec<(String, Value)> = assignments(js);
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "window.__INITIAL_STATE__",
                "window.__APOLLO_STATE__",
                "pageData",
                "eventsConfig"
            ]
        );
        assert_eq!(got[0].1["events"][0]["name"], "Nora");
        assert_eq!(got[1].1["ROOT_QUERY"]["a"], 1);
        assert_eq!(got[2].1["t"], "café");
    }

    #[test]
    fn srcsets_and_refresh() {
        assert_eq!(largest("a.jpg 480w, b.jpg 1080w, c.jpg 720w").as_deref(), Some("b.jpg"));
        assert_eq!(largest("a.jpg 1x, b.jpg 2x").as_deref(), Some("b.jpg"));
        assert_eq!(largest("data:image/gif;base64,R0 1x").as_deref(), None);
        let p = PagePass::new(PageOptions::default(), Url::parse("https://v.example/a/").ok());
        let r = refresh("0; URL='/next?x=1'", &p).unwrap();
        assert_eq!((r.seconds, r.url.as_deref()), (0.0, Some("https://v.example/next?x=1")));
        assert_eq!(refresh("5", &p).unwrap().url, None);
        assert_eq!(
            refresh("3;url=b", &p).unwrap().url.as_deref(),
            Some("https://v.example/a/b")
        );
        assert!(refresh("soon", &p).is_none());
    }
}
