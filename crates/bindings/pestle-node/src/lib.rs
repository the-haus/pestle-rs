#![deny(clippy::all)]

//! Node.js bindings: HTML read as a stream with CSS selectors, values read
//! as dates, prices, lists, and the page's structured data; pest grammars
//! with the `grammar` feature (see `ts/index.ts` for the documented API;
//! these are its native halves).
//!
//! Offsets handed to JavaScript count UTF-16 units, as its strings do.

use std::collections::HashMap;
#[cfg(feature = "grammar")]
use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Either, Uint8Array};
use napi::JsString;
use napi::{Env, Error, Result, Status, Task};
use napi_derive::napi;
use pestle_html::{Extractor, ExtractorOptions, Grammars, Output, Session, SkeletonOptions, StripOptions};

#[cfg(feature = "grammar")]
mod grammar;
#[cfg(feature = "grammar")]
pub use grammar::*;

/// Whether this build reads pest grammars.
#[napi(catch_unwind)]
pub fn has_grammars() -> bool {
    cfg!(feature = "grammar")
}

// Not on macOS: mimalloc keeps each thread's heap in fixed TLS slots of the
// process (108 and 109), so two addons that each bundle it (this one and
// @wikidata-rs/node, or two copies of this one) take each other's heaps and
// crash. Linux and Windows give each copy its own thread-local storage.
#[cfg(all(feature = "mimalloc", not(target_vendor = "apple")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "grammar")]
fn utf16_at(input: &str, byte: usize) -> u32 {
    let at = byte.min(input.len());
    input[..at].chars().map(|c| c.len_utf16() as u32).sum()
}

/// A failure the TypeScript side turns into a typed error: `<KIND>:<json>`.
fn coded(kind: &str, payload: serde_json::Value) -> Error {
    Error::new(Status::InvalidArg, format!("{kind}:{payload}"))
}

// ── Extraction ───────────────────────────────────────────────────────────

#[napi(object)]
pub struct NativeSkeletonOptions {
    pub depth: Option<u32>,
    pub ignore: Option<Vec<String>>,
    pub paths: Option<bool>,
}

#[napi(object)]
pub struct NativeStripOptions {
    pub max_chars: Option<u32>,
    pub max_text: Option<u32>,
    pub max_attr: Option<u32>,
    pub max_alike: Option<u32>,
    pub max_json_ld: Option<u32>,
    pub drop: Option<Vec<String>>,
    pub keep_attrs: Option<Vec<String>>,
    pub keep_data: Option<bool>,
    pub head: Option<bool>,
    pub meta: Option<String>,
    pub drop_empty: Option<Vec<String>>,
}

#[napi(object)]
pub struct NativeStructuredOptions {
    pub json_ld: Option<bool>,
    pub microdata: Option<bool>,
    pub rdfa: Option<bool>,
    pub microformats: Option<bool>,
    pub raw: Option<bool>,
    pub events: Option<bool>,
    /// Keep only items of these types (schema.org subtypes count).
    pub types: Option<Vec<String>>,
    pub arrays: Option<bool>,
    pub detailed: Option<bool>,
    pub embed: Option<bool>,
    pub collapse: Option<bool>,
    /// Each item's JSON-LD `@context` (default true).
    pub context: Option<bool>,
    /// Nested items listed on their own too.
    pub flatten: Option<bool>,
    /// The vocabulary of JSON-LD without a context (schema.org); an empty string: none.
    pub default_vocab: Option<String>,
    /// Microdata leniency (`content` anywhere, `itemtype` alone).
    pub lenient_microdata: Option<bool>,
    /// What sites get wrong read as meant (default true); false: every format to its spec's letter.
    pub lenient: Option<bool>,
    /// One event for the same event stated twice (default true).
    pub merge_events: Option<bool>,
}

fn structured_options(o: NativeStructuredOptions) -> pestle_html::structured::StructuredOptions {
    let mut s = pestle_html::structured::StructuredOptions {
        json_ld: o.json_ld.unwrap_or(true),
        microdata: o.microdata.unwrap_or(true),
        rdfa: o.rdfa.unwrap_or(true),
        microformats: o.microformats.unwrap_or(true),
        raw: o.raw.unwrap_or(false),
        events: o.events.unwrap_or(true),
        merge_events: o.merge_events.unwrap_or(true),
        ..Default::default()
    };
    if let Some(t) = o.types {
        s.render.types = t;
    }
    s.render.arrays = o.arrays.unwrap_or(false);
    s.render.detailed = o.detailed.unwrap_or(false);
    s.render.embed = o.embed.unwrap_or(true);
    s.render.collapse = o.collapse.unwrap_or(true);
    s.render.context = o.context.unwrap_or(true);
    s.render.flatten = o.flatten.unwrap_or(false);
    if let Some(v) = o.default_vocab {
        s.json_ld_options.default_vocab = (!v.is_empty()).then_some(v);
    }
    if o.lenient == Some(false) {
        s.microdata_options.lenient = false;
        s.rdfa_options.default_vocab = None;
        s.json_ld_options.strict = true;
    }
    if let Some(l) = o.lenient_microdata {
        s.microdata_options.lenient = l;
    }
    s
}

#[napi(object)]
pub struct NativePageOptions {
    pub links: Option<bool>,
    pub images: Option<bool>,
    pub headings: Option<bool>,
    pub embedded: Option<bool>,
    pub max_links: Option<u32>,
    pub max_images: Option<u32>,
    pub max_headings: Option<u32>,
    pub max_text: Option<u32>,
    pub max_embedded: Option<u32>,
}

fn page_options(o: NativePageOptions) -> pestle_html::page::PageOptions {
    let d = pestle_html::page::PageOptions::default();
    let n = |v: Option<u32>, d: usize| v.map_or(d, |v| v as usize);
    pestle_html::page::PageOptions {
        links: o.links.unwrap_or(d.links),
        images: o.images.unwrap_or(d.images),
        headings: o.headings.unwrap_or(d.headings),
        embedded: o.embedded.unwrap_or(d.embedded),
        max_links: n(o.max_links, d.max_links),
        max_images: n(o.max_images, d.max_images),
        max_headings: n(o.max_headings, d.max_headings),
        max_text: n(o.max_text, d.max_text),
        max_embedded: n(o.max_embedded, d.max_embedded),
        max_script_bytes: d.max_script_bytes,
    }
}

#[napi(object)]
pub struct NativeAriaOptions {
    /// `"default"` (Playwright's `ariaSnapshot()`) or `"ai"`.
    pub mode: Option<String>,
    pub depth: Option<u32>,
    pub style_blocks: Option<bool>,
    pub concealed: Option<bool>,
}

fn aria_options(o: NativeAriaOptions) -> Result<pestle_html::aria::AriaOptions> {
    use pestle_html::aria::{AriaMode, AriaOptions};
    let d = AriaOptions::default();
    let mode = match o.mode.as_deref() {
        None | Some("default") => AriaMode::Default,
        Some("ai") => AriaMode::Ai,
        Some(m) => {
            return Err(Error::new(
                Status::InvalidArg,
                format!("aria.mode: {m:?} is neither \"default\" nor \"ai\""),
            ))
        }
    };
    Ok(AriaOptions {
        mode,
        depth: o.depth.map(|d| d as usize).filter(|&d| d > 0),
        style_blocks: o.style_blocks.unwrap_or(d.style_blocks),
        concealed: o.concealed.unwrap_or(d.concealed),
    })
}

#[napi(object, object_from_js = false)]
pub struct NativeAria {
    pub snapshot: String,
    pub refs: Vec<NativeAriaRef>,
}

#[napi(object, object_from_js = false)]
pub struct NativeAriaRef {
    pub id: String,
    pub selector: String,
}

#[napi(object)]
pub struct NativeExtractorOptions {
    /// Grammar sources by name.
    pub grammars: Option<HashMap<String, String>>,
    pub skeleton: Option<NativeSkeletonOptions>,
    pub strip: Option<NativeStripOptions>,
    pub lenient: Option<bool>,
    pub max_depth: Option<u32>,
    pub max_capture: Option<u32>,
    pub structured: Option<NativeStructuredOptions>,
    pub page: Option<NativePageOptions>,
    pub aria: Option<NativeAriaOptions>,
}

#[napi(object)]
pub struct NativeSkeleton {
    pub hash: String,
    pub paths: Option<Vec<String>>,
}

#[napi(object)]
pub struct NativeStripped {
    pub html: String,
    pub tokens: u32,
    pub truncated: bool,
}

#[napi(object)]
pub struct NativeStats {
    pub bytes: f64,
    pub elements: f64,
    pub max_depth: u32,
    pub too_deep: f64,
    pub matches: f64,
}

/// An extraction's value, handed to JavaScript as plain objects.
pub struct Data(pub pestle_html::Value);

/// A page's structured data, handed to JavaScript as plain objects.
pub struct StructuredData(pub pestle_html::structured::Structured);

pub struct PageData(pub pestle_html::page::Page);

/// A Rust value handed to JavaScript as it serializes (straight to JS objects, no JSON between).
pub struct Js<T>(pub T);

impl<T> napi::bindgen_prelude::TypeName for Js<T> {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Object
    }
}

impl<T: serde::Serialize> napi::bindgen_prelude::ToNapiValue for Js<T> {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        use napi::NapiRaw;
        let env = Env::from_raw(env);
        Ok(env.to_js_value(&val.0)?.raw())
    }
}

impl napi::bindgen_prelude::TypeName for PageData {
    fn type_name() -> &'static str {
        "Page"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Object
    }
}

impl napi::bindgen_prelude::ToNapiValue for PageData {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        use napi::NapiRaw;
        let env = Env::from_raw(env);
        Ok(env.to_js_value(&val.0)?.raw())
    }
}

impl napi::bindgen_prelude::TypeName for StructuredData {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

impl napi::bindgen_prelude::ToNapiValue for StructuredData {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        use napi::NapiRaw;
        let env = Env::from_raw(env);
        Ok(env.to_js_value(&val.0)?.raw())
    }
}

impl napi::bindgen_prelude::TypeName for Data {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

impl napi::bindgen_prelude::ToNapiValue for Data {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        let env = Env::from_raw(env);
        use napi::NapiRaw;
        Ok(env.to_js_value(&val.0)?.raw())
    }
}

#[napi(object, object_from_js = false)]
pub struct NativeOutput {
    pub data: Data,
    pub errors: Vec<String>,
    pub skeleton: Option<NativeSkeleton>,
    pub stripped: Option<NativeStripped>,
    pub structured: Option<StructuredData>,
    pub page: Option<PageData>,
    pub aria: Option<NativeAria>,
    /// The encoding the page was read in (`UTF-8`, `windows-1252`).
    pub charset: String,
    pub stats: NativeStats,
    /// Items of `emit` fields the end of the document completed.
    pub items: Vec<NativeItem>,
}

#[napi(object, object_from_js = false)]
pub struct NativeItem {
    pub field: String,
    pub value: Data,
}

fn native_output(o: Output) -> NativeOutput {
    NativeOutput {
        items: o
            .items
            .into_iter()
            .map(|(field, value)| NativeItem {
                field,
                value: Data(value),
            })
            .collect(),
        data: Data(o.data),
        errors: o.errors,
        skeleton: o.skeleton.map(|s| NativeSkeleton {
            hash: s.hash,
            paths: s.paths,
        }),
        structured: o.structured.map(StructuredData),
        page: o.page.map(PageData),
        aria: o.aria.map(|a| NativeAria {
            snapshot: a.yaml,
            refs: a
                .refs
                .into_iter()
                .map(|r| NativeAriaRef {
                    id: r.id,
                    selector: r.selector,
                })
                .collect(),
        }),
        charset: o.charset.to_owned(),
        stripped: o.stripped.map(|s| NativeStripped {
            html: s.html,
            tokens: s.tokens as u32,
            truncated: s.truncated,
        }),
        stats: NativeStats {
            bytes: o.stats.bytes as f64,
            elements: o.stats.elements as f64,
            max_depth: o.stats.max_depth as u32,
            too_deep: o.stats.too_deep as f64,
            matches: o.stats.matches as f64,
        },
    }
}

fn skeleton_options(o: NativeSkeletonOptions) -> SkeletonOptions {
    let d = SkeletonOptions::default();
    SkeletonOptions {
        depth: o.depth.map_or(d.depth, |n| n as usize),
        ignore: o.ignore.unwrap_or(d.ignore),
        paths: o.paths.unwrap_or(d.paths),
    }
}

fn strip_options(o: NativeStripOptions) -> Result<StripOptions> {
    let d = StripOptions::default();
    let n = |v: Option<u32>, def: usize| v.map_or(def, |n| n as usize);
    let meta = match o.meta {
        Some(m) => regex::Regex::new(&m).map_err(|e| Error::new(Status::InvalidArg, format!("strip.meta: {e}")))?,
        None => d.meta.clone(),
    };
    Ok(StripOptions {
        max_chars: n(o.max_chars, d.max_chars),
        max_text: n(o.max_text, d.max_text),
        max_attr: n(o.max_attr, d.max_attr),
        max_alike: n(o.max_alike, d.max_alike),
        max_json_ld: n(o.max_json_ld, d.max_json_ld),
        drop: o.drop.unwrap_or(d.drop),
        keep_attrs: o.keep_attrs.unwrap_or(d.keep_attrs),
        keep_data: o.keep_data.unwrap_or(d.keep_data),
        head: o.head.unwrap_or(d.head),
        meta,
        drop_empty: o.drop_empty.unwrap_or(d.drop_empty),
    })
}

/// A document from JavaScript: bytes as they are, a string as UTF-8. A
/// string is copied out as UTF-16 (V8's own form, a copy) and transcoded
/// here, about three times faster than asking V8 for UTF-8.
enum Html {
    /// A JavaScript string, as UTF-8: text already decoded.
    Owned(Vec<u8>),
    /// Bytes, in whatever encoding the page is in.
    Borrowed(Uint8Array),
}

impl Html {
    /// The charset to read it in: UTF-8 for text, the caller's (or the page's own) for bytes.
    fn charset<'c>(&self, stated: Option<&'c str>) -> Option<&'c str> {
        match self {
            Html::Owned(_) => Some("utf-8"),
            Html::Borrowed(_) => stated,
        }
    }
}

impl std::ops::Deref for Html {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Html::Owned(v) => v,
            Html::Borrowed(b) => b,
        }
    }
}

fn html(input: Either<JsString, Uint8Array>) -> Result<Html> {
    match input {
        Either::A(s) => {
            let units = s.into_utf16()?;
            let units = units.as_slice();
            let units = units.split_last().map_or(units, |(_, rest)| rest);
            let mut out = Vec::new();
            pestle_html::utf16_to_utf8(units, &mut out);
            Ok(Html::Owned(out))
        }
        Either::B(b) => Ok(Html::Borrowed(b)),
    }
}

/// A compiled spec.
/// Grammar sources compiled; throws `GRAMMAR:{name, errors}`.
#[cfg(feature = "grammar")]
fn compile_grammars(sources: HashMap<String, String>) -> Result<Grammars> {
    let mut grammars = Grammars::new();
    for (name, source) in sources {
        match pestle_grammar::Grammar::new(&source) {
            Ok(g) => {
                grammars.insert(name, Arc::new(g));
            }
            Err(e) => {
                let errors: Vec<serde_json::Value> = e
                    .errors
                    .into_iter()
                    .map(|e| grammar::error_json(&grammar::native_error(e, &source)))
                    .collect();
                return Err(coded("GRAMMAR", serde_json::json!({ "name": name, "errors": errors })));
            }
        }
    }
    Ok(grammars)
}

/// Without the `grammar` feature, any grammar is an error that says so.
#[cfg(not(feature = "grammar"))]
fn compile_grammars(sources: HashMap<String, String>) -> Result<Grammars> {
    match sources.into_keys().next() {
        None => Ok(Grammars::new()),
        Some(name) => Err(coded(
            "GRAMMAR",
            serde_json::json!({ "name": name, "errors": [{ "message": "this build of @pestle-rs/node reads no grammars (built without the `grammar` feature); use `as` (date, price, number, duration, list) or a build with grammars", "line": 1, "col": 1, "start": 0, "end": 0 }] }),
        )),
    }
}

#[napi]
pub struct NativeExtractor {
    inner: Extractor,
}

#[napi]
impl NativeExtractor {
    /// Throws `SPEC:[errors]` or `GRAMMAR:{name, errors}`.
    #[napi(constructor, catch_unwind)]
    pub fn new(spec: serde_json::Value, options: Option<NativeExtractorOptions>) -> Result<Self> {
        let o = options.unwrap_or(NativeExtractorOptions {
            grammars: None,
            skeleton: None,
            strip: None,
            lenient: None,
            max_depth: None,
            max_capture: None,
            structured: None,
            page: None,
            aria: None,
        });
        let grammars = compile_grammars(o.grammars.unwrap_or_default())?;
        let d = ExtractorOptions::default();
        let options = ExtractorOptions {
            skeleton: o.skeleton.map(skeleton_options),
            strip: o.strip.map(strip_options).transpose()?,
            max_depth: o.max_depth.map_or(d.max_depth, |n| n as usize),
            max_capture: o.max_capture.map_or(d.max_capture, |n| n as usize),
            lenient: o.lenient.unwrap_or(false),
            structured: o.structured.map(structured_options),
            page: o.page.map(page_options),
            aria: o.aria.map(aria_options).transpose()?,
        };
        Extractor::new(&spec, &grammars, options)
            .map(|inner| NativeExtractor { inner })
            .map_err(|e| coded("SPEC", serde_json::json!(e.errors)))
    }

    /// Fields left out of a lenient spec, with why.
    #[napi(getter, catch_unwind)]
    pub fn warnings(&self) -> Vec<String> {
        self.inner.warnings().to_vec()
    }

    #[napi(catch_unwind)]
    /// `charset`: the one the server stated (`Content-Type`), for bytes; text is read as it is.
    pub fn extract(
        &self,
        input: Either<JsString, Uint8Array>,
        url: Option<String>,
        charset: Option<String>,
    ) -> Result<NativeOutput> {
        let html = html(input)?;
        let charset = html.charset(charset.as_deref());
        Ok(native_output(self.inner.extract_with(&html, url.as_deref(), charset)))
    }

    /// The same, on the thread pool.
    #[napi(catch_unwind)]
    pub fn extract_async(
        &self,
        input: Either<JsString, Uint8Array>,
        url: Option<String>,
        charset: Option<String>,
    ) -> Result<AsyncTask<ExtractTask>> {
        let html = html(input)?;
        let charset = html.charset(charset.as_deref()).map(str::to_owned);
        Ok(AsyncTask::new(ExtractTask {
            extractor: self.inner.clone(),
            html: html.to_vec(),
            url,
            charset,
        }))
    }

    #[napi(catch_unwind)]
    pub fn session(&self, url: Option<String>, charset: Option<String>) -> NativeSession {
        NativeSession {
            inner: Some(self.inner.session_with(url.as_deref(), charset.as_deref())),
        }
    }
}

pub struct ExtractTask {
    extractor: Extractor,
    html: Vec<u8>,
    url: Option<String>,
    charset: Option<String>,
}

impl Task for ExtractTask {
    type Output = Output;
    type JsValue = NativeOutput;

    fn compute(&mut self) -> Result<Output> {
        Ok(self
            .extractor
            .extract_with(&self.html, self.url.as_deref(), self.charset.as_deref()))
    }

    fn resolve(&mut self, _env: Env, output: Output) -> Result<NativeOutput> {
        Ok(native_output(output))
    }
}

/// A document read chunk by chunk.
#[napi]
pub struct NativeSession {
    inner: Option<Session>,
}

fn ended() -> Error {
    Error::new(Status::GenericFailure, "the session has ended".to_owned())
}

#[napi]
impl NativeSession {
    #[napi(catch_unwind)]
    pub fn write(&mut self, chunk: Either<JsString, Uint8Array>) -> Result<()> {
        let chunk = html(chunk)?;
        let session = self.inner.as_mut().ok_or_else(ended)?;
        match &chunk {
            // A string is text already decoded.
            Html::Owned(text) => session.write_text(std::str::from_utf8(text).unwrap_or_default()),
            Html::Borrowed(bytes) => session.write(bytes),
        }
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn take(&mut self) -> Result<Vec<NativeItem>> {
        let items = self.inner.as_mut().ok_or_else(ended)?.take();
        Ok(items
            .into_iter()
            .map(|(field, value)| NativeItem {
                field,
                value: Data(value),
            })
            .collect())
    }

    #[napi(catch_unwind)]
    pub fn end(&mut self) -> Result<NativeOutput> {
        Ok(native_output(self.inner.take().ok_or_else(ended)?.finish()))
    }
}

/// The skeleton hash that pages share (the paths at least half of them have).
#[napi(catch_unwind)]
pub fn template_hash(pages: Vec<Vec<String>>) -> Option<String> {
    pestle_html::template_hash(&pages)
}

// ── robots.txt and sitemaps ──────────────────────────────────────────────

/// A robots.txt read for one product token.
#[napi]
pub struct NativeRobots {
    inner: pestle_html::robots::Robots,
}

#[napi]
impl NativeRobots {
    #[napi(constructor, catch_unwind)]
    pub fn new(text: String, token: String) -> Self {
        NativeRobots {
            inner: pestle_html::robots::Robots::parse(&text, &token),
        }
    }

    /// Allows everything (a robots.txt that is absent: 4xx).
    #[napi(factory, catch_unwind)]
    pub fn allow_all() -> Self {
        NativeRobots {
            inner: pestle_html::robots::Robots::allow_all(),
        }
    }

    /// Allows nothing (a robots.txt that could not be read: 5xx, no answer).
    #[napi(factory, catch_unwind)]
    pub fn disallow_all() -> Self {
        NativeRobots {
            inner: pestle_html::robots::Robots::disallow_all(),
        }
    }

    #[napi(catch_unwind)]
    pub fn allows(&self, url: String) -> bool {
        self.inner.allows(&url)
    }

    /// Many URLs in one call.
    #[napi(catch_unwind)]
    pub fn allows_all(&self, urls: Vec<String>) -> Vec<bool> {
        urls.iter().map(|u| self.inner.allows(u)).collect()
    }

    #[napi(getter, catch_unwind)]
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(|e| Error::from_reason(e.to_string()))
    }
}

#[napi(object)]
pub struct NativeSitemapOptions {
    pub max_entry_bytes: Option<u32>,
    pub max_bytes: Option<f64>,
    pub max_entries: Option<u32>,
}

/// A sitemap read chunk by chunk; throws `SITEMAP:"why"` for a body that is none.
#[napi]
pub struct NativeSitemapReader {
    inner: Option<pestle_html::sitemap::SitemapReader>,
}

fn not_a_sitemap(e: pestle_html::sitemap::NotASitemap) -> Error {
    coded("SITEMAP", serde_json::json!(e.0))
}

#[napi]
impl NativeSitemapReader {
    #[napi(constructor, catch_unwind)]
    pub fn new(url: Option<String>, options: Option<NativeSitemapOptions>) -> Self {
        let d = pestle_html::sitemap::SitemapOptions::default();
        let opts = match options {
            None => d,
            Some(o) => pestle_html::sitemap::SitemapOptions {
                max_entry_bytes: o.max_entry_bytes.map_or(d.max_entry_bytes, |n| n as usize),
                max_bytes: o.max_bytes.map_or(d.max_bytes, |n| n.max(0.0) as usize),
                max_entries: o.max_entries.map_or(d.max_entries, |n| n as usize),
                ..d
            },
        };
        NativeSitemapReader {
            inner: Some(pestle_html::sitemap::SitemapReader::with_options(url.as_deref(), opts)),
        }
    }

    /// The next bytes; the entries they complete.
    #[napi(catch_unwind, ts_return_type = "unknown[]")]
    pub fn write(&mut self, chunk: Either<JsString, Uint8Array>) -> Result<Js<Vec<pestle_html::sitemap::Entry>>> {
        let reader = self.inner.as_mut().ok_or_else(ended)?;
        let bytes = html(chunk)?;
        reader.write(&bytes).map_err(not_a_sitemap)?;
        Ok(Js(reader.take()))
    }

    /// [`NativeSitemapReader::write`], the entries as JSON text (cheaper to hand over in bulk).
    #[napi(catch_unwind)]
    pub fn write_json(&mut self, chunk: Either<JsString, Uint8Array>) -> Result<String> {
        let reader = self.inner.as_mut().ok_or_else(ended)?;
        let bytes = html(chunk)?;
        reader.write(&bytes).map_err(not_a_sitemap)?;
        serde_json::to_string(&reader.take()).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// [`NativeSitemapReader::end`] as JSON text.
    #[napi(catch_unwind)]
    pub fn end_json(&mut self) -> Result<String> {
        let reader = self.inner.take().ok_or_else(ended)?;
        let done = reader.end().map_err(not_a_sitemap)?;
        serde_json::to_string(&done).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The end: `{ format, entries (the last ones), gzip, warnings }`.
    #[napi(catch_unwind, ts_return_type = "unknown")]
    pub fn end(&mut self) -> Result<Js<pestle_html::sitemap::Sitemap>> {
        let reader = self.inner.take().ok_or_else(ended)?;
        Ok(Js(reader.end().map_err(not_a_sitemap)?))
    }
}

/// A whole sitemap read on the thread pool: `{ format, entries, gzip, warnings }` as JSON text.
pub struct SitemapTask {
    body: Vec<u8>,
    url: Option<String>,
    options: pestle_html::sitemap::SitemapOptions,
}

impl Task for SitemapTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        let mut r = pestle_html::sitemap::SitemapReader::with_options(self.url.as_deref(), self.options);
        r.write(&self.body).map_err(not_a_sitemap)?;
        let done = r.end().map_err(not_a_sitemap)?;
        serde_json::to_string(&done).map_err(|e| Error::from_reason(e.to_string()))
    }

    fn resolve(&mut self, _env: Env, output: String) -> Result<String> {
        Ok(output)
    }
}

#[napi(catch_unwind)]
pub fn read_sitemap_async(
    body: Either<JsString, Uint8Array>,
    url: Option<String>,
    options: Option<NativeSitemapOptions>,
) -> Result<AsyncTask<SitemapTask>> {
    let d = pestle_html::sitemap::SitemapOptions::default();
    let options = match options {
        None => d,
        Some(o) => pestle_html::sitemap::SitemapOptions {
            max_entry_bytes: o.max_entry_bytes.map_or(d.max_entry_bytes, |n| n as usize),
            max_bytes: o.max_bytes.map_or(d.max_bytes, |n| n.max(0.0) as usize),
            max_entries: o.max_entries.map_or(d.max_entries, |n| n as usize),
            ..d
        },
    };
    Ok(AsyncTask::new(SitemapTask {
        body: html(body)?.to_vec(),
        url,
        options,
    }))
}

// ── Routes ───────────────────────────────────────────────────────────────

#[napi(object)]
pub struct NativeRouteOptions {
    pub min_siblings: Option<u32>,
    pub max_examples: Option<u32>,
    pub max_urls: Option<u32>,
}

/// A page from a sitemap: what it says of the page goes into its route's figures.
#[napi(object)]
pub struct NativeRouteEntry {
    pub url: String,
    pub lastmod: Option<String>,
    pub images: Option<bool>,
}

/// URLs read in batches; routes inferred at [`NativeRouteBuilder::finish`].
#[napi]
pub struct NativeRouteBuilder {
    inner: Option<pestle_html::routes::RouteBuilder>,
}

#[napi]
impl NativeRouteBuilder {
    #[napi(constructor, catch_unwind)]
    pub fn new(options: Option<NativeRouteOptions>) -> Self {
        let d = pestle_html::routes::RouteOptions::default();
        let opts = match options {
            None => d,
            Some(o) => pestle_html::routes::RouteOptions {
                min_siblings: o.min_siblings.map_or(d.min_siblings, |n| n as usize),
                max_examples: o.max_examples.map_or(d.max_examples, |n| n as usize),
                max_urls: o.max_urls.map_or(d.max_urls, |n| n as usize),
                ..d
            },
        };
        NativeRouteBuilder {
            inner: Some(pestle_html::routes::RouteBuilder::new(opts)),
        }
    }

    #[napi(catch_unwind)]
    pub fn add(&mut self, urls: Vec<String>) -> Result<()> {
        let b = self.inner.as_mut().ok_or_else(ended)?;
        for u in &urls {
            b.add(u);
        }
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn add_entries(&mut self, entries: Vec<NativeRouteEntry>) -> Result<()> {
        let b = self.inner.as_mut().ok_or_else(ended)?;
        for e in entries {
            b.add_with(&e.url, e.lastmod, e.images.unwrap_or(false));
        }
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn finish(&mut self) -> Result<NativeRoutes> {
        Ok(NativeRoutes {
            inner: self.inner.take().ok_or_else(ended)?.finish(),
        })
    }
}

/// A site's routes: the tree, a URL's route, the tree as text.
#[napi]
pub struct NativeRoutes {
    inner: pestle_html::routes::Routes,
}

#[napi]
impl NativeRoutes {
    /// `{ sites, warnings }` as JSON text.
    #[napi(catch_unwind)]
    pub fn json(&self) -> Result<String> {
        serde_json::to_string(&self.inner).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// `[origin, pattern]` of the route each URL is, or null.
    #[napi(catch_unwind)]
    pub fn find_all(&self, urls: Vec<String>) -> Vec<Option<Vec<String>>> {
        urls.iter()
            .map(|u| {
                let (site, route) = self.inner.locate(u)?;
                Some(vec![site.origin.clone(), route.pattern.clone()])
            })
            .collect()
    }

    #[napi(catch_unwind)]
    pub fn outline(&self) -> String {
        self.inner.outline()
    }
}
