//! How fast structured data is read: the pages in `bench/.work/` (and its `events/`), then
//! synthetic pages of many items. `cargo run --release -p pestle-html --example structured_bench [filter]`

use std::time::Instant;

use pestle_html::structured::StructuredOptions;
use pestle_html::{Extractor, ExtractorOptions};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let x = Extractor::new(
        &serde_json::json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(StructuredOptions::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let page_x = Extractor::new(
        &serde_json::json!({}),
        &Default::default(),
        ExtractorOptions {
            page: Some(Default::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let plain = Extractor::new(&serde_json::json!({}), &Default::default(), ExtractorOptions::default()).unwrap();
    let mut pages: Vec<(String, Vec<u8>)> = Vec::new();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/.work");
    for dir in [root.to_owned(), format!("{root}/events")] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "html"))
                .collect();
            files.sort();
            for f in files {
                pages.push((
                    f.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&f).unwrap(),
                ));
            }
        }
    }
    let md = r#"<div itemscope itemtype="https://schema.org/Event"><b itemprop="name">A</b><span itemprop="location" itemscope itemtype="https://schema.org/Place"><i itemprop="name">P</i></span></div>"#;
    let rdfa = r#"<div vocab="https://schema.org/" typeof="Event"><b property="name">A</b><span property="location" typeof="Place"><i property="name">P</i></span></div>"#;
    let ld = r#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"Event","name":"A","location":{"@type":"Place","name":"P"}}</script>"#;
    pages.push(("synthetic: 20k microdata items".into(), md.repeat(20_000).into_bytes()));
    pages.push(("synthetic: 20k RDFa items".into(), rdfa.repeat(20_000).into_bytes()));
    pages.push(("synthetic: 250 JSON-LD blocks".into(), ld.repeat(250).into_bytes()));
    // `SECS=n`: read the chosen pages over and over for n seconds (for a profiler).
    if let Some(secs) = std::env::var("SECS").ok().and_then(|s| s.parse::<f64>().ok()) {
        let t = Instant::now();
        while t.elapsed().as_secs_f64() < secs {
            for (name, html) in pages.iter().filter(|(n, _)| n.contains(&filter)) {
                let _ = (name, x.extract(html, Some("https://v.example/")));
            }
        }
        return;
    }
    let (mut total_bytes, mut total_s, mut total_plain) = (0usize, 0f64, 0f64);
    for (name, html) in &pages {
        if !name.contains(&filter) {
            continue;
        }
        let reps = (8_000_000 / html.len().max(1)).clamp(1, 200);
        let time = |e: &Extractor| {
            let t = Instant::now();
            let mut n = 0;
            for _ in 0..reps {
                n += e
                    .extract(html, Some("https://v.example/"))
                    .structured
                    .map_or(0, |s| s.items.len());
            }
            (t.elapsed().as_secs_f64() / reps as f64, n / reps)
        };
        let (s, items) = time(&x);
        let (p, _) = time(&plain);
        let (pg, _) = time(&page_x);
        total_bytes += html.len();
        total_s += s;
        total_plain += p;
        println!(
            "{name:<48} {:>7} kB {:>9.2} ms  {:>6.0} MB/s  ({:>5.2} ms without structured data, {:>5.2} ms page)  {items} items",
            html.len() / 1000,
            s * 1e3,
            html.len() as f64 / s / 1e6,
            p * 1e3,
            pg * 1e3
        );
    }
    println!(
        "total: {:.1} MB in {:.1} ms ({:.0} MB/s); without structured data {:.1} ms",
        total_bytes as f64 / 1e6,
        total_s * 1e3,
        total_bytes as f64 / total_s / 1e6,
        total_plain * 1e3
    );
}
