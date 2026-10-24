//! The W3C JSON-LD 1.1 toRdf tests (https://github.com/w3c/json-ld-api), when a checkout is at
//! `bench/.work/suites/json-ld-api`: every negative test, and every positive test whose output has
//! no blank nodes (compared as sets of lines; the rest need canonicalization, which
//! `bench/jsonld-suite.mjs` in the Node binding does).

use std::collections::BTreeSet;
use std::path::PathBuf;

use pestle_html::structured::jsonld11::{document_to_rdf, Loader};
use serde_json::Value;

const BASE: &str = "https://w3c.github.io/json-ld-api/tests/";

struct Files(PathBuf);

impl Loader for Files {
    fn load(&self, url: &str) -> Option<(Value, String)> {
        let text = std::fs::read_to_string(self.0.join(url.strip_prefix(BASE)?)).ok()?;
        Some((serde_json::from_str(&text).ok()?, url.to_owned()))
    }
}

#[test]
fn the_w3c_to_rdf_tests() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/.work/suites/json-ld-api/tests");
    let Ok(manifest) = std::fs::read_to_string(dir.join("toRdf-manifest.jsonld")) else {
        eprintln!("no json-ld-api checkout; skipped");
        return;
    };
    let manifest: Value = serde_json::from_str(&manifest).unwrap();
    let loader = Files(dir.clone());
    let (mut run, mut failed) = (0, Vec::new());
    for t in manifest["sequence"].as_array().unwrap() {
        let o = &t["option"];
        if o["specVersion"] == "json-ld-1.0"
            || o["processingMode"] == "json-ld-1.0"
            || !o["rdfDirection"].is_null()
            || !o["produceGeneralizedRdf"].is_null()
            || !o["expandContext"].is_null()
        {
            continue;
        }
        let input = t["input"].as_str().unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(input)).unwrap()).unwrap();
        let base = o["base"]
            .as_str()
            .map_or_else(|| format!("{BASE}{input}"), str::to_owned);
        let got = document_to_rdf(&doc, Some(&base), &loader);
        let types = t["@type"].to_string();
        let ok = if types.contains("NegativeEvaluationTest") {
            got.as_ref().err().map(|e| e.0) == t["expectErrorCode"].as_str()
        } else if types.contains("PositiveSyntaxTest") {
            got.is_ok()
        } else {
            let want = std::fs::read_to_string(dir.join(t["expect"].as_str().unwrap())).unwrap();
            if want.contains("_:") || got.as_ref().is_ok_and(|g| g.contains("_:")) {
                continue;
            }
            let lines = |s: &str| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>()
            };
            got.as_ref().is_ok_and(|g| lines(g) == lines(&want))
        };
        run += 1;
        if !ok {
            failed.push(format!("{} {}: {:?}", t["@id"], t["name"], got));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {run} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    assert!(run > 200, "only {run} tests ran");
}
