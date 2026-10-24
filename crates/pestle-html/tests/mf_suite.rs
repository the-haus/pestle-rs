//! The official microformats tests (https://github.com/microformats/tests), when a checkout is at
//! `bench/.work/suites/mf-tests`: each case's microformats2 JSON must be pestle's.

use std::path::{Path, PathBuf};

use pestle_html::structured::StructuredOptions;
use pestle_html::{Extractor, ExtractorOptions};
use serde_json::{json, Value};

/// Contradicts the value-class pattern ("If the offset uses XX:YY format, remove the colon") and
/// the suite's own `h-event/time`, `h-event/concatenate` and `hcalendar/time`.
const KNOWN: &[&str] = &["microformats-v2-unit/value/value-dt.html"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "html") {
            out.push(p);
        }
    }
}

#[test]
fn the_microformats_tests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/.work/suites/mf-tests/tests");
    if !root.is_dir() {
        eprintln!("no microformats tests checkout; skipped");
        return;
    }
    let opts = StructuredOptions {
        json_ld: false,
        microdata: false,
        rdfa: false,
        raw: true,
        ..Default::default()
    };
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(opts),
            ..Default::default()
        },
    )
    .unwrap();
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let (mut run, mut failed) = (0, Vec::new());
    for f in files {
        let Ok(expected) = std::fs::read_to_string(f.with_extension("json")) else {
            continue;
        };
        let Ok(expected) = serde_json::from_str::<Value>(&expected) else {
            continue;
        };
        let name = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let base = if name.contains("v2-unit") {
            "http://example.test"
        } else {
            "http://example.com/"
        };
        let html = std::fs::read(&f).unwrap();
        let got = x
            .extract(&html, Some(base))
            .structured
            .unwrap()
            .raw
            .unwrap()
            .microformats;
        let pick = |v: &Value| json!({ "items": v["items"], "rels": v["rels"], "rel-urls": v["rel-urls"] });
        run += 1;
        if pick(&got) != pick(&expected) && !KNOWN.contains(&name.as_str()) {
            failed.push(name);
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {run} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    assert!(run >= 140, "only {run} cases ran");
}
