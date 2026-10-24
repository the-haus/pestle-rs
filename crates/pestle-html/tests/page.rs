//! The page pass on pages as sites write them, and pages in every encoding.

use pestle_html::page::{Page, PageOptions};
use pestle_html::{Extractor, ExtractorOptions, Output};
use serde_json::json;

fn extractor() -> Extractor {
    Extractor::new(
        &json!({ "h": "h1" }),
        &Default::default(),
        ExtractorOptions {
            page: Some(PageOptions::default()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// The page read whole, and in chunks of several sizes: the same either way.
fn read_with(html: &[u8], charset: Option<&str>) -> Output {
    let x = extractor();
    let url = Some("https://venue.example/agenda/");
    let whole = x.extract_with(html, url, charset);
    for size in [1usize, 3, 100, 1023, 1024, 4096] {
        let mut s = x.session_with(url, charset);
        for c in html.chunks(size) {
            s.write(c);
        }
        let chunked = s.finish();
        assert_eq!(chunked.charset, whole.charset, "chunks of {size}");
        assert_eq!(
            serde_json::to_value(chunked.page.as_ref().unwrap()).unwrap(),
            serde_json::to_value(whole.page.as_ref().unwrap()).unwrap(),
            "chunks of {size}"
        );
        assert_eq!(chunked.data, whole.data, "chunks of {size}");
    }
    whole
}

fn page(html: &str) -> Page {
    read_with(html.as_bytes(), None).page.unwrap()
}

const HEAD: &str = r#"<!DOCTYPE html>
<html lang="nl-NL"><head>
<meta charset="utf-8">
<base href="/nl/">
<title> Nora &amp; Co — Paradiso </title>
<meta name="description" content="  Een avond   met Nora ">
<meta property="og:title" content="Nora live">
<meta name="og:image" content="https://cdn.example/nora.jpg">
<meta name="twitter:card" content="summary_large_image">
<meta NAME="ROBOTS" content="noindex, Max-Image-Preview:large">
<meta name="googlebot" content="nofollow">
<meta http-equiv="refresh" content="30; URL='/nl/agenda'">
<meta name="empty" content="">
<meta content="no key">
<link rel="canonical" href="https://venue.example/nl/e/nora#top">
<link rel="canonical" href="https://venue.example/second">
<link rel="alternate" hreflang="en" href="/en/e/nora">
<link rel="alternate" hreflang="x-default" href="https://venue.example/e/nora">
<link rel="alternate" type="application/rss+xml" title="Agenda" href="/feed.xml">
<link rel="alternate" type="application/atom+xml" href="feed.atom">
<link rel="alternate" type="text/calendar" href="agenda.ics">
<link rel="amphtml" href="/amp/e/nora">
<link rel="icon" href="/favicon.ico" sizes="32x32">
<link rel="apple-touch-icon" href="/touch.png">
<link rel="manifest" href="/site.webmanifest">
<link rel="next" href="?page=2">
<link rel="stylesheet" href="/s.css">
<link rel="canonical">
</head>"#;

#[test]
fn the_head() {
    let p = page(&format!("{HEAD}<body></body></html>"));
    assert_eq!(p.charset, "UTF-8");
    assert_eq!(p.title.as_deref(), Some("Nora & Co — Paradiso"));
    assert_eq!(p.description.as_deref(), Some("Een avond met Nora"));
    assert_eq!(p.lang.as_deref(), Some("nl-NL"));
    assert_eq!(p.base.as_deref(), Some("https://venue.example/nl/"));
    assert_eq!(
        p.canonical.as_deref(),
        Some("https://venue.example/nl/e/nora"),
        "the first, without its fragment"
    );
    assert!(!p.robots.index && !p.robots.follow);
    assert_eq!(p.robots.directives, ["noindex", "max-image-preview:large", "nofollow"]);
    let r = p.refresh.unwrap();
    assert_eq!(
        (r.seconds, r.url.as_deref()),
        (30.0, Some("https://venue.example/nl/agenda"))
    );
    assert_eq!(p.meta["og:title"], "Nora live");
    assert_eq!(
        p.meta["og:image"], "https://cdn.example/nora.jpg",
        "OpenGraph written with name="
    );
    assert_eq!(p.meta["twitter:card"], "summary_large_image");
    assert!(p.meta.get("empty").is_none());
    let alts: Vec<(&str, Option<&str>)> = p
        .alternates
        .iter()
        .map(|a| (a.href.as_str(), a.hreflang.as_deref()))
        .collect();
    assert_eq!(
        alts,
        [
            ("https://venue.example/en/e/nora", Some("en")),
            ("https://venue.example/e/nora", Some("x-default"))
        ]
    );
    let feeds: Vec<&str> = p.feeds.iter().map(|f| f.href.as_str()).collect();
    assert_eq!(
        feeds,
        ["https://venue.example/feed.xml", "https://venue.example/nl/feed.atom"]
    );
    assert_eq!(p.feeds[0].title.as_deref(), Some("Agenda"));
    assert_eq!(p.calendars, ["https://venue.example/nl/agenda.ics"]);
    assert_eq!(p.amp.as_deref(), Some("https://venue.example/amp/e/nora"));
    assert_eq!(p.icons.len(), 2);
    assert_eq!(p.icons[0].sizes.as_deref(), Some("32x32"));
    assert_eq!(p.manifest.as_deref(), Some("https://venue.example/site.webmanifest"));
    assert_eq!(p.next.as_deref(), Some("https://venue.example/nl/?page=2"));
}

#[test]
fn the_body() {
    let p = page(&format!(
        r##"{HEAD}<body>
        <svg><title>an icon</title></svg>
        <h1>  Nora <small>&amp; Co</small></h1><h2></h2><h3>Line-up</h3>
        <a href="e/nora">Nora</a>
        <a href="e/nora#tickets">Tickets</a>
        <a href="https://shop.example/t/1" rel="nofollow sponsored"><img src="/logo.png" alt="Ticket shop"></a>
        <a href="/x" title="By title"></a>
        <a href="#top">top</a><a href="javascript:void(0)">js</a><a>no href</a><a href="">empty</a>
        <a href="mailto:Info@Venue.example?subject=Hi">mail</a><a href="mailto:a@x.example,%20b@x.example">two</a>
        <a href="tel:+31 20 626 4521">call</a><a href="tel:0">bad</a>
        <a href="webcal://venue.example/cal">calendar</a>
        <a href="?page=3" rel="next">next</a>
        <img data-src="/lazy.jpg" src="data:image/gif;base64,R0lGOD" alt=" Lazy " width="300px" height="200">
        <img srcset="/s.jpg 480w, /l.jpg 1200w">
        <img src="/logo.png">
        <img src="">
        <iframe src="https://www.youtube.com/embed/x"></iframe><iframe src="about:blank" data-src="https://maps.example/m"></iframe>
        <template><a href="/hidden">hidden</a><img src="/hidden.jpg"></template>
        </body></html>"##
    ));
    assert_eq!(p.title.as_deref(), Some("Nora & Co — Paradiso"), "not the svg's title");
    let heads: Vec<(u8, &str)> = p.headings.iter().map(|h| (h.level, h.text.as_str())).collect();
    assert_eq!(heads, [(1, "Nora & Co"), (3, "Line-up")]);
    let links: Vec<(&str, &str)> = p.links.iter().map(|l| (l.href.as_str(), l.text.as_str())).collect();
    assert_eq!(
        links,
        [
            ("https://venue.example/nl/e/nora", "Nora"),
            ("https://shop.example/t/1", "Ticket shop"),
            ("https://venue.example/x", "By title"),
            ("https://venue.example/cal", "calendar"),
            ("https://venue.example/nl/?page=3", "next"),
        ]
    );
    assert!(p.links[1].nofollow);
    assert_eq!(p.links[1].rel, ["nofollow", "sponsored"]);
    assert_eq!(p.emails, ["Info@Venue.example", "a@x.example", "b@x.example"]);
    assert_eq!(p.phones, ["+31 20 626 4521"]);
    assert_eq!(
        p.calendars,
        ["https://venue.example/nl/agenda.ics", "https://venue.example/cal"]
    );
    let images: Vec<&str> = p.images.iter().map(|i| i.src.as_str()).collect();
    assert_eq!(
        images,
        [
            "https://venue.example/logo.png",
            "https://venue.example/lazy.jpg",
            "https://venue.example/l.jpg"
        ]
    );
    assert_eq!(p.images[1].alt.as_deref(), Some("Lazy"));
    assert_eq!((p.images[1].width, p.images[1].height), (Some(300), Some(200)));
    assert_eq!(p.frames, ["https://www.youtube.com/embed/x", "https://maps.example/m"]);
}

#[test]
fn script_data() {
    let p = page(
        r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"event":{"name":"Nora"}}}}</script>
        <script id="__NUXT_DATA__" type="application/json">[{"a":1}]</script>
        <script type="application/ld+json">{"@type":"Event"}</script>
        <script type="application/json">{}</script>
        <script>window.__INITIAL_STATE__ = {"events":[1,2]}; var ga = function(){};</script>
        <script src="/app.js">window.__IGNORED__ = {"a":1}</script>
        <script type="text/template">window.__T__ = {"a":1}</script>
        <script>if (a == {}) {}; x = [1</script>"#,
    );
    let names: Vec<&str> = p.embedded.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["__NEXT_DATA__", "__NUXT_DATA__", "window.__INITIAL_STATE__"]);
    assert_eq!(p.embedded[0].value["props"]["pageProps"]["event"]["name"], "Nora");
    assert_eq!(p.embedded[2].value["events"], json!([1, 2]));
}

#[test]
fn fallbacks_and_odd_heads() {
    // No title: OpenGraph's; no description: OpenGraph's; no lang attribute: content-language.
    let p = page(
        r#"<meta property="og:title" content="OG"><meta property="og:description" content="D"><meta http-equiv="content-language" content="de, en"><meta property="og:url" content="/canon">"#,
    );
    assert_eq!(p.title.as_deref(), Some("OG"));
    assert_eq!(p.description.as_deref(), Some("D"));
    assert_eq!(p.lang.as_deref(), Some("de"));
    assert_eq!(p.canonical.as_deref(), Some("https://venue.example/canon"));
    // Two titles, an empty one first; robots "none".
    let p = page("<title> </title><title>Second</title><meta name=robots content=none>");
    assert_eq!(p.title.as_deref(), Some("Second"));
    assert!(!p.robots.index && !p.robots.follow);
    // A title never closed is the rest of the page.
    let p = page("<title>Unclosed <b>still title");
    assert_eq!(p.title.as_deref(), Some("Unclosed <b>still title"));
    // Nothing at all.
    let p = page("");
    assert!(p.title.is_none() && p.links.is_empty() && p.robots.index);
}

#[test]
fn limits() {
    let many: String = (0..6000)
        .map(|i| format!(r#"<a href="/e/{i}">e</a><img src="/i/{i}.jpg">"#))
        .collect();
    let p = page(&many);
    assert_eq!(p.links.len(), 5000);
    assert_eq!(p.images.len(), 2000);
    assert_eq!(p.warnings.len(), 2);
    let long = format!(
        "<h1>{}</h1><a href=/x>{}</a><title>{}</title>",
        "h".repeat(100_000),
        "t".repeat(100_000),
        "x".repeat(100_000)
    );
    let p = page(&long);
    assert!(p.headings[0].text.chars().count() <= 240 + 1);
    assert!(p.links[0].text.chars().count() <= 240 + 1);
    assert!(p.title.unwrap().chars().count() <= 1000 + 1);
}

// ── Encodings ────────────────────────────────────────────────────────────

const TEXT: &str = "Café crème — 5 €";

#[track_caller]
fn decoded(html: &[u8], declared: Option<&str>, charset: &str) {
    let out = read_with(html, declared);
    assert_eq!(out.charset, charset);
    let p = out.page.unwrap();
    assert_eq!(p.title.as_deref(), Some(TEXT), "{charset}");
    assert_eq!(out.data.to_json()["h"], TEXT, "{charset}");
}

fn doc(meta: &str) -> String {
    format!("<html><head>{meta}<title>{TEXT}</title></head><body><h1>{TEXT}</h1></body></html>")
}

#[test]
fn every_encoding() {
    let w1252_src = doc(r#"<meta charset="windows-1252">"#);
    let (w1252, _, _) = encoding_rs::WINDOWS_1252.encode(&w1252_src);
    decoded(&w1252, None, "windows-1252");
    // The server's word, a page's label for the same, or no word at all.
    let plain_src = doc("");
    let (plain, _, _) = encoding_rs::WINDOWS_1252.encode(&plain_src);
    decoded(&plain, Some("ISO-8859-1"), "windows-1252");
    decoded(&plain, None, "windows-1252");
    let latin_src = doc(r#"<meta http-equiv="Content-Type" content="text/html; charset=latin1">"#);
    let (latin, _, _) = encoding_rs::WINDOWS_1252.encode(&latin_src);
    decoded(&latin, None, "windows-1252");
    // UTF-8 with and without a byte order mark, and a server that says otherwise (the BOM wins).
    decoded(doc("").as_bytes(), None, "UTF-8");
    decoded(
        &[b"\xef\xbb\xbf".as_slice(), doc("").as_bytes()].concat(),
        Some("windows-1252"),
        "UTF-8",
    );
    // UTF-16 by its byte order mark.
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(doc("").encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    decoded(&utf16, None, "UTF-16LE");
}

#[test]
fn far_eastern_and_cyrillic() {
    for (enc, text) in [
        (encoding_rs::SHIFT_JIS, "東京ドーム 2026年11月14日"),
        (encoding_rs::EUC_KR, "서울 콘서트"),
        (encoding_rs::GBK, "北京音乐节"),
        (encoding_rs::WINDOWS_1251, "Концерт в Москве"),
        (encoding_rs::KOI8_R, "Фестиваль"),
    ] {
        let html = format!("<meta charset=\"{}\"><title>{text}</title><h1>{text}</h1>", enc.name());
        let (bytes, _, unmappable) = enc.encode(&html);
        assert!(!unmappable, "{}", enc.name());
        let out = read_with(&bytes, None);
        assert_eq!(out.charset, enc.name());
        assert_eq!(out.page.unwrap().title.as_deref(), Some(text), "{}", enc.name());
        assert_eq!(out.data.to_json()["h"], text);
    }
}

#[test]
fn bot_walls() {
    for (html, wall) in [
        (
            "<title>Just a moment...</title><script>window._cf_chl_opt={cvId: '3'}</script>",
            "cloudflare",
        ),
        (r#"<div id="challenge-running"></div>"#, "cloudflare"),
        (
            r#"<script src="/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1"></script>"#,
            "cloudflare",
        ),
        (
            r#"<iframe src="https://geo.captcha-delivery.com/captcha/?initialCid=x"></iframe>"#,
            "datadome",
        ),
        (r#"<div id="px-captcha"></div>"#, "perimeterx"),
        (
            r#"<script src="/_Incapsula_Resource?SWJIYLWA=x"></script>"#,
            "incapsula",
        ),
        ("<title>Access Denied</title><h1>Access Denied</h1>", "akamai"),
        ("<title>Are you a robot?</title>", "captcha"),
    ] {
        assert_eq!(page(html).blocked.as_deref(), Some(wall), "{html}");
    }
    assert_eq!(
        page("<title>Agenda</title><a href=/e>e</a><script src=/app.js></script>").blocked,
        None
    );
    // Soft hyphens and zero-width characters are not text.
    let p = page("<a href=/info>Bezoekers\u{ad}info</a><h2>Line\u{200b}up</h2>");
    assert_eq!(p.links[0].text, "Bezoekersinfo");
    assert_eq!(p.headings[0].text, "Lineup");
}
