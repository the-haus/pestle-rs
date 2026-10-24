//! Sitemaps (sitemaps.org) read as their bytes arrive: XML (`<urlset>`,
//! `<sitemapindex>`, any namespace prefix), text (one URL a line), and the
//! feeds sitemaps.org allows in their place (RSS 2.0, RSS 1.0, Atom). Gzip
//! is undone on the way (by its magic bytes, whatever the name says), with a
//! limit on what it may grow to; the charset is the file's (UTF-8, or what
//! its BOM or XML declaration says).
//!
//! Only the entry being read is held. A `<image:loc>` or `<xhtml:link>` in a
//! `<url>` is never taken for its `<loc>`: images and language versions are
//! given apart. URLs are absolute http(s) without `#…` (relative ones
//! resolve against the sitemap's own URL, when given); a `<lastmod>` that is
//! no date, a `<changefreq>` that is none of the seven, a `<priority>`
//! outside 0–1 are left out. A body that is no sitemap (an HTML page, a soft
//! 404) is an error, [`NotASitemap`]; one broken partway keeps what it gave.
//!
//! ```
//! use pestle_html::sitemap::SitemapReader;
//! let mut r = SitemapReader::new(None);
//! r.write(br#"<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
//!   <url><loc>https://venue.example/e/nora</loc><lastmod>2026-11-01</lastmod></url></urlset>"#).unwrap();
//! let done = r.end().unwrap();
//! assert_eq!(done.entries[0].url, "https://venue.example/e/nora");
//! ```

use std::io::Write;

use serde::Serialize;
use url::Url;

use crate::charset::Transcoder;

/// A body that is no sitemap, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotASitemap(pub String);

impl std::fmt::Display for NotASitemap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotASitemap {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A page (`<url>`, a feed's item).
    Url,
    /// A sitemap an index names (`<sitemap>`).
    Sitemap,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Alternate {
    pub hreflang: String,
    pub href: String,
}

/// One entry of a sitemap.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    pub kind: Kind,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastmod: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changefreq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    /// `<image:loc>`s.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// `<xhtml:link rel="alternate" hreflang>`s.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<Alternate>,
    /// A feed item's or a news entry's title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// What kind of file it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Urlset,
    Index,
    Text,
    Rss,
    Atom,
}

/// How much a sitemap may be.
#[derive(Clone, Copy, Debug)]
pub struct SitemapOptions {
    /// An entry longer than this without its end is skipped.
    pub max_entry_bytes: usize,
    /// How far into the body its root (`<urlset>` …) must be.
    pub max_prolog_bytes: usize,
    /// The most a body may be once gunzipped (a gzip bomb stops here).
    pub max_bytes: usize,
    /// At most this many entries (sitemaps.org: 50,000 a file).
    pub max_entries: usize,
}

impl Default for SitemapOptions {
    fn default() -> Self {
        SitemapOptions {
            max_entry_bytes: 64 * 1024,
            max_prolog_bytes: 16 * 1024,
            max_bytes: 256 << 20,
            max_entries: 1_000_000,
        }
    }
}

/// A sitemap read to its end.
#[derive(Clone, Debug, Serialize)]
pub struct Sitemap {
    pub format: Format,
    /// The entries not yet taken ([`SitemapReader::take`]).
    pub entries: Vec<Entry>,
    /// It was gzipped.
    pub gzip: bool,
    /// What was cut or skipped (`a gzip stream broken at …`, `entries past max_entries`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

enum Input {
    /// Not yet known whether it is gzipped.
    Sniff(Vec<u8>),
    Plain,
    Gzip(Box<flate2::write::MultiGzDecoder<Vec<u8>>>),
    /// Stopped: a broken gzip stream, or past `max_bytes`.
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Xml,
    Text,
}

/// A sitemap read chunk by chunk.
pub struct SitemapReader {
    opts: SitemapOptions,
    base: Option<Url>,
    input: Input,
    transcoder: Transcoder,
    gzip: bool,
    bytes: usize,
    buf: String,
    mode: Option<Mode>,
    format: Option<Format>,
    /// The root's namespace prefix (`s:` in `<s:urlset>`), and the entry tag (`url`, `sitemap`, `item`, `entry`).
    prefix: String,
    tag: &'static str,
    entries: Vec<Entry>,
    /// The first bytes of a character cut at a chunk's end.
    carry: Vec<u8>,
    acc: Acc,
    count: usize,
    warnings: Vec<String>,
    error: Option<NotASitemap>,
}

impl SitemapReader {
    /// `base`: the sitemap's own URL (relative `<loc>`s resolve against it).
    pub fn new(base: Option<&str>) -> SitemapReader {
        SitemapReader::with_options(base, SitemapOptions::default())
    }

    pub fn with_options(base: Option<&str>, opts: SitemapOptions) -> SitemapReader {
        SitemapReader {
            opts,
            base: base.and_then(|b| Url::parse(b).ok()),
            input: Input::Sniff(Vec::new()),
            transcoder: Transcoder::new(None),
            gzip: false,
            bytes: 0,
            buf: String::new(),
            mode: None,
            format: None,
            prefix: String::new(),
            tag: "url",
            entries: Vec::new(),
            carry: Vec::new(),
            acc: Acc::default(),
            count: 0,
            warnings: Vec::new(),
            error: None,
        }
    }

    /// The next bytes of the body (gzipped or not, any size, cut anywhere).
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), NotASitemap> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let mut plain: Vec<u8> = Vec::new();
        match &mut self.input {
            Input::Sniff(head) => {
                head.extend_from_slice(chunk);
                if head.len() < 2 {
                    return Ok(());
                }
                let head = std::mem::take(head);
                if head[0] == 0x1f && head[1] == 0x8b {
                    self.gzip = true;
                    self.input = Input::Gzip(Box::new(flate2::write::MultiGzDecoder::new(Vec::new())));
                    self.gunzip(&head, &mut plain);
                } else {
                    self.input = Input::Plain;
                    plain = head;
                }
            }
            // Plain bytes go straight on.
            Input::Plain => return self.feed(chunk, false),
            Input::Gzip(_) => self.gunzip(chunk, &mut plain),
            Input::Stopped => return Ok(()),
        }
        self.feed(&plain, false)
    }

    fn gunzip(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        let Input::Gzip(d) = &mut self.input else { return };
        if let Err(e) = d.write_all(chunk) {
            self.warnings.push(format!("a gzip stream broken: {e}"));
            let _ = d.flush();
            out.append(d.get_mut());
            self.input = Input::Stopped;
            return;
        }
        out.append(d.get_mut());
    }

    /// Decoded bytes into the text buffer, and the entries they complete.
    fn feed(&mut self, bytes: &[u8], end: bool) -> Result<(), NotASitemap> {
        let room = self.opts.max_bytes.saturating_sub(self.bytes);
        let bytes = if bytes.len() > room {
            self.warnings
                .push(format!("the body was cut at max_bytes ({})", self.opts.max_bytes));
            self.input = Input::Stopped;
            &bytes[..room]
        } else {
            bytes
        };
        self.bytes += bytes.len();
        let buf = &mut self.buf;
        let carry = &mut self.carry;
        // UTF-8 passed through may be cut inside a character: its first bytes wait for the rest.
        let mut push = |b: &[u8]| {
            let joined;
            let b = if carry.is_empty() {
                b
            } else {
                carry.extend_from_slice(b);
                joined = std::mem::take(carry);
                &joined[..]
            };
            match std::str::from_utf8(b) {
                Ok(t) => buf.push_str(t),
                Err(e) if e.error_len().is_none() => {
                    let (whole, cut) = b.split_at(e.valid_up_to());
                    buf.push_str(std::str::from_utf8(whole).unwrap_or_default());
                    carry.extend_from_slice(cut);
                }
                // Broken UTF-8: replaced, as a reader would.
                Err(_) => buf.push_str(&String::from_utf8_lossy(b)),
            }
        };
        self.transcoder.write(bytes, &mut push);
        if end {
            self.transcoder.end(&mut push);
            if !self.carry.is_empty() {
                self.buf
                    .push_str(&String::from_utf8_lossy(&std::mem::take(&mut self.carry)));
            }
        }
        let r = self.drain(end);
        if let Err(e) = &r {
            self.error = Some(e.clone());
        }
        r
    }

    /// The entries completed since the last call.
    pub fn take(&mut self) -> Vec<Entry> {
        std::mem::take(&mut self.entries)
    }

    /// The end of the body.
    pub fn end(mut self) -> Result<Sitemap, NotASitemap> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        let mut plain = Vec::new();
        match std::mem::replace(&mut self.input, Input::Stopped) {
            Input::Sniff(head) => plain = head,
            Input::Gzip(mut d) => {
                if let Err(e) = d.try_finish() {
                    self.warnings.push(format!("a gzip stream ended early: {e}"));
                }
                plain.append(d.get_mut());
            }
            Input::Plain | Input::Stopped => {}
        }
        self.feed(&plain, true)?;
        if self.mode.is_none() {
            return Err(NotASitemap("an empty body".into()));
        }
        let format = self.format.unwrap_or(Format::Text);
        Ok(Sitemap {
            format,
            entries: self.entries,
            gzip: self.gzip,
            warnings: self.warnings,
        })
    }

    fn drain(&mut self, end: bool) -> Result<(), NotASitemap> {
        if self.mode.is_none() {
            let start = self
                .buf
                .trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
            if start.is_empty() {
                self.buf.clear();
                return Ok(());
            }
            let skipped = self.buf.len() - start.len();
            self.buf.drain(..skipped);
            self.mode = Some(if self.buf.starts_with('<') {
                Mode::Xml
            } else {
                Mode::Text
            });
        }
        if self.mode == Some(Mode::Text) {
            self.format = Some(Format::Text);
            let keep_from = if end {
                self.buf.len()
            } else {
                self.buf.rfind(['\n', '\r']).map_or(0, |i| i + 1)
            };
            let lines: Vec<String> = self.buf[..keep_from].split(['\n', '\r']).map(str::to_owned).collect();
            self.buf.drain(..keep_from);
            for line in lines {
                if let Some(url) = self.url(line.trim()) {
                    self.push(Entry::new(Kind::Url, url));
                }
            }
            return Ok(());
        }
        if self.format.is_none() && !self.find_root(end)? {
            return Ok(());
        }
        self.scan(end);
        Ok(())
    }

    /// The root element, which says what the file is.
    fn find_root(&mut self, end: bool) -> Result<bool, NotASitemap> {
        let lower = self.buf.to_ascii_lowercase();
        let mut i = 0;
        while let Some(lt) = lower[i..].find('<').map(|k| i + k) {
            let rest = &lower[lt + 1..];
            if rest.starts_with('?') || rest.starts_with('!') && !rest.starts_with("!doctype html") {
                i = lt + 1;
                continue;
            }
            if rest.starts_with("html") || rest.starts_with("!doctype html") {
                return Err(NotASitemap("an HTML page, not a sitemap".into()));
            }
            let name: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
                .collect();
            if name.is_empty() || lt + 1 + name.len() >= lower.len() && !end {
                // Cut at the chunk's end: wait.
                break;
            }
            let (prefix, local) = match name.rfind(':') {
                Some(c) => (&name[..=c], &name[c + 1..]),
                None => ("", name.as_str()),
            };
            let found = match local {
                "urlset" => Some((Format::Urlset, "url", prefix)),
                "sitemapindex" => Some((Format::Index, "sitemap", prefix)),
                "rss" => Some((Format::Rss, "item", "")),
                "rdf" => Some((Format::Rss, "item", "")),
                "feed" => Some((Format::Atom, "entry", prefix)),
                _ => None,
            };
            match found {
                Some((format, tag, p)) => {
                    // The prefix as the file writes it (case kept).
                    self.prefix = self.buf[lt + 1..lt + 1 + p.len()].to_owned();
                    self.format = Some(format);
                    self.tag = tag;
                    let at = floor_boundary(&self.buf, lt);
                    self.buf.drain(..at);
                    return Ok(true);
                }
                None => {
                    return Err(NotASitemap(format!("<{name}>, not <urlset> or <sitemapindex>")));
                }
            }
        }
        if end || self.buf.len() > self.opts.max_prolog_bytes {
            return Err(NotASitemap("no <urlset> or <sitemapindex>".into()));
        }
        Ok(false)
    }

    fn push(&mut self, e: Entry) {
        if self.count >= self.opts.max_entries {
            if !self.warnings.iter().any(|w| w.starts_with("entries past")) {
                self.warnings.push(format!(
                    "entries past max_entries ({}) were left out",
                    self.opts.max_entries
                ));
            }
            return;
        }
        self.count += 1;
        self.entries.push(e);
    }

    /// [`SitemapReader::url`] for a string already owned: kept as it is when it is canonical.
    fn url_owned(&self, href: String) -> Option<String> {
        if href.len() == href.trim_ascii().len() && canonical_http(&href).is_some() {
            return Some(href);
        }
        self.url(&href)
    }

    /// `href` as a page address: absolute http(s), no `#…`.
    fn url(&self, href: &str) -> Option<String> {
        let href = href.trim_ascii();
        if href.is_empty() {
            return None;
        }
        if let Some(u) = canonical_http(href) {
            return Some(u.to_owned());
        }
        let mut u = match Url::parse(href) {
            Ok(u) => u,
            Err(url::ParseError::RelativeUrlWithoutBase) => self.base.as_ref()?.join(href).ok()?,
            Err(_) => return None,
        };
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(str::is_empty) {
            return None;
        }
        u.set_fragment(None);
        Some(u.to_string())
    }

    /// The tags of the buffer, one pass: an entry's fields are taken as their text arrives, and
    /// what is cut at the buffer's end (a tag, a field's text) waits for the next bytes.
    fn scan(&mut self, end: bool) {
        let buf = std::mem::take(&mut self.buf);
        let b = buf.as_bytes();
        let own = self.prefix.to_ascii_lowercase();
        let mut lowered = String::new();
        let mut at = 0;
        loop {
            let Some(lt) = memchr::memchr(b'<', &b[at..]).map(|k| at + k) else {
                at = b.len();
                break;
            };
            let rest = &buf[lt..];
            // Comments, CDATA outside a field, declarations: skipped whole.
            let skip_to = if rest.starts_with("<!--") {
                Some("-->")
            } else if rest.starts_with("<![CDATA[") {
                Some("]]>")
            } else {
                None
            };
            if let Some(close) = skip_to {
                match rest.find(close) {
                    Some(k) => {
                        at = lt + k + close.len();
                        continue;
                    }
                    None => {
                        at = if end { b.len() } else { lt };
                        break;
                    }
                }
            }
            let Some(gt) = memchr::memchr(b'>', &b[lt..]).map(|k| lt + k) else {
                at = if end { b.len() } else { lt };
                break;
            };
            let inner = &buf[lt + 1..gt];
            if inner.starts_with(['?', '!']) {
                at = gt + 1;
                continue;
            }
            let closing = inner.starts_with('/');
            let self_closing = inner.ends_with('/');
            let body = inner.trim_start_matches('/');
            let name_end = body
                .bytes()
                .position(|c| c.is_ascii_whitespace() || c == b'/')
                .unwrap_or(body.len());
            let raw_name = &body[..name_end];
            let attrs = &body[name_end..];
            let name: &str = if raw_name.bytes().any(|c| c.is_ascii_uppercase()) {
                lowered.clear();
                lowered.extend(raw_name.chars().map(|c| c.to_ascii_lowercase()));
                &lowered
            } else {
                raw_name
            };
            let colon = name.bytes().rposition(|c| c == b':').map_or(0, |i| i + 1);
            let (ns, local) = name.split_at(colon);
            let is_entry = ns == own && local == self.tag;
            if closing {
                if is_entry && self.acc.open {
                    self.finish_entry();
                }
                at = gt + 1;
                continue;
            }
            if is_entry {
                // An entry inside one not closed is part of it (its first values stay).
                if !self_closing && !self.acc.open {
                    self.acc = Acc {
                        open: true,
                        ..Acc::default()
                    };
                }
                at = gt + 1;
                continue;
            }
            if !self.acc.open {
                at = gt + 1;
                continue;
            }
            let format = self.format.unwrap_or(Format::Urlset);
            let field = field_of(format, ns, local, &own);
            match field {
                Field::None => {
                    at = gt + 1;
                }
                Field::AtomLink => {
                    let rel = attr_of(attrs, "rel")
                        .unwrap_or_else(|| "alternate".into())
                        .to_ascii_lowercase();
                    if rel == "alternate" && self.acc.loc.is_none() {
                        self.acc.loc = attr_of(attrs, "href");
                    }
                    at = gt + 1;
                }
                Field::Alternate => {
                    let rel = attr_of(attrs, "rel").unwrap_or_default().to_ascii_lowercase();
                    if let (true, Some(lang), Some(href)) =
                        (rel == "alternate", attr_of(attrs, "hreflang"), attr_of(attrs, "href"))
                    {
                        if self.acc.alternates.len() < 500 {
                            self.acc.alternates.push((lang, href));
                        }
                    }
                    at = gt + 1;
                }
                _ if self_closing => at = gt + 1,
                _ => {
                    let Some(text_end) = leaf_end(&buf, gt + 1) else {
                        // Its text is cut: read it with the next bytes (or not at all, at the end).
                        at = if end { b.len() } else { lt };
                        break;
                    };
                    let text = leaf_text(&buf[gt + 1..text_end]);
                    let a = &mut self.acc;
                    let first = |slot: &mut Option<String>, v: String| {
                        if slot.is_none() {
                            *slot = Some(v);
                        }
                    };
                    match field {
                        Field::Loc => first(&mut a.loc, text),
                        Field::Lastmod => first(&mut a.lastmod, text),
                        Field::Changefreq => first(&mut a.changefreq, text.to_ascii_lowercase()),
                        Field::Priority => first(&mut a.priority, text),
                        Field::Title => first(&mut a.title, text),
                        Field::Image => {
                            if a.images.len() < 1000 {
                                a.images.push(text);
                            }
                        }
                        Field::Guid => {
                            if attr_of(attrs, "ispermalink").is_none_or(|v| !v.eq_ignore_ascii_case("false")) {
                                first(&mut a.guid, text);
                            }
                        }
                        Field::None | Field::AtomLink | Field::Alternate => {}
                    }
                    at = text_end;
                }
            }
        }
        let at = floor_boundary(&buf, at);
        if end {
            if self.acc.open {
                self.finish_entry();
            }
        } else if buf.len() - at > self.opts.max_entry_bytes {
            // A tag or a field that never ends: dropped, not held.
            self.warnings.push("an entry without its end was skipped".into());
            self.acc = Acc::default();
        } else {
            self.buf = buf[at..].to_owned();
        }
    }

    /// The entry read so far, out.
    fn finish_entry(&mut self) {
        let a = std::mem::take(&mut self.acc);
        let format = self.format.unwrap_or(Format::Urlset);
        let Some(url) = a.loc.or(a.guid).and_then(|l| self.url_owned(l)) else {
            return;
        };
        let kind = if format == Format::Index {
            Kind::Sitemap
        } else {
            Kind::Url
        };
        let mut e = Entry::new(kind, url);
        e.lastmod = a.lastmod.filter(|l| is_date(l));
        if kind == Kind::Url {
            e.changefreq = a.changefreq.filter(|c| {
                matches!(
                    c.as_str(),
                    "always" | "hourly" | "daily" | "weekly" | "monthly" | "yearly" | "never"
                )
            });
            e.priority = a
                .priority
                .and_then(|p| p.parse::<f64>().ok())
                .filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
            e.images = a.images.into_iter().filter_map(|i| self.url_owned(i)).collect();
            e.alternates = a
                .alternates
                .into_iter()
                .filter_map(|(hreflang, href)| self.url_owned(href).map(|href| Alternate { hreflang, href }))
                .collect();
            e.title = a.title.filter(|t| !t.is_empty());
        }
        self.push(e);
    }
}

/// An entry being read.
#[derive(Default)]
struct Acc {
    open: bool,
    loc: Option<String>,
    guid: Option<String>,
    lastmod: Option<String>,
    changefreq: Option<String>,
    priority: Option<String>,
    title: Option<String>,
    images: Vec<String>,
    alternates: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    Loc,
    Lastmod,
    Changefreq,
    Priority,
    Title,
    Image,
    Guid,
    /// Atom's `<link href>`.
    AtomLink,
    /// `<xhtml:link rel="alternate" hreflang href>`.
    Alternate,
}

/// What a tag inside an entry is, by the file's format and its name.
fn field_of(format: Format, ns: &str, local: &str, own: &str) -> Field {
    match format {
        Format::Urlset | Format::Index if ns == own => match local {
            "loc" => Field::Loc,
            "lastmod" => Field::Lastmod,
            "changefreq" => Field::Changefreq,
            "priority" => Field::Priority,
            _ => Field::None,
        },
        Format::Urlset => match local {
            "loc" if has(ns, b"image") => Field::Image,
            "link" => Field::Alternate,
            "title" if has(ns, b"news") => Field::Title,
            "publication_date" if has(ns, b"news") => Field::Lastmod,
            _ => Field::None,
        },
        Format::Rss => match (ns, local) {
            ("", "link") => Field::Loc,
            ("", "guid") => Field::Guid,
            ("", "pubdate") | ("dc:", "date") | ("", "updated") => Field::Lastmod,
            ("", "title") => Field::Title,
            _ => Field::None,
        },
        Format::Atom if ns == own => match local {
            "link" => Field::AtomLink,
            "updated" | "published" => Field::Lastmod,
            "title" => Field::Title,
            _ => Field::None,
        },
        _ => Field::None,
    }
}

/// Whether a (short) namespace prefix has `word` in it.
fn has(ns: &str, word: &[u8]) -> bool {
    ns.as_bytes().windows(word.len()).any(|w| w == word)
}

/// Where a leaf's text (from `from`) ends: at the next tag, CDATA sections inside it kept;
/// none when it is cut at the end of the buffer.
fn leaf_end(buf: &str, from: usize) -> Option<usize> {
    let b = buf.as_bytes();
    let mut i = from;
    loop {
        let at = i + memchr::memchr(b'<', &b[i..])?;
        if buf[at..].starts_with("<![CDATA[") {
            i = at + buf[at..].find("]]>")? + 3;
            continue;
        }
        if buf.len() - at < 9 && "<![CDATA[".starts_with(&buf[at..]) {
            // `<![CD` cut at the end: wait.
            return None;
        }
        return Some(at);
    }
}

/// A leaf's text: CDATA unwrapped, references decoded, trimmed.
fn leaf_text(raw: &str) -> String {
    if !raw.bytes().any(|c| c == b'&' || c == b'<') {
        return raw.trim_ascii().to_owned();
    }
    let mut out = String::new();
    if !raw.contains('<') {
        decode(raw, &mut out);
        return out.trim_ascii().to_owned();
    }
    let mut s = raw;
    while let Some(i) = s.find("<![CDATA[") {
        decode(&s[..i], &mut out);
        let after = &s[i + 9..];
        match after.find("]]>") {
            Some(j) => {
                out.push_str(&after[..j]);
                s = &after[j + 3..];
            }
            None => {
                out.push_str(after);
                s = "";
            }
        }
    }
    decode(s, &mut out);
    out.trim().to_owned()
}

/// An attribute's value, from the text after a tag's name.
fn attr_of(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|k| from + k) {
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = lower[i + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let off = attrs.len() - rest.len() + 1;
            let v = attrs[off..].trim_start();
            let value = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
                _ => v
                    .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .next()
                    .unwrap_or(""),
            };
            let mut out = String::new();
            decode(value, &mut out);
            return Some(out);
        }
        from = i + name.len();
    }
    None
}

impl Entry {
    fn new(kind: Kind, url: String) -> Entry {
        Entry {
            kind,
            url,
            lastmod: None,
            changefreq: None,
            priority: None,
            images: Vec::new(),
            alternates: Vec::new(),
            title: None,
        }
    }
}

/// `href` when it is already as the URL standard would write it (lowercase `http(s)://`, a
/// plain lowercase host without a port, a path, a query of characters it keeps as they are,
/// no fragment, no dot segments): most sitemap URLs, read without parsing.
fn canonical_http(href: &str) -> Option<&str> {
    let rest = href.strip_prefix("https://").or_else(|| href.strip_prefix("http://"))?;
    let b = rest.as_bytes();
    let host_end = b.iter().position(|&c| c == b'/')?;
    let host = &b[..host_end];
    if host.is_empty()
        || host[0] == b'.'
        || host[host.len() - 1] == b'.'
        || host.windows(2).any(|w| w == b"..")
        || !host.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        // `xn--` labels, all-digit hosts (IPv4 forms) and the like: parsed.
        || host.starts_with(b"xn--")
        || host.windows(5).any(|w| w == b".xn--")
        || host.iter().all(|c| c.is_ascii_digit() || *c == b'.')
    {
        return None;
    }
    let tail = &b[host_end..];
    let query_at = tail.iter().position(|&c| c == b'?').unwrap_or(tail.len());
    let (path, query) = tail.split_at(query_at);
    // 1: kept as is in a path and a query; 2: in a path only (`'`); 3: in a query only (`?`); 4: `%`.
    const KEPT: [u8; 256] = {
        let mut t = [0u8; 256];
        let mut c = 0;
        while c < 256 {
            let b = c as u8;
            if b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                        | b'/'
                )
            {
                t[c] = 1;
            }
            c += 1;
        }
        t[b'\'' as usize] = 2;
        t[b'?' as usize] = 3;
        t[b'%' as usize] = 4;
        t
    };
    let plain = |c: u8, in_query: bool| {
        let k = KEPT[c as usize];
        k == 1 || k == 4 || (k == 2 && !in_query) || (k == 3 && in_query)
    };
    let escapes_ok = |s: &[u8]| {
        s.iter().enumerate().all(|(i, &c)| {
            c != b'%'
                || (s.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && s.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
        })
    };
    let path_ok = path.iter().all(|&c| plain(c, false))
        && escapes_ok(path)
        && !path.windows(3).any(|w| w == b"/./" || w == b"/..")
        && !path.ends_with(b"/.")
        && !path.windows(4).any(|w| w.eq_ignore_ascii_case(b"%2e") || w == b"/%2e");
    let query_ok = query.iter().all(|&c| plain(c, true)) && escapes_ok(query);
    (path_ok && query_ok).then_some(href)
}

fn decode(s: &str, out: &mut String) {
    out.reserve(s.len());
    crate::decode::decode_into_valid(s.as_bytes(), false, out, true);
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A W3C datetime (`2026`, `2026-11`, `2026-11-14`, `2026-11-14T20:00:00+01:00`, `… 20:00`),
/// or an RFC 822 date as feeds write them (`Sat, 14 Nov 2026 20:00:00 +0100`).
pub fn is_date(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    if digits(0..4) {
        let month_ok = |m: &[u8]| {
            std::str::from_utf8(m)
                .ok()
                .and_then(|m| m.parse::<u32>().ok())
                .is_some_and(|m| (1..=12).contains(&m))
        };
        let day_ok = |d: &[u8]| {
            std::str::from_utf8(d)
                .ok()
                .and_then(|d| d.parse::<u32>().ok())
                .is_some_and(|d| (1..=31).contains(&d))
        };
        return match b.len() {
            4 => true,
            7 => b[4] == b'-' && digits(5..7) && month_ok(&b[5..7]),
            _ => {
                b.len() >= 10
                    && b[4] == b'-'
                    && b[7] == b'-'
                    && digits(5..7)
                    && digits(8..10)
                    && month_ok(&b[5..7])
                    && day_ok(&b[8..10])
                    && (b.len() == 10 || matches!(b[10], b'T' | b't' | b' ') && digits(11..13))
            }
        };
    }
    // RFC 822: `[Day, ]DD Mon YYYY`.
    let rest = s.split_once(',').map_or(s, |(_, r)| r).trim();
    let mut parts = rest.split_whitespace();
    let day = parts.next().and_then(|d| d.parse::<u32>().ok());
    let month = parts.next().map(|m| m.to_ascii_lowercase());
    let year = parts.next().and_then(|y| y.parse::<u32>().ok());
    matches!((day, month.as_deref(), year), (Some(1..=31), Some("jan" | "feb" | "mar" | "apr" | "may" | "jun" | "jul" | "aug" | "sep" | "oct" | "nov" | "dec"), Some(y)) if y >= 1970)
}

/// A whole sitemap at once.
pub fn read(bytes: &[u8], base: Option<&str>) -> Result<Sitemap, NotASitemap> {
    let mut r = SitemapReader::new(base);
    r.write(bytes)?;
    r.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(s: &Sitemap) -> Vec<&str> {
        s.entries.iter().map(|e| e.url.as_str()).collect()
    }

    #[test]
    fn dates() {
        for d in [
            "2026",
            "2026-11",
            "2026-11-14",
            "2026-11-14T20:00:00+01:00",
            "2026-11-14 20:00",
            "Sat, 14 Nov 2026 20:00:00 +0100",
            "14 Nov 2026",
        ] {
            assert!(is_date(d), "{d}");
        }
        for d in [
            "",
            "yesterday",
            "2026-13-01",
            "2026-11-32",
            "14/11/2026",
            "Nov 2026",
            "0000-00",
        ] {
            assert!(!is_date(d), "{d}");
        }
    }

    #[test]
    fn forms() {
        let s = read(br#"<?xml version="1.0" encoding="UTF-8"?>
            <sm:urlset xmlns:sm="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1" xmlns:xhtml="http://www.w3.org/1999/xhtml">
              <sm:url><sm:loc> https://v.example/e/1#x </sm:loc><sm:lastmod>2026-11-01</sm:lastmod><sm:changefreq>Weekly</sm:changefreq><sm:priority>0.8</sm:priority>
                <image:image><image:loc>https://cdn.example/1.jpg</image:loc></image:image>
                <xhtml:link rel="alternate" hreflang="en" href="https://v.example/en/e/1"/></sm:url>
              <sm:url><sm:loc><![CDATA[https://v.example/e/2?a=1&b=2]]></sm:loc><sm:lastmod>never</sm:lastmod><sm:priority>7</sm:priority></sm:url>
              <sm:url><sm:loc>https://v.example/e/3?a=1&amp;b=2</sm:loc><sm:changefreq>sometimes</sm:changefreq></sm:url>
              <sm:url><sm:loc>javascript:alert(1)</sm:loc></sm:url><sm:url/>
            </sm:urlset>"#, None)
        .unwrap();
        assert_eq!(s.format, Format::Urlset);
        assert_eq!(
            urls(&s),
            [
                "https://v.example/e/1",
                "https://v.example/e/2?a=1&b=2",
                "https://v.example/e/3?a=1&b=2"
            ]
        );
        let e = &s.entries[0];
        assert_eq!(
            (e.lastmod.as_deref(), e.changefreq.as_deref(), e.priority),
            (Some("2026-11-01"), Some("weekly"), Some(0.8))
        );
        assert_eq!(e.images, ["https://cdn.example/1.jpg"]);
        assert_eq!(
            e.alternates,
            [Alternate {
                hreflang: "en".into(),
                href: "https://v.example/en/e/1".into()
            }]
        );
        assert_eq!((s.entries[1].lastmod.as_deref(), s.entries[1].priority), (None, None));
        assert_eq!(s.entries[2].changefreq, None);

        let idx = read(b"<sitemapindex><sitemap><loc>/sitemap-events.xml</loc><lastmod>2026-10-01T00:00:00Z</lastmod></sitemap></sitemapindex>", Some("https://v.example/sitemap.xml")).unwrap();
        assert_eq!(idx.format, Format::Index);
        assert_eq!(idx.entries[0].kind, Kind::Sitemap);
        assert_eq!(idx.entries[0].url, "https://v.example/sitemap-events.xml");

        let text = read(
            b"\xef\xbb\xbfhttps://v.example/a\r\n\r\nnot a url\nhttps://v.example/b",
            None,
        )
        .unwrap();
        assert_eq!(
            (text.format, urls(&text)),
            (Format::Text, vec!["https://v.example/a", "https://v.example/b"])
        );
    }

    #[test]
    fn feeds_as_sitemaps() {
        let rss = read(br#"<?xml version="1.0"?><rss version="2.0"><channel><title>Agenda</title><link>https://v.example/</link>
            <item><title>Nora</title><link>https://v.example/e/nora</link><pubDate>Sat, 14 Nov 2026 20:00:00 +0100</pubDate></item>
            <item><title>Guid only</title><guid>https://v.example/e/g</guid></item>
            <item><guid isPermaLink="false">abc</guid></item></channel></rss>"#, None)
        .unwrap();
        assert_eq!(rss.format, Format::Rss);
        assert_eq!(urls(&rss), ["https://v.example/e/nora", "https://v.example/e/g"]);
        assert_eq!(rss.entries[0].title.as_deref(), Some("Nora"));
        assert_eq!(
            rss.entries[0].lastmod.as_deref(),
            Some("Sat, 14 Nov 2026 20:00:00 +0100")
        );
        let atom = read(br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><link href="https://v.example/"/>
            <entry><title>Mira</title><link rel="self" href="https://v.example/self"/><link href="https://v.example/e/mira"/><updated>2026-11-15T21:00:00Z</updated></entry></feed>"#, None)
        .unwrap();
        assert_eq!(
            (atom.format, urls(&atom)),
            (Format::Atom, vec!["https://v.example/e/mira"])
        );
        assert_eq!(atom.entries[0].lastmod.as_deref(), Some("2026-11-15T21:00:00Z"));
    }

    #[test]
    fn not_sitemaps() {
        assert!(read(b"<!DOCTYPE html><html><body>404</body></html>", None).is_err());
        assert!(read(b"", None).is_err());
        assert!(read(b"   \n ", None).is_err());
        assert!(read(b"<note><to>x</to></note>", None).is_err());
        assert!(read(
            format!("<?xml version=\"1.0\"?>{}", "<!-- x -->".repeat(3000)).as_bytes(),
            None
        )
        .is_err());
    }

    #[test]
    fn streamed_gzipped_and_broken() {
        let mut xml =
            String::from(r#"<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#);
        for i in 0..2000 {
            xml.push_str(&format!(
                "<url><loc>https://v.example/e/{i}</loc><lastmod>2026-11-01</lastmod></url>\n"
            ));
        }
        xml.push_str("</urlset>");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(xml.as_bytes()).unwrap();
        let gz = gz.finish().unwrap();
        for (bytes, gzip) in [(xml.as_bytes().to_vec(), false), (gz.clone(), true)] {
            for size in [1, 7, 333, 100_000] {
                let mut r = SitemapReader::new(None);
                let mut got = Vec::new();
                for c in bytes.chunks(size) {
                    r.write(c).unwrap();
                    got.extend(r.take());
                }
                let done = r.end().unwrap();
                got.extend(done.entries);
                assert_eq!(got.len(), 2000, "size {size}, gzip {gzip}");
                assert_eq!(got[1999].url, "https://v.example/e/1999");
                assert_eq!(done.gzip, gzip);
            }
        }
        // Cut short: what was read is kept, the cut entry too when it has its loc.
        let cut = read(&xml.as_bytes()[..xml.len() / 2], None).unwrap();
        assert!(cut.entries.len() >= 990);
        // A gzip stream broken in the middle: what it gave, and a warning.
        let mut broken = gz[..gz.len() / 2].to_vec();
        broken.extend_from_slice(b"garbage that is no deflate");
        let b = read(&broken, None).unwrap();
        assert!(!b.entries.is_empty() && !b.warnings.is_empty());
        // A gzip bomb stops at max_bytes.
        let mut bomb = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        bomb.write_all(b"<urlset>").unwrap();
        for _ in 0..(64 << 20) / 4096 {
            bomb.write_all(&[b' '; 4096]).unwrap();
        }
        let bomb = bomb.finish().unwrap();
        let mut r = SitemapReader::with_options(
            None,
            SitemapOptions {
                max_bytes: 8 << 20,
                ..Default::default()
            },
        );
        r.write(&bomb).unwrap();
        let done = r.end().unwrap();
        assert!(done.warnings.iter().any(|w| w.contains("max_bytes")));
    }

    #[test]
    fn characters_cut_between_chunks() {
        let xml = "<urlset><url><loc>https://v.example/café-ö-東京</loc></url></urlset>";
        for size in 1..8 {
            let mut r = SitemapReader::new(None);
            for c in xml.as_bytes().chunks(size) {
                r.write(c).unwrap();
            }
            let done = r.end().unwrap();
            assert_eq!(
                done.entries[0].url, "https://v.example/caf%C3%A9-%C3%B6-%E6%9D%B1%E4%BA%AC",
                "chunks of {size}"
            );
        }
    }

    #[test]
    fn other_charsets() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(r#"<?xml version="1.0" encoding="windows-1252"?><urlset><url><loc>https://v.example/café</loc></url></urlset>"#);
        let s = read(&bytes, None).unwrap();
        assert_eq!(s.entries[0].url, "https://v.example/caf%C3%A9");
    }
}
