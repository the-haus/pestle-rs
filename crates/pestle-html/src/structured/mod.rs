//! Structured data: JSON-LD, microdata, RDFa (OpenGraph and Dublin Core
//! included) and microformats, read in the same streaming pass as
//! everything else and given back as one list of items, one flat list of
//! events, and (when asked) each format in its own standard form.
//!
//! ```
//! use pestle_html::{Extractor, ExtractorOptions};
//! use pestle_html::structured::StructuredOptions;
//!
//! let html = br#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"MusicEvent","name":"Nora","startDate":"2026-11-14T20:00"}</script>"#;
//! let x = Extractor::new(&serde_json::json!({}), &Default::default(),
//!     ExtractorOptions { structured: Some(StructuredOptions::default()), ..Default::default() }).unwrap();
//! let s = x.extract(html, Some("https://venue.example/")).structured.unwrap();
//! assert_eq!(s.events[0]["name"], "Nora");
//! ```
//!
//! Every reader is bounded (text per value, triples, tree nodes, nesting)
//! and lenient: a malformed page gives what can be read, never an error.

pub mod events;
pub mod graph;
pub mod iri;
pub mod jsonld;
pub mod jsonld11;
pub mod lenient;
pub mod microdata;
pub mod microformats;
pub mod rdfa;
mod schema_org;
pub mod vocab;

use serde::Serialize;
use serde_json::Value;
use url::Url;

use self::graph::{Graph, Render, Writer};
use self::jsonld::{JsonLd, JsonLdOptions};
use self::microdata::{Microdata, MicrodataOptions};
use self::microformats::{Microformats, MicroformatsOptions};
use self::rdfa::{Rdfa, RdfaOptions};
use crate::tree::{Handler, Text, Tree};

/// What the structured pass reads, and how it writes it.
#[derive(Clone, Debug)]
pub struct StructuredOptions {
    pub json_ld: bool,
    pub microdata: bool,
    pub rdfa: bool,
    pub microformats: bool,
    /// Also each format in its own standard form: the JSON-LD blocks as parsed, microdata's
    /// WHATWG JSON, RDFa's statements, microformats2 JSON.
    pub raw: bool,
    /// The flat events view.
    pub events: bool,
    /// One event for the same event stated twice (see [`events`]).
    pub merge_events: bool,
    /// How items are written (types to keep, arrays, literals, embedding).
    pub render: Render,
    pub json_ld_options: JsonLdOptions,
    pub microdata_options: MicrodataOptions,
    pub rdfa_options: RdfaOptions,
    pub microformats_options: MicroformatsOptions,
    /// At most this many JSON-LD blocks, of at most this many bytes each.
    pub max_blocks: usize,
    pub max_block_bytes: usize,
}

impl Default for StructuredOptions {
    fn default() -> Self {
        StructuredOptions {
            json_ld: true,
            microdata: true,
            rdfa: true,
            microformats: true,
            raw: false,
            events: true,
            merge_events: true,
            render: Render::default(),
            json_ld_options: JsonLdOptions::default(),
            microdata_options: MicrodataOptions::default(),
            rdfa_options: RdfaOptions::default(),
            microformats_options: MicroformatsOptions::default(),
            max_blocks: 256,
            max_block_bytes: 8 << 20,
        }
    }
}

/// The structured data of a page.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Structured {
    /// Every item, of every format, schema.org names short, references embedded.
    pub items: Vec<Value>,
    /// Every event, flat ([`events`]).
    pub events: Vec<Value>,
    /// With [`StructuredOptions::raw`]: each format in its own form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Raw>,
    /// What had to be repaired or was cut (`json-ld: trailing commas`).
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Raw {
    /// One entry per JSON-LD `<script>`, in page order: `null` for a block that cannot be read,
    /// an array for several values glued together in one.
    #[serde(rename = "jsonLd")]
    pub json_ld: Vec<Value>,
    pub microdata: Value,
    pub rdfa: Value,
    pub microformats: Value,
}

pub(crate) struct StructuredPass {
    opts: StructuredOptions,
    page: Option<Url>,
    base: Option<Url>,
    /// The base as written (the page's URL as given, or `<base href>` resolved against it, RFC 3986):
    /// what microformats resolve against.
    base_raw: Option<String>,
    base_seen: bool,
    blocks: Vec<String>,
    /// The JSON-LD script being read: its depth, its text, and whether it was cut.
    script: Option<(usize, String, bool)>,
    warnings: Vec<String>,
    microdata: Option<Microdata>,
    rdfa: Option<Rdfa>,
    microformats: Option<Microformats>,
}

impl StructuredPass {
    pub(crate) fn new(opts: StructuredOptions, page: Option<Url>, raw: Option<&str>) -> StructuredPass {
        StructuredPass {
            microdata: opts.microdata.then(|| Microdata::new(opts.microdata_options)),
            rdfa: opts.rdfa.then(|| Rdfa::new(page.as_ref(), opts.rdfa_options)),
            microformats: opts.microformats.then(|| Microformats::new(opts.microformats_options)),
            base: page.clone(),
            base_raw: raw.filter(|_| page.is_some()).map(|r| r.trim().to_owned()),
            page,
            base_seen: false,
            blocks: Vec::new(),
            script: None,
            warnings: Vec::new(),
            opts,
        }
    }

    pub(crate) fn wants_text(&self) -> bool {
        self.script.is_some()
            || self.microdata.as_ref().is_some_and(Microdata::wants_text)
            || self.rdfa.as_ref().is_some_and(|r| r.wants_text() || r.wants_markup())
            || self.microformats.as_ref().is_some_and(Microformats::wants_text)
    }

    pub(crate) fn finish(mut self) -> Structured {
        let mut graph = Graph::default();
        let mut raw = Raw::default();
        if self.opts.json_ld {
            let mut j = JsonLd::new(&mut graph, self.base.as_ref(), &self.opts.json_ld_options);
            for block in &self.blocks {
                if self.opts.json_ld_options.strict {
                    let warning = match serde_json::from_str::<Value>(block.trim()) {
                        Ok(v) => {
                            let w = j
                                .block_strict(&v)
                                .err()
                                .map(|e| format!("json-ld: a block was left out: {}", e.0));
                            if self.opts.raw {
                                raw.json_ld.push(v);
                            }
                            w
                        }
                        Err(_) => {
                            if self.opts.raw {
                                raw.json_ld.push(Value::Null);
                            }
                            Some("json-ld: a block was left out: it is not JSON".into())
                        }
                    };
                    if let Some(w) = warning.filter(|w| !self.warnings.contains(w)) {
                        self.warnings.push(w);
                    }
                    continue;
                }
                let read = lenient::read(block);
                for r in read.repairs {
                    let w = format!("json-ld: {r}");
                    if !self.warnings.contains(&w) {
                        self.warnings.push(w);
                    }
                }
                for v in &read.values {
                    j.block(v);
                }
                if self.opts.raw {
                    // One entry per block, so a block keeps its place when one before it breaks:
                    // `null` when nothing can be read, values glued together as one array.
                    let mut values = read.values;
                    raw.json_ld.push(match values.len() {
                        0 => Value::Null,
                        1 => values.pop().unwrap_or_default(),
                        _ => Value::Array(values),
                    });
                }
            }
        }
        if let Some(m) = &self.microdata {
            m.into_graph(&mut graph);
            if self.opts.raw {
                raw.microdata = m.json();
            }
        }
        if let Some(r) = &mut self.rdfa {
            r.finish();
        }
        if let Some(r) = &self.rdfa {
            if r.truncated {
                self.warnings.push(format!(
                    "rdfa: statements past {} were left out",
                    self.opts.rdfa_options.max_triples
                ));
            }
            r.into_graph(&mut graph);
            if self.opts.raw {
                raw.rdfa = r.json();
            }
        }
        if let Some(m) = &mut self.microformats {
            m.finish(self.base_raw.as_deref());
        }
        if let Some(m) = &self.microformats {
            m.into_graph(&mut graph);
            if self.opts.raw {
                raw.microformats = m.json();
            }
        }
        let items = Writer::new(&graph, &self.opts.render).items();
        let events = if self.opts.events {
            events::events(&graph, &self.opts.render, self.opts.merge_events)
        } else {
            Vec::new()
        };
        Structured {
            items,
            events,
            raw: self.opts.raw.then_some(raw),
            warnings: self.warnings,
        }
    }
}

impl Handler for StructuredPass {
    fn open(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        let el = &tree.frames[depth].el;
        if !self.base_seen && el.kind == crate::tags::BASE {
            if let Some(href) = el.attr("href") {
                self.base_seen = true;
                let base = match &self.page {
                    Some(p) => p.join(href.trim()).ok(),
                    None => Url::parse(href.trim()).ok(),
                };
                if let Some(b) = base {
                    if let Some(r) = &mut self.rdfa {
                        r.set_base(&b);
                    }
                    self.base_raw = Some(match &self.base_raw {
                        Some(page) => iri::resolve(page, href.trim()),
                        None => b.to_string(),
                    });
                    self.base = Some(b);
                }
            }
        }
        if self.opts.json_ld
            && el.kind == crate::tags::SCRIPT
            && self.blocks.len() < self.opts.max_blocks
            // `application/ld+json`, and as pages misspell it (`text/ld+json`, `application/json+ld`).
            && el.attr("type").is_some_and(|t| {
                let t = t.trim().to_ascii_lowercase();
                t.contains("ld+json") || t.contains("json+ld") || t.contains("jsonld")
            })
        {
            self.script = Some((depth, String::new(), false));
        }
        let base = self.base.clone();
        let base_raw = self.base_raw.clone();
        if let Some(m) = &mut self.microdata {
            m.open(tree, base.as_ref());
        }
        if let Some(r) = &mut self.rdfa {
            r.open(tree);
        }
        if let Some(m) = &mut self.microformats {
            m.open(tree, base_raw.as_deref());
        }
    }

    fn close(&mut self, tree: &Tree) {
        let depth = tree.frames.len() - 1;
        if self.script.as_ref().is_some_and(|(d, ..)| *d == depth) {
            if let Some((_, text, _)) = self.script.take() {
                self.blocks.push(text);
            }
        }
        if let Some(m) = &mut self.microdata {
            m.close();
        }
        if let Some(r) = &mut self.rdfa {
            r.close(tree);
        }
        let base = self.base_raw.clone();
        if let Some(m) = &mut self.microformats {
            m.close(tree, base.as_deref());
        }
    }

    fn text(&mut self, _: &Tree, text: &Text<'_>) {
        let t = text.get();
        if let Some((_, s, cut)) = &mut self.script {
            if *cut {
            } else if s.len() + t.len() <= self.opts.max_block_bytes {
                s.push_str(t);
            } else {
                // What fits, and nothing after it: a later, shorter run would not join on.
                s.push_str(crate::text::cut_bytes(t, self.opts.max_block_bytes - s.len()));
                *cut = true;
                let w = "json-ld: a block was cut at max_block_bytes";
                if !self.warnings.iter().any(|x| x == w) {
                    self.warnings.push(w.into());
                }
            }
        }
        if let Some(m) = &mut self.microdata {
            if m.wants_text() {
                m.text(t);
            }
        }
        if let Some(r) = &mut self.rdfa {
            if r.wants_text() {
                r.text(t);
            }
        }
        if let Some(m) = &mut self.microformats {
            if m.wants_text() {
                m.text(t);
            }
        }
    }

    fn markup(&mut self, _: &Tree, raw: &[u8]) {
        if let Some(r) = &mut self.rdfa {
            if r.wants_markup() {
                r.markup(&crate::decode::utf8(raw));
            }
        }
    }
}
