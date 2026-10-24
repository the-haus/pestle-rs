#[cfg(feature = "grammar")]
use std::sync::Arc;

#[cfg(feature = "grammar")]
use pestle_grammar::Grammar;
use serde_json::{json, Value};

use crate::{Extractor, ExtractorOptions, Grammars, Output, SkeletonOptions, StripOptions};

const AGENDA: &str = r#"<!doctype html>
<html lang="nl"><head>
  <meta charset="utf-8"><title>Agenda &middot; Paradiso</title>
  <base href="https://www.paradiso.example/nl/">
  <meta property="og:title" content="Agenda">
  <script type="application/ld+json">{"@type":"MusicEvent","name":"Nora &amp; Co","startDate":"2026-11-14T20:00"}</script>
</head><body>
  <main class="agenda">
    <article class="event" data-id="101">
      <h2 class="title"><a href="programma/nora-14-11#tickets">Nora <b>&amp;</b> Co</a></h2>
      <p class="date">za 14 nov 2026 · 20:00</p>
      <ul class="acts"><li>Nora<li>The Cozies<li class="support">DJ Lu</ul>
      <span class="price">€ 25,-</span>
      <span class="soldout"></span>
    </article>
    <article class="event" data-id="102">
      <h2 class="title"><a href="programma/mira">Mira</a></h2>
      <p class="date">zo 15 nov 2026 · 21:30</p>
      <ul class="acts"><li>Mira</ul>
      <span class="price">gratis</span>
    </article>
    <article class="event cancelled" data-id="103">
      <h2 class="title"><a href="mailto:x@y">Gone</a></h2>
      <ul class="acts"></ul>
    </article>
  </main>
  <script>var x = "<article class='event'>not one</article>";</script>
</body></html>"#;

fn run(spec: Value, grammars: &Grammars, options: ExtractorOptions, html: &str) -> Output {
    let x = Extractor::new(&spec, grammars, options).unwrap_or_else(|e| panic!("{e}"));
    let whole = x.extract(html.as_bytes(), Some("https://www.paradiso.example/agenda"));
    // The same in chunks of 1, 2, 3 … 7 bytes.
    for size in 1..8 {
        let mut s = x.session(Some("https://www.paradiso.example/agenda"));
        for chunk in html.as_bytes().chunks(size) {
            s.write(chunk);
        }
        let chunked = s.finish();
        assert_eq!(chunked.data.to_json(), whole.data.to_json(), "chunks of {size}");
        assert_eq!(chunked.skeleton, whole.skeleton, "chunks of {size}");
        assert_eq!(chunked.stripped, whole.stripped, "chunks of {size}");
    }
    whole
}

fn data(spec: Value, html: &str) -> Value {
    run(spec, &Grammars::new(), ExtractorOptions::default(), html)
        .data
        .to_json()
}

#[test]
fn fields_text_attributes_urls() {
    let v = data(
        json!({
            "title": "title",
            "og": "meta[property='og:title']@content",
            "first": "article h2",
            "links": { "select": "article h2 a", "attr": "href", "url": "http", "all": true },
            "raw": { "select": "article h2 a", "attr": "href", "url": true },
            "ids": ["article.event@data-id"],
            "cancelled": { "select": "article.cancelled", "exists": true },
            "missing": { "select": "article.nope", "exists": true },
            "nothing": "blink",
            "fallback": { "select": "blink", "default": "none" },
        }),
        AGENDA,
    );
    assert_eq!(
        v,
        json!({
            "title": "Agenda · Paradiso",
            "og": "Agenda",
            "first": "Nora & Co",
            "links": ["https://www.paradiso.example/nl/programma/nora-14-11", "https://www.paradiso.example/nl/programma/mira"],
            "raw": "https://www.paradiso.example/nl/programma/nora-14-11#tickets",
            "ids": ["101", "102", "103"],
            "cancelled": true,
            "missing": false,
            "nothing": null,
            "fallback": "none",
        })
    );
}

#[test]
fn nested_scopes_and_values() {
    let v = data(
        json!({
            "events": {
                "select": "article.event",
                "all": true,
                "fields": {
                    "id": { "attr": "data-id" },
                    "title": "h2",
                    "acts": ["li"],
                    "headliner": "> ul > li:first-child",
                    "time": { "select": ".date", "pattern": "(\\d{1,2}:\\d{2})" },
                    "price": { "select": ".price", "number": true },
                    "soldOut": { "select": ".soldout", "exists": true },
                }
            }
        }),
        AGENDA,
    );
    assert_eq!(
        v["events"],
        json!([
            { "id": "101", "title": "Nora & Co", "acts": ["Nora", "The Cozies", "DJ Lu"], "headliner": "Nora", "time": "20:00", "price": 25.0, "soldOut": true },
            { "id": "102", "title": "Mira", "acts": ["Mira"], "headliner": "Mira", "time": "21:30", "price": null, "soldOut": false },
            { "id": "103", "title": "Gone", "acts": [], "headliner": null, "time": null, "price": null, "soldOut": false },
        ])
    );
}

#[test]
fn json_ld_raw_text_and_html() {
    let v = data(
        json!({
            "ld": { "select": "script[type='application/ld+json']", "text": "raw", "json": true, "all": true },
            "scriptText": "body > script",
            "card": { "select": "article.event:nth-of-type(2) h2", "html": true },
            "own": { "select": "h2 a", "text": "own" },
        }),
        AGENDA,
    );
    assert_eq!(
        v["ld"],
        json!([{ "@type": "MusicEvent", "name": "Nora &amp; Co", "startDate": "2026-11-14T20:00" }])
    );
    assert_eq!(
        v["scriptText"],
        json!("var x = \"<article class='event'>not one</article>\";")
    );
    assert_eq!(
        v["card"],
        json!(r#"<h2 class="title"><a href="programma/mira">Mira</a></h2>"#)
    );
    assert_eq!(v["own"], json!("Nora Co"));
}

#[test]
fn settled_later_pseudo_classes() {
    let v = data(
        json!({
            "lastAct": "article:first-child li:last-child",
            "lastActs": ["li:last-child"],
            "onlyAct": ["li:only-child"],
            "eventsWithSupport": { "select": "article:has(li.support)", "all": true, "fields": { "t": "h2" } },
            "emptyLists": { "select": "ul:empty", "all": true, "exists": true },
            "mira": { "select": "article:contains('Mira')", "attr": "data-id" },
            "secondToLast": { "select": "article:nth-last-child(2)", "attr": "data-id" },
        }),
        AGENDA,
    );
    assert_eq!(v["lastAct"], json!("DJ Lu"));
    assert_eq!(v["lastActs"], json!(["DJ Lu", "Mira"]));
    assert_eq!(v["onlyAct"], json!(["Mira"]));
    assert_eq!(v["eventsWithSupport"], json!([{ "t": "Nora & Co" }]));
    assert_eq!(v["emptyLists"], json!([true]));
    assert_eq!(v["mira"], json!("102"));
    assert_eq!(v["secondToLast"], json!("102"));
}

#[test]
fn ancestor_with_settled_later_condition_is_an_error() {
    for sel in ["article:first-child li:last-child x", "article:has(li.support) h2"] {
        let x = Extractor::new(&json!({ "a": sel }), &Grammars::new(), ExtractorOptions::default());
        assert!(x.is_err(), "{sel}");
    }
    let lenient = ExtractorOptions {
        lenient: true,
        ..ExtractorOptions::default()
    };
    let x = Extractor::new(
        &json!({ "a": "li:last-child x", "b": "h2", "c": { "select": "li", "pattern": "(" } }),
        &Grammars::new(),
        lenient,
    )
    .unwrap();
    assert_eq!(x.warnings().len(), 2, "{:?}", x.warnings());
    assert_eq!(
        x.extract(AGENDA.as_bytes(), None).data.to_json(),
        json!({ "b": "Nora & Co" })
    );
}

#[cfg(feature = "grammar")]
#[test]
fn grammars_shape_values() {
    let date = Grammar::new(
        r#"
        WHITESPACE = _{ " " }
        when    = { weekday? ~ #day = day ~ #month = month ~ #year = year ~ "·" ~ #time = time }
        weekday = @{ ("ma" | "di" | "wo" | "do" | "vr" | "za" | "zo") }
        day     = @{ ASCII_DIGIT{1,2} }
        month   = @{ ASCII_ALPHA+ }
        year    = @{ ASCII_DIGIT{4} }
        time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }
        "#,
    )
    .unwrap();
    let money = Grammar::new(r#"amount = @{ ASCII_DIGIT+ ~ ("," ~ (ASCII_DIGIT{2} | "-"))? }"#).unwrap();
    let mut grammars = Grammars::new();
    grammars.insert("date".into(), Arc::new(date));
    grammars.insert("money".into(), Arc::new(money));
    let out = run(
        json!({
            "dates": { "select": ".date", "all": true, "parse": { "grammar": "date", "numbers": ["day", "year"] } },
            "time": { "select": ".date", "parse": "date.time" },
            "prices": { "select": "main", "parse": { "grammar": "money", "all": true } },
            "found": { "select": ".price", "parse": { "grammar": "money", "find": true } },
            "tree": { "select": ".date", "parse": { "grammar": "date", "rule": "time", "find": true, "tree": true } },
        }),
        &grammars,
        ExtractorOptions::default(),
        AGENDA,
    );
    assert_eq!(
        out.data.to_json()["dates"],
        json!([
            { "day": 14.0, "month": "nov", "year": 2026.0, "time": "20:00" },
            { "day": 15.0, "month": "nov", "year": 2026.0, "time": "21:30" },
        ])
    );
    assert_eq!(
        out.data.to_json()["time"],
        json!(null),
        "anchored: the text starts with a weekday"
    );
    assert!(
        out.errors.iter().any(|e| e.starts_with("time: grammar \"date\"")),
        "{:?}",
        out.errors
    );
    assert_eq!(out.data.to_json()["found"], json!("25,-"));
    assert_eq!(
        out.data.to_json()["prices"].as_array().map(Vec::len),
        Some(9),
        "14 2026 20 00 25,- 15 2026 21 30"
    );
    assert_eq!(out.data.to_json()["tree"], json!({ "rule": "time", "text": "20:00" }));
}

#[test]
fn emitted_items_stream_out() {
    let x = Extractor::new(
        &json!({ "events": { "select": "article", "all": true, "emit": true, "fields": { "t": "h2" } }, "title": "title" }),
        &Grammars::new(),
        ExtractorOptions::default(),
    )
    .unwrap();
    // A charset stated streams from the first byte (else the first kilobyte decides it).
    let mut s = x.session_with(None, Some("utf-8"));
    let mut seen = Vec::new();
    let cut = AGENDA.find("data-id=\"102\"").unwrap();
    s.write(&AGENDA.as_bytes()[..cut]);
    seen.push(s.take().len());
    s.write(&AGENDA.as_bytes()[cut..]);
    let rest = s.take();
    seen.push(rest.len());
    let out = s.finish();
    assert_eq!(seen, [1, 2], "the first article is out before the second is read");
    assert_eq!(
        (rest[1].0.as_str(), rest[1].1.to_json()),
        ("events", json!({ "t": "Gone" }))
    );
    assert_eq!(out.data.to_json(), json!({ "title": "Agenda · Paradiso" }));
    assert!(out.items.is_empty());

    // The last item closes with the document: it comes with the output.
    let x = Extractor::new(
        &json!({ "acts": { "select": "li", "all": true, "emit": true } }),
        &Grammars::new(),
        ExtractorOptions::default(),
    )
    .unwrap();
    // A charset stated streams from the first byte (else the first kilobyte decides it).
    let mut s = x.session_with(None, Some("utf-8"));
    s.write(b"<ul><li>A<li>B");
    assert_eq!(
        s.take().into_iter().map(|(k, v)| (k, v.to_json())).collect::<Vec<_>>(),
        [("acts".to_owned(), json!("A"))]
    );
    assert_eq!(
        s.finish()
            .items
            .into_iter()
            .map(|(k, v)| (k, v.to_json()))
            .collect::<Vec<_>>(),
        [("acts".to_owned(), json!("B"))]
    );
}

#[test]
fn skeleton_and_strip_in_the_same_pass() {
    let options = ExtractorOptions {
        skeleton: Some(SkeletonOptions {
            paths: true,
            ..SkeletonOptions::default()
        }),
        strip: Some(StripOptions {
            max_alike: 2,
            ..StripOptions::default()
        }),
        ..ExtractorOptions::default()
    };
    let out = run(json!({ "t": "h2" }), &Grammars::new(), options, AGENDA);
    let skeleton = out.skeleton.unwrap();
    let paths = skeleton.paths.unwrap();
    assert!(
        paths.contains(&"main.agenda>article.event>h2.title>a".to_owned()),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"main.agenda>article.cancelled.event".to_owned()),
        "{paths:?}"
    );
    assert!(!paths.iter().any(|p| p.contains("script")));
    assert_eq!(skeleton.hash.len(), 64);
    let html = out.stripped.unwrap().html;
    assert!(html.starts_with("<title>Agenda · Paradiso</title>\n<meta property=\"og:title\" content=\"Agenda\">\n<script type=\"application/ld+json\">"), "{html}");
    assert!(html.contains("<article class=\"event\" data-id=\"101\"><h2 class=\"title\"><a href=\"programma/nora-14-11#tickets\">Nora<b>&amp;</b>Co</a></h2>"), "{html}");
    assert!(!html.contains("soldout"), "an empty span is dropped: {html}");
    assert!(
        html.contains("<li>Nora</li><li>The Cozies</li><li class=\"support\">DJ Lu</li>"),
        "{html}"
    );
    assert!(!html.contains("var x"), "{html}");
}

#[test]
fn strip_collapses_alike_siblings_and_keeps_the_budget() {
    let items: String = (0..50)
        .map(|i| format!("<div class=\"card c{i}\"><span>Item {i}</span></div>"))
        .collect();
    let cards: String = (0..50).map(|i| format!("<li class=card>Card {i}</li>")).collect();
    let html = format!("<body><section>{items}</section><ul>{cards}</ul><p>end</p></body>");
    let options = ExtractorOptions {
        strip: Some(StripOptions::default()),
        ..ExtractorOptions::default()
    };
    let out = run(json!({}), &Grammars::new(), options, &html);
    let stripped = out.stripped.unwrap();
    assert!(
        stripped
            .html
            .contains("<li class=\"card\">Card 3</li><!-- 46 more alike --></ul><p>end</p>"),
        "{}",
        stripped.html
    );
    let budget = ExtractorOptions {
        strip: Some(StripOptions {
            max_chars: 200,
            ..StripOptions::default()
        }),
        ..ExtractorOptions::default()
    };
    let out = run(json!({}), &Grammars::new(), budget, &html);
    let stripped = out.stripped.unwrap();
    assert!(stripped.truncated);
    assert!(stripped.html.ends_with("</section>"), "{}", stripped.html);
    assert!(stripped.html.len() < 260);
}

#[test]
fn deep_and_hostile_input_stays_bounded() {
    let deep = "<div>".repeat(100_000) + "x";
    let out = run(
        json!({ "x": "div" }),
        &Grammars::new(),
        ExtractorOptions::default(),
        &deep,
    );
    assert_eq!(out.stats.max_depth, crate::DEFAULT_MAX_DEPTH);
    assert_eq!(out.data.to_json()["x"], json!("x"));
    let junk = "<<<>>>&&&#;<a\u{0} =\"\"\"><!--<!--</b></b></script><script>".repeat(100);
    run(
        json!({ "a": ["a"] }),
        &Grammars::new(),
        ExtractorOptions::default(),
        &junk,
    );
}
