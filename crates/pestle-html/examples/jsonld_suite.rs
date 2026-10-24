//! Runs the W3C JSON-LD 1.1 toRdf tests through `structured::jsonld11` and prints one JSON line per
//! test: `{"id", "nq"}` or `{"id", "error"}`. `bench/jsonld-suite.mjs` (in the Node binding)
//! canonicalizes and scores them.
//!
//! cargo run --release -p pestle-html --example jsonld_suite -- <json-ld-api/tests>

use std::path::{Path, PathBuf};

use pestle_html::structured::jsonld11::{document_to_rdf, Loader};
use serde_json::{json, Value};

const BASE: &str = "https://w3c.github.io/json-ld-api/tests/";

/// Suite URLs read from the local checkout.
struct Files(PathBuf);

impl Loader for Files {
    fn load(&self, url: &str) -> Option<(Value, String)> {
        let rel = url.strip_prefix(BASE)?;
        let text = std::fs::read_to_string(self.0.join(rel)).ok()?;
        Some((serde_json::from_str(&text).ok()?, url.to_owned()))
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("the json-ld-api/tests directory"));
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("toRdf-manifest.jsonld")).unwrap()).unwrap();
    let loader = Files(dir.clone());
    for t in manifest["sequence"].as_array().unwrap() {
        let id = t["@id"].as_str().unwrap();
        let input = t["input"].as_str().unwrap();
        let base = t["option"]["base"]
            .as_str()
            .map_or_else(|| format!("{BASE}{input}"), str::to_owned);
        let line = match read(&dir, input) {
            Some(doc) => match document_to_rdf(&doc, Some(&base), &loader) {
                Ok(nq) => json!({ "id": id, "nq": nq }),
                Err(e) => json!({ "id": id, "error": e.0 }),
            },
            None => json!({ "id": id, "error": "loading document failed" }),
        };
        println!("{line}");
    }
}

fn read(dir: &Path, input: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(input)).ok()?).ok()
}
