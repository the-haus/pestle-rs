//! Streaming HTML extraction: CSS selectors matched over a token stream, no
//! DOM, values read as text, attributes or markup and shaped by regexes,
//! JSON or pest grammars.
//!
//! The document is read once, in chunks of any size, with htmlparser2's
//! rules (the tree css-select queries), and the memory it takes is the
//! stack of open elements plus what the spec captures. In the same pass it
//! can also fingerprint the page's template ([`SkeletonOptions`]), strip
//! the page down for a model's prompt ([`StripOptions`]) and take its
//! accessibility snapshot ([`aria`]).
//!
//! ```
//! use pestle_html::{Extractor, ExtractorOptions};
//! use serde_json::json;
//!
//! let spec = json!({
//!     "title": "h1",
//!     "acts": ["ul.lineup > li"],
//!     "tickets": { "select": "a.tickets", "attr": "href", "url": "http" },
//! });
//! let x = Extractor::new(&spec, &Default::default(), ExtractorOptions::default()).unwrap();
//! let html = br#"<h1>Night</h1><ul class=lineup><li>A<li>B</ul><a class=tickets href="/t/1">Buy</a>"#;
//! let out = x.extract(html, Some("https://venue.example/e/1"));
//! assert_eq!(out.data, json!({ "title": "Night", "acts": ["A", "B"], "tickets": "https://venue.example/t/1" }));
//! ```

pub mod aria;
pub mod charset;
mod decode;
mod entities;
mod extract;
mod number;
mod out;
pub mod page;
pub mod robots;
pub mod selector;
pub mod sitemap;
mod skeleton;
mod spec;
mod strip;
pub mod structured;
mod tags;
pub mod text;
mod tokenizer;
pub mod tree;
mod utf16;
mod value;

use std::sync::Arc;

use url::Url;

pub use out::Value;
pub use skeleton::{template_hash, Skeleton, SkeletonOptions, DEFAULT_IGNORE};
#[cfg(not(feature = "grammar"))]
pub use spec::NoGrammar;
pub use spec::{CompileOptions, Grammars, Spec, SpecError};
pub use strip::{StripOptions, Stripped};
pub use tree::{TreeStats, DEFAULT_MAX_DEPTH};
pub use utf16::utf16_to_utf8;
#[cfg(feature = "grammar")]
pub use value::tree_json;
pub use value::{http_url, parse_json};

use extract::Engine;
use skeleton::SkeletonPass;
use strip::StripPass;
use structured::StructuredPass;
use tokenizer::{AttrFilter, Tokenizer};
use tree::{Builder, Handler, Tree};

/// How a pass runs, besides the spec.
#[derive(Clone, Debug)]
pub struct ExtractorOptions {
    /// Also fingerprint the template.
    pub skeleton: Option<SkeletonOptions>,
    /// Also strip the page down.
    pub strip: Option<StripOptions>,
    /// Also read the page's structured data (JSON-LD, microdata, RDFa, microformats).
    pub structured: Option<structured::StructuredOptions>,
    /// Also read what a scraper wants of any page ([`page::Page`]: title, meta, links, images, feeds…).
    pub page: Option<page::PageOptions>,
    /// Also take the page's accessibility snapshot, as Playwright's `ariaSnapshot()` gives it.
    pub aria: Option<aria::AriaOptions>,
    /// How deep elements nest at most.
    pub max_depth: usize,
    /// The most one value may hold, in bytes (text past it is cut).
    pub max_capture: usize,
    /// Leave out fields that do not compile (listed in [`Extractor::warnings`]) instead of failing.
    pub lenient: bool,
}

impl Default for ExtractorOptions {
    fn default() -> Self {
        ExtractorOptions {
            skeleton: None,
            strip: None,
            structured: None,
            page: None,
            aria: None,
            max_depth: DEFAULT_MAX_DEPTH,
            max_capture: 4 << 20,
            lenient: false,
        }
    }
}

/// A compiled spec and its options; cheap to share across threads.
#[derive(Clone, Debug)]
pub struct Extractor {
    spec: Arc<Spec>,
    options: Arc<ExtractorOptions>,
    warnings: Arc<Vec<String>>,
    filter: AttrFilter,
}

/// Counts of one pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub bytes: u64,
    pub elements: u64,
    pub max_depth: usize,
    /// Start tags ignored past [`ExtractorOptions::max_depth`].
    pub too_deep: u64,
    /// Elements a field matched.
    pub matches: u64,
}

/// What one pass found.
#[derive(Clone, Debug)]
pub struct Output {
    /// The spec's top-level object.
    pub data: Value,
    /// Values that could not be read (bad JSON, a grammar that did not match), each with its field.
    pub errors: Vec<String>,
    pub skeleton: Option<Skeleton>,
    pub stripped: Option<Stripped>,
    pub structured: Option<structured::Structured>,
    pub page: Option<page::Page>,
    pub aria: Option<aria::AriaSnapshot>,
    /// The encoding the page was read in (`UTF-8`, `windows-1252`, `Shift_JIS`), and how it was found.
    pub charset: &'static str,
    pub charset_source: charset::Source,
    pub stats: Stats,
    /// Items of `emit` fields not yet taken ([`Session::take`]): those the end of the document completed.
    pub items: Vec<(String, Value)>,
}

impl Extractor {
    pub fn new(
        spec: &serde_json::Value,
        grammars: &Grammars,
        options: ExtractorOptions,
    ) -> Result<Extractor, SpecError> {
        let (spec, warnings) = Spec::new(
            spec,
            grammars,
            CompileOptions {
                lenient: options.lenient,
            },
        )?;
        // The attributes anything in this pass reads; the rest are not copied.
        let mut filter = AttrFilter {
            names: spec.attributes(),
            ..AttrFilter::default()
        };
        if options.skeleton.is_some() {
            filter.names.extend(["id", "class"].map(String::from));
        }
        if let Some(strip) = &options.strip {
            filter.names.extend(strip.keep_attrs.iter().cloned());
            filter
                .names
                .extend(["hidden", "content", "property", "name", "type"].map(String::from));
            filter.data = strip.keep_data;
        }
        // Structured data reads any attribute (RDFa's, microformats' classes, `xmlns:`…).
        if options.structured.is_some() || options.page.is_some() || options.aria.is_some() {
            filter.all = true;
        }
        let filter = filter.prepare();
        Ok(Extractor {
            spec: Arc::new(spec),
            options: Arc::new(options),
            warnings: Arc::new(warnings),
            filter,
        })
    }

    /// The fields left out ([`ExtractorOptions::lenient`]), with why.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// A pass to feed chunk by chunk; `url` is the page's (relative URLs resolve against it).
    pub fn session(&self, url: Option<&str>) -> Session {
        self.session_with(url, None)
    }

    /// [`Extractor::session`], with the charset the server stated (`Content-Type`), if any: the
    /// page is read in the encoding it is in ([`charset`]).
    pub fn session_with(&self, url: Option<&str>, charset: Option<&str>) -> Session {
        let mut s = self.bare_session(url);
        s.transcoder = Some(charset::Transcoder::new(charset));
        s
    }

    fn bare_session(&self, url: Option<&str>) -> Session {
        let page = url.and_then(|u| Url::parse(u).ok());
        let pass = Pass {
            engine: Engine::new(self.spec.clone(), page.clone(), self.options.max_capture),
            skeleton: self.options.skeleton.clone().map(SkeletonPass::new),
            strip: self.options.strip.clone().map(StripPass::new),
            structured: self
                .options
                .structured
                .clone()
                .map(|o| StructuredPass::new(o, page.clone(), url)),
            page: self.options.page.clone().map(|o| page::PagePass::new(o, page.clone())),
            aria: self.options.aria.clone().map(|o| aria::AriaPass::new(o, page.as_ref())),
        };
        Session {
            tokenizer: Tokenizer::new(self.filter.clone()),
            builder: Builder::new(pass, self.spec.siblings, self.spec.types, self.options.max_depth),
            bytes: 0,
            transcoder: None,
            charset: (encoding_rs::UTF_8, charset::Source::Guessed),
        }
    }

    /// A whole document at once. Its elements are read in place: nothing of it is copied
    /// but what the values hold.
    pub fn extract(&self, html: &[u8], url: Option<&str>) -> Output {
        self.extract_with(html, url, None)
    }

    /// [`Extractor::extract`], with the charset the server stated, if any. A page in UTF-8 is read
    /// in place; one in another encoding is decoded first.
    pub fn extract_with(&self, html: &[u8], url: Option<&str>, charset: Option<&str>) -> Output {
        let mut s = self.bare_session(url);
        s.bytes += html.len() as u64;
        let (body, enc, source) = charset::whole(html, charset);
        s.charset = (enc, source);
        // `body` outlives the session, which ends here.
        s.tokenizer.write_pinned(&body, &mut s.builder);
        s.finish()
    }
}

struct Pass {
    engine: Engine,
    skeleton: Option<SkeletonPass>,
    strip: Option<StripPass>,
    structured: Option<StructuredPass>,
    page: Option<page::PagePass>,
    aria: Option<aria::AriaPass>,
}

impl Handler for Pass {
    fn open(&mut self, tree: &Tree) {
        self.engine.open(tree);
        if let Some(s) = &mut self.skeleton {
            s.open(tree);
        }
        if let Some(s) = &mut self.strip {
            s.open(tree);
        }
        if let Some(s) = &mut self.structured {
            s.open(tree);
        }
        if let Some(p) = &mut self.page {
            p.open(tree);
        }
        if let Some(a) = &mut self.aria {
            a.open(tree);
        }
    }

    fn close(&mut self, tree: &Tree) {
        self.engine.close(tree);
        if let Some(s) = &mut self.skeleton {
            s.close(tree);
        }
        if let Some(s) = &mut self.strip {
            s.close(tree);
        }
        if let Some(s) = &mut self.structured {
            s.close(tree);
        }
        if let Some(p) = &mut self.page {
            p.close(tree);
        }
        if let Some(a) = &mut self.aria {
            a.close(tree);
        }
    }

    fn text(&mut self, tree: &Tree, text: &tree::Text<'_>) {
        self.engine.text(tree, text);
        if let Some(s) = &mut self.strip {
            s.text(tree, text);
        }
        if let Some(s) = &mut self.structured {
            if s.wants_text() {
                s.text(tree, text);
            }
        }
        if let Some(p) = &mut self.page {
            if p.wants_text() {
                p.text(tree, text);
            }
        }
        if let Some(a) = &mut self.aria {
            a.text(tree, text);
        }
    }

    fn markup(&mut self, tree: &Tree, raw: &[u8]) {
        self.engine.markup(tree, raw);
        if let Some(s) = &mut self.strip {
            s.markup(tree, raw);
        }
        if let Some(s) = &mut self.structured {
            s.markup(tree, raw);
        }
        if let Some(a) = &mut self.aria {
            a.markup(tree, raw);
        }
    }

    fn wants_text(&self) -> bool {
        self.strip.is_some()
            || self.engine.wants_text()
            || self.structured.as_ref().is_some_and(StructuredPass::wants_text)
            || self.page.as_ref().is_some_and(page::PagePass::wants_text)
            || self.aria.is_some()
    }
}

/// One document being read.
pub struct Session {
    tokenizer: Tokenizer,
    builder: Builder<Pass>,
    bytes: u64,
    /// Bytes to UTF-8 (a whole document read at once is decoded before).
    transcoder: Option<charset::Transcoder>,
    charset: (&'static encoding_rs::Encoding, charset::Source),
}

impl Session {
    /// The next chunk of the document (in any encoding; any size, cut anywhere).
    pub fn write(&mut self, chunk: &[u8]) {
        self.bytes += chunk.len() as u64;
        let Session {
            tokenizer,
            builder,
            transcoder,
            ..
        } = self;
        match transcoder {
            Some(t) => t.write(chunk, &mut |b| tokenizer.write(b, builder)),
            None => tokenizer.write(chunk, builder),
        }
    }

    /// The next chunk of the document as text already decoded (a JavaScript string): read as it
    /// is, whatever charset the page states.
    pub fn write_text(&mut self, text: &str) {
        self.bytes += text.len() as u64;
        let Session {
            tokenizer,
            builder,
            transcoder,
            ..
        } = self;
        match transcoder {
            Some(t) => t.text(text, &mut |b| tokenizer.write(b, builder)),
            None => tokenizer.write(text.as_bytes(), builder),
        }
    }

    /// Items of `emit` fields completed so far (field name, value), in document order.
    pub fn take(&mut self) -> Vec<(String, Value)> {
        self.builder.handler.engine.take()
    }

    /// The end of the document: everything found.
    pub fn finish(mut self) -> Output {
        if let Some(mut t) = self.transcoder.take() {
            let Session { tokenizer, builder, .. } = &mut self;
            t.end(&mut |b| tokenizer.write(b, builder));
            self.charset = (t.encoding, t.source);
        }
        self.tokenizer.end(&mut self.builder);
        self.builder.finish();
        let tree_stats = self.builder.stats;
        let Pass {
            mut engine,
            skeleton,
            strip,
            structured,
            page,
            aria,
        } = self.builder.handler;
        let data = engine.finish(&self.builder.tree);
        Output {
            data,
            items: engine.take(),
            errors: engine.errors,
            skeleton: skeleton.map(SkeletonPass::finish),
            stripped: strip.map(StripPass::finish),
            structured: structured.map(StructuredPass::finish),
            page: page.map(|p| p.finish(self.charset.0.name())),
            aria: aria.map(aria::AriaPass::finish),
            charset: self.charset.0.name(),
            charset_source: self.charset.1,
            stats: Stats {
                bytes: self.bytes,
                elements: tree_stats.elements,
                max_depth: tree_stats.max_depth,
                too_deep: tree_stats.too_deep,
                matches: engine.matches,
            },
        }
    }
}

/// Layer timings for benchmarks: the tokenizer alone, and with the tree (no handler work).
#[doc(hidden)]
pub mod layers {
    use crate::tokenizer::{AttrFilter, TokenSink, Tokenizer};
    use crate::tree::{Builder, Element, Handler, Text, Tree};

    struct Null(u64);
    impl TokenSink for Null {
        fn start_tag(&mut self, el: Element, _: bool, _: &[u8]) {
            self.0 += el.name().len() as u64;
        }
        fn end_tag(&mut self, _: &str, _: &[u8]) {}
        fn text(&mut self, raw: &[u8], _: bool, _: bool) {
            self.0 += raw.len() as u64;
        }
        fn comment(&mut self, _: &[u8]) {}
        fn in_foreign(&self) -> bool {
            false
        }
    }

    struct Nothing;
    impl Handler for Nothing {
        fn open(&mut self, _: &Tree) {}
        fn close(&mut self, _: &Tree) {}
        fn text(&mut self, _: &Tree, _: &Text<'_>) {}
        fn markup(&mut self, _: &Tree, _: &[u8]) {}
    }

    /// Tokenizes `html`, keeping attributes named in `attrs` (all when `None`).
    pub fn tokenize(html: &[u8], attrs: Option<&[&str]>) -> u64 {
        let filter = match attrs {
            None => AttrFilter {
                all: true,
                ..AttrFilter::default()
            },
            Some(a) => AttrFilter {
                names: a.iter().map(|s| s.to_string()).collect(),
                ..AttrFilter::default()
            }
            .prepare(),
        };
        let mut t = Tokenizer::new(filter);
        let mut sink = Null(0);
        t.write(html, &mut sink);
        t.end(&mut sink);
        sink.0
    }

    /// Tokenizes and builds the tree.
    pub fn tree(html: &[u8], attrs: Option<&[&str]>) -> u64 {
        let filter = match attrs {
            None => AttrFilter {
                all: true,
                ..AttrFilter::default()
            },
            Some(a) => AttrFilter {
                names: a.iter().map(|s| s.to_string()).collect(),
                ..AttrFilter::default()
            }
            .prepare(),
        };
        let mut t = Tokenizer::new(filter);
        let mut b = Builder::new(Nothing, false, false, crate::DEFAULT_MAX_DEPTH);
        t.write(html, &mut b);
        t.end(&mut b);
        b.finish();
        b.stats.elements
    }
}

#[cfg(test)]
mod tests;
