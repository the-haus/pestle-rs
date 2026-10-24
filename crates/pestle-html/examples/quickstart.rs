//! The examples of the README, run: a spec, a streamed document with emitted items, the page,
//! structured data and accessibility snapshot in one pass, robots.txt and a sitemap.
//!
//!     cargo run -p pestle-html --example quickstart
use pestle_html::aria::{AriaMode, AriaOptions};
use pestle_html::page::PageOptions;
use pestle_html::robots::Robots;
use pestle_html::structured::StructuredOptions;
use pestle_html::{sitemap, Extractor, ExtractorOptions};
use serde_json::json;

const PAGE: &str = r#"<!doctype html><html lang=en><head><title>Agenda</title>
<script type="application/ld+json">{"@type":"MusicEvent","name":"Nora & Co","startDate":"2026-11-14T20:00"}</script>
</head><body><nav aria-label=Main><a href="/">Home</a></nav>
<article class=event><h2><a href="/e/nora">Nora &amp; Co</a></h2><p class=date>za 14 nov</p><span class=price>€ 25,-</span></article>
<article class=event><h2><a href="/e/mira">Mira</a></h2><p class=date>zo 15 nov</p></article>
<button>Buy tickets</button></body></html>"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. A spec, compiled once; an Extractor is cheap to clone and share across threads.
    let spec = json!({
        "title": "title",
        "events": {
            "select": "article.event",
            "all": true,
            "fields": {
                "title": "h2",
                "url": { "select": "h2 a", "attr": "href", "url": "http" },
                "price": { "select": ".price", "number": true },
            },
        },
    });
    let x = Extractor::new(&spec, &Default::default(), ExtractorOptions::default())?;
    let out = x.extract(PAGE.as_bytes(), Some("https://venue.example/agenda"));
    // Values are pestle's own compact `Value`; `to_json()` gives a serde_json::Value.
    let data = out.data.to_json();
    assert_eq!(data["events"][0]["url"], json!("https://venue.example/e/nora"));
    assert_eq!(data["events"][0]["price"], json!(25.0));
    println!("{}", serde_json::to_string_pretty(&data)?);

    // 2. Streamed: chunks of any size, cut anywhere; items handed over as they close.
    let streaming = Extractor::new(
        &json!({ "events": { "select": "article.event", "all": true, "emit": true, "fields": { "title": "h2" } } }),
        &Default::default(),
        ExtractorOptions::default(),
    )?;
    let mut session = streaming.session(Some("https://venue.example/agenda"));
    let mut titles = Vec::new();
    for chunk in PAGE.as_bytes().chunks(7) {
        session.write(chunk);
        for (_field, item) in session.take() {
            titles.push(item.to_json()["title"].clone());
        }
    }
    let rest = session.finish();
    titles.extend(rest.items.into_iter().map(|(_, item)| item.to_json()["title"].clone()));
    assert_eq!(titles, [json!("Nora & Co"), json!("Mira")]);

    // 3. The page, its structured data and its accessibility snapshot, in the same pass.
    let everything = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            page: Some(PageOptions::default()),
            structured: Some(StructuredOptions::default()),
            aria: Some(AriaOptions {
                mode: AriaMode::Ai,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let out = everything.extract(PAGE.as_bytes(), Some("https://venue.example/agenda"));
    let page = out.page.expect("page pass");
    let structured = out.structured.expect("structured pass");
    let aria = out.aria.expect("aria pass");
    assert_eq!(page.title.as_deref(), Some("Agenda"));
    assert_eq!(page.links[0].href, "https://venue.example/");
    assert_eq!(structured.items[0]["@type"], json!("MusicEvent"));
    assert!(aria.yaml.contains("button \"Buy tickets\" [ref="));
    println!("{}", aria.yaml);
    for r in &aria.refs {
        println!("{}: {}", r.id, r.selector);
    }

    // 4. The crawler's half.
    let robots = Robots::parse("User-agent: *\nDisallow: /private/\n", "mybot");
    assert!(robots.allows("https://venue.example/agenda"));
    assert!(!robots.allows("https://venue.example/private/x"));
    let map = sitemap::read(
        b"<urlset><url><loc>/e/nora</loc><lastmod>2026-10-01</lastmod></url></urlset>",
        Some("https://venue.example/sitemap.xml"),
    )?;
    assert_eq!(map.entries[0].url, "https://venue.example/e/nora");
    Ok(())
}
