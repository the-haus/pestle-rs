//! pestle on saved pages (bench/.work/*.html), best of several runs.
//!
//!     cargo run --release -p pestle-html --example pages [dir]
use std::time::Instant;

use pestle_html::{Extractor, ExtractorOptions, SkeletonOptions};
use serde_json::json;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/.work").into());
    let mut pages: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .expect("a directory of pages")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .filter(|(_, b)| b.len() > 10_000)
        .collect();
    pages.sort();
    let spec = json!({
        "title": "h1",
        "ogTitle": "meta[property='og:title']@content",
        "description": "meta[name=description]@content",
        "links": { "select": "a[href]", "all": true, "fields": { "url": { "attr": "href", "url": "http" }, "text": { "max": 120 } } },
        "images": { "select": "img[src]", "attr": "src", "url": "http", "all": true },
        "times": ["time[datetime]@datetime"],
        "headings": ["h2, h3"],
        "jsonLd": { "select": "script[type^='application/ld+json' i]", "text": "raw", "json": true, "all": true },
        "feeds": ["link[type='text/calendar' i], a[href$='.ics' i]@href"],
    });
    let read = Extractor::new(
        &spec,
        &Default::default(),
        ExtractorOptions {
            skeleton: Some(SkeletonOptions::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let parse = Extractor::new(&json!({}), &Default::default(), ExtractorOptions::default()).unwrap();
    let runs: usize = std::env::var("RUNS").ok().and_then(|r| r.parse().ok()).unwrap_or(7);
    let best = |x: &Extractor, html: &[u8]| {
        let mut b = f64::MAX;
        for _ in 0..runs {
            let t = Instant::now();
            std::hint::black_box(x.extract(html, Some("https://venue.example/agenda/")));
            b = b.min(t.elapsed().as_secs_f64());
        }
        b
    };
    let (mut total_read, mut total_parse, mut bytes) = (0.0, 0.0, 0usize);
    for (name, html) in &pages {
        let r = best(&read, html);
        let p = best(&parse, html);
        total_read += r;
        total_parse += p;
        bytes += html.len();
        println!(
            "{name:28} {:>6} kB  read {:6.2} ms  parse {:6.2} ms",
            html.len() / 1024,
            r * 1e3,
            p * 1e3
        );
    }
    let mb = bytes as f64 / 1048576.0;
    println!(
        "total: read {:.1} ms ({:.0} MB/s), parse {:.1} ms ({:.0} MB/s)",
        total_read * 1e3,
        mb / total_read,
        total_parse * 1e3,
        mb / total_parse
    );
}
