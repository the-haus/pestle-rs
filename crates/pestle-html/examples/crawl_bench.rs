//! robots.txt and sitemap speed, natively: `cargo run --release -p pestle-html --example crawl_bench`.
use std::time::Instant;

use pestle_html::robots::Robots;
use pestle_html::sitemap;

fn main() {
    let mut sm = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">"#,
    );
    for i in 0..50_000 {
        sm.push_str(&format!("<url><loc>https://v.example/agenda/event-{i}?lang=nl&amp;ref=sitemap</loc><lastmod>2026-11-{:02}T10:00:00+01:00</lastmod><changefreq>daily</changefreq><priority>0.{}</priority><image:image><image:loc>https://cdn.example/{i}.jpg</image:loc></image:image></url>\n", i % 28 + 1, i % 10));
    }
    sm.push_str("</urlset>");
    let secs = std::env::var("SECS").ok().and_then(|s| s.parse::<f64>().ok());
    let t = Instant::now();
    let mut rounds = 0;
    let n = loop {
        let n = sitemap::read(sm.as_bytes(), None).unwrap().entries.len();
        rounds += 1;
        if secs.is_none_or(|s| t.elapsed().as_secs_f64() > s) && rounds >= 3 {
            break n;
        }
    };
    println!(
        "sitemap: {n} entries, {:.1} ms a read",
        t.elapsed().as_secs_f64() * 1e3 / rounds as f64
    );

    let mut txt =
        String::from("User-agent: *\nDisallow: /admin\nDisallow: /*?sort=\nDisallow: /*.pdf$\nAllow: /admin/public\n");
    for i in 0..60 {
        txt.push_str(&format!("Disallow: /section-{i}/*/private\n"));
    }
    txt.push_str("User-agent: festimapbot\nDisallow: /tickets\nAllow: /tickets/info\n");
    let r = Robots::parse(&txt, "festimapbot");
    let urls: Vec<String> = (0..100_000)
        .map(|i| {
            format!(
                "https://v.example/section-{}/x/{}/page-{i}?sort={}",
                i % 80,
                if i % 3 == 0 { "private" } else { "public" },
                i % 5
            )
        })
        .collect();
    let t = Instant::now();
    let allowed = urls.iter().filter(|u| r.allows(u)).count();
    println!(
        "robots: {allowed} of 100000 allowed, {:.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
}
