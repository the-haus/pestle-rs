//! pestle alone on the html bench's workload, in a loop (for a profiler).
use serde_json::json;
fn main() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../.work");
    let pages: Vec<Vec<u8>> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .filter(|e| std::env::var("PAGE").map_or(true, |p| e.file_name().to_string_lossy().starts_with(&p)))
        .map(|e| std::fs::read(e.path()).unwrap())
        .collect();
    let x = pestle_html::Extractor::new(&json!({
        "links": { "select": "a[href]", "all": true, "fields": { "href": { "attr": "href", "collapse": false }, "text": { "text": "raw" } } },
        "headings": { "select": "h2, h3", "all": true, "text": "raw" },
        "images": { "select": "img[src]", "attr": "src", "all": true, "collapse": false },
        "title": { "select": "h1", "text": "raw" },
    }), &Default::default(), Default::default()).unwrap();
    let n: usize = std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let mut k = 0;
    for _ in 0..n {
        for p in &pages {
            k += x.extract(p, None).stats.elements;
        }
    }
    println!("{k}");
}
