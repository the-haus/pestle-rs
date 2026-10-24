//! The page pass on saved pages: `cargo run --release -p pestle-html --example page_probe [file]`.
use pestle_html::page::PageOptions;
use pestle_html::{Extractor, ExtractorOptions};

fn main() {
    let x = Extractor::new(
        &serde_json::json!({}),
        &Default::default(),
        ExtractorOptions {
            page: Some(PageOptions::default()),
            ..Default::default()
        },
    )
    .unwrap();
    for f in std::env::args().skip(1) {
        let html = std::fs::read(&f).unwrap();
        let t = std::time::Instant::now();
        let out = x.extract(&html, Some("https://example.nl/"));
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let p = out.page.unwrap();
        println!("== {f} ({} kB, {ms:.2} ms, {})", html.len() / 1000, p.charset);
        println!(
            "  title {:?}\n  description {:?}\n  lang {:?} canonical {:?} robots {}/{} refresh {:?}",
            p.title,
            p.description.as_deref().map(|d| &d[..d.len().min(80)]),
            p.lang,
            p.canonical,
            p.robots.index,
            p.robots.follow,
            p.refresh.map(|r| r.seconds)
        );
        println!(
            "  meta {} alternates {} feeds {} calendars {:?} icons {} next {:?}",
            p.meta.len(),
            p.alternates.len(),
            p.feeds.len(),
            p.calendars,
            p.icons.len(),
            p.next
        );
        println!(
            "  links {} (first: {:?}) images {} headings {} frames {:?} emails {:?} phones {:?}",
            p.links.len(),
            p.links.iter().take(3).map(|l| (&l.href, &l.text)).collect::<Vec<_>>(),
            p.images.len(),
            p.headings.len(),
            p.frames,
            p.emails,
            p.phones
        );
        println!(
            "  embedded {:?} warnings {:?}",
            p.embedded
                .iter()
                .map(|e| (e.name.clone(), serde_json::to_string(&e.value).unwrap().len()))
                .collect::<Vec<_>>(),
            p.warnings
        );
    }
}
