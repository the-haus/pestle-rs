//! Structured data as the web writes it: every case is a page (or a piece of one) the way some
//! site gets it wrong, and the JSON-LD it must give. Then inputs meant to break the reader:
//! none may panic, hang or grow without bound.

use std::time::{Duration, Instant};

use pestle_html::structured::{Structured, StructuredOptions};
use pestle_html::{Extractor, ExtractorOptions};
use serde_json::{json, Value};

fn extractor() -> Extractor {
    Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(StructuredOptions {
                microformats: false,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap()
}

fn read(html: &[u8]) -> Structured {
    extractor()
        .extract(html, Some("https://v.example/e/"))
        .structured
        .unwrap()
}

/// The items, without `@format` and a schema.org-only `@context` (checked apart).
fn items(html: &str) -> Vec<Value> {
    read(html.as_bytes())
        .items
        .into_iter()
        .map(|mut i| {
            let o = i.as_object_mut().unwrap();
            o.remove("@format");
            if o.get("@context") == Some(&json!("https://schema.org")) {
                o.remove("@context");
            }
            i
        })
        .collect()
}

fn ld(json: &str) -> String {
    format!(r#"<script type="application/ld+json">{json}</script>"#)
}

#[track_caller]
fn same(html: &str, want: Value) {
    let got = items(html);
    let want = match want {
        Value::Array(a) => a,
        one => vec![one],
    };
    assert_eq!(got, want, "\n{html}\n{}", serde_json::to_string_pretty(&got).unwrap());
}

// ── JSON-LD: broken JSON ─────────────────────────────────────────────────

#[test]
fn json_ld_syntax_mistakes() {
    let event = json!({ "@type": "Event", "name": "A" });
    for j in [
        r#"{"@type":"Event","name":"A",}"#,
        r#"{"@type":"Event",,"name":"A"}"#,
        "// generated\n{\"@type\":\"Event\", /* x */ \"name\":\"A\"}",
        r#"{'@type':'Event','name':'A'}"#,
        r#"{@type:"Event",name:"A"}"#,
        r#"{"@type":"Event" "name":"A"}"#,
        "\u{feff}{\"@type\":\"Event\",\"name\":\"A\"}",
        "<![CDATA[{\"@type\":\"Event\",\"name\":\"A\"}]]>",
        "//<![CDATA[\n{\"@type\":\"Event\",\"name\":\"A\"}\n//]]>",
        "<!-- {\"@type\":\"Event\",\"name\":\"A\"} -->",
        "{&quot;@type&quot;:&quot;Event&quot;,&quot;name&quot;:&quot;A&quot;}",
        r#"{"@type":"Event","name":"A"};"#,
        r#"  {"@type":"Event","name":"A"}  garbage after"#,
        r#"{"@type":"Event","name":"A""#,
        r#"{"@type":"Event","name":"A","location":{"@type":"Place""#,
        r#"[{"@type":"Event","name":"A"},]"#,
        r#"{"@type":"Event","name":"A","x":undefined}"#,
        r#"{"@type":"Event","name":"A","x":NaN,"y":Infinity}"#,
    ] {
        let got = items(&ld(j));
        assert!(!got.is_empty(), "{j}");
        assert_eq!(got[0]["@type"], event["@type"], "{j}\n{got:?}");
        assert_eq!(got[0]["name"], event["name"], "{j}\n{got:?}");
    }
}

#[test]
fn json_ld_strings() {
    same(
        &ld(r#"{"@type":"Event","name":"The "Best" Night"}"#),
        json!({ "@type": "Event", "name": "The \"Best\" Night" }),
    );
    same(
        &ld("{\"@type\":\"Event\",\"name\":\"A\nB\tC\"}"),
        json!({ "@type": "Event", "name": "A B C" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":"Café \/ \q"}"#),
        json!({ "@type": "Event", "name": "Café / q" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":"\ud800 lone"}"#),
        json!({ "@type": "Event", "name": "\u{fffd} lone" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":"<b>Nora</b> &amp; <i>Co</i>"}"#),
        json!({ "@type": "Event", "name": "Nora & Co" }),
    );
    same(
        &ld(r#"{"@type":"Event","description":"<p>One</p><p>Two<br>Three</p>"}"#),
        json!({ "@type": "Event", "description": "One Two Three" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":"3 < 5 & 7 > 2"}"#),
        json!({ "@type": "Event", "name": "3 < 5 & 7 > 2" }),
    );
    same(
        &ld(r#"{"@type":"Event","url":"https://v.example/?a=1&b=2&amp;c=3"}"#),
        json!({ "@type": "Event", "url": "https://v.example/?a=1&b=2&c=3" }),
    );
}

#[test]
fn json_ld_values_that_are_no_values() {
    same(
        &ld(
            r#"{"@type":"Event","name":"A","description":"","url":"  ","image":null,"offers":[],"location":{},"performer":[null,"",{}]}"#,
        ),
        json!({ "@type": "Event", "name": "A" }),
    );
    // Nothing at all: no item.
    for j in [
        "",
        "   ",
        "null",
        "42",
        "\"x\"",
        "[]",
        "{}",
        "{{{",
        "undefined",
        r#"{"@context":"https://schema.org"}"#,
        r##"{"@id":"#x"}"##,
    ] {
        assert_eq!(items(&ld(j)), Vec::<Value>::new(), "{j}");
    }
}

// ── JSON-LD: contexts, types and names ───────────────────────────────────

#[test]
fn json_ld_contexts() {
    let want = json!({ "@type": "Event", "name": "A" });
    for context in [
        r#""https://schema.org""#,
        r#""https://schema.org/""#,
        r#""http://schema.org""#,
        r#""http://schema.org/""#,
        r#""https://www.schema.org""#,
        r#""schema.org""#,
        r#""https://schema.org/docs/jsonldcontext.json""#,
        r#"{"@vocab":"http://schema.org/"}"#,
        r#"["https://schema.org",{"x":"https://x.example/"}]"#,
        "null",
    ] {
        same(
            &ld(&format!(r#"{{"@context":{context},"@type":"Event","name":"A"}}"#)),
            want.clone(),
        );
    }
    same(&ld(r#"{"@type":"Event","name":"A"}"#), want.clone());
    // Aliases and prefixes.
    same(
        &ld(r#"{"@context":"https://schema.org","type":"Event","name":"A"}"#),
        want.clone(),
    );
    same(
        &ld(r#"{"@context":{"s":"https://schema.org/"},"@type":"s:Event","s:name":"A"}"#),
        want.clone(),
    );
    same(&ld(r#"{"@type":"schema:Event","schema:name":"A"}"#), want.clone());
    same(
        &ld(r#"{"@type":"http://schema.org/Event","http://schema.org/name":"A"}"#),
        want.clone(),
    );
    // Another vocabulary stays itself.
    same(
        &ld(r#"{"@context":{"@vocab":"https://x.example/ns#"},"@type":"Gig","title":"A"}"#),
        json!({ "@type": "https://x.example/ns#Gig", "https://x.example/ns#title": "A" }),
    );
}

#[test]
fn json_ld_type_and_name_mistakes() {
    same(
        &ld(r#"{"@type":"musicevent","Name":"A"}"#),
        json!({ "@type": "MusicEvent", "name": "A" }),
    );
    same(
        &ld(r#"{"@type":"MUSICEVENT","NAME":"A"}"#),
        json!({ "@type": "MusicEvent", "name": "A" }),
    );
    same(
        &ld(r#"{"@type":"Music Event","start_date":"x","end-date":"y","door time":"z"}"#),
        json!({ "@type": "MusicEvent", "startDate": "x", "endDate": "y", "doorTime": "z" }),
    );
    same(
        &ld(r#"{"@type":"Event, MusicEvent","name":"A"}"#),
        json!({ "@type": ["Event", "MusicEvent"], "name": "A" }),
    );
    same(
        &ld(r#"{"@type":["Event",5,null,"",{}],"name":"A"}"#),
        json!({ "@type": "Event", "name": "A" }),
    );
    same(
        &ld(r#"{"@type":"https://schema.org/Event/","name":"A"}"#),
        json!({ "@type": "Event", "name": "A" }),
    );
    // Unknown names are kept as written: nothing is lost.
    same(
        &ld(r#"{"@type":"Event","name":"A","ticketLink":"t","x-custom":1}"#),
        json!({ "@type": "Event", "name": "A", "ticketLink": "t", "x-custom": 1 }),
    );
    same(
        &ld(r#"{"@type":"Evnt","nmae":"A"}"#),
        json!({ "@type": "Evnt", "nmae": "A" }),
    );
}

#[test]
fn json_ld_values_of_the_wrong_kind_are_kept() {
    same(
        &ld(
            r#"{"@type":"Event","name":["A"],"location":"Paradiso","performer":"Nora","offers":{"price":25},"startDate":["2026-11-14"],"isAccessibleForFree":"true","eventStatus":"EventScheduled"}"#,
        ),
        json!({ "@type": "Event", "name": "A", "location": "Paradiso", "performer": "Nora", "offers": { "price": 25 }, "startDate": "2026-11-14", "isAccessibleForFree": "true", "eventStatus": "EventScheduled" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":12345,"x":true}"#),
        json!({ "@type": "Event", "name": 12345, "x": true }),
    );
    same(
        &ld(r#"{"@type":"Event","name":{"@value":"A","@language":"nl"}}"#),
        json!({ "@type": "Event", "name": "A" }),
    );
    same(
        &ld(r#"{"@type":"Event","name":[["A"]]}"#),
        json!({ "@type": "Event", "name": "A" }),
    );
    // Enumerations however written.
    for status in [
        "EventCancelled",
        "https://schema.org/EventCancelled",
        "http://schema.org/EventCancelled",
        "eventcancelled",
        "schema:EventCancelled",
    ] {
        same(
            &ld(&format!(r#"{{"@type":"Event","eventStatus":"{status}"}}"#)),
            json!({ "@type": "Event", "eventStatus": "EventCancelled" }),
        );
    }
}

#[test]
fn json_ld_ids_graphs_and_references() {
    same(
        &ld(
            r##"{"@context":"https://schema.org","@graph":[{"@type":"Event","name":"A","location":{"@id":"#venue"}},{"@type":"Place","@id":"#venue","name":"P"}]}"##,
        ),
        json!([
            { "@type": "Event", "name": "A", "location": { "@type": "Place", "@id": "https://v.example/e/#venue", "name": "P" } },
            { "@type": "Place", "@id": "https://v.example/e/#venue", "name": "P" },
        ]),
    );
    // A reference to nothing stays a reference.
    same(
        &ld(r##"{"@type":"Event","location":{"@id":"#nowhere"}}"##),
        json!({ "@type": "Event", "location": "https://v.example/e/#nowhere" }),
    );
    // Nested graphs, top-level arrays, several blocks: all their items.
    let names: Vec<Value> = items(&(ld(r#"[{"@graph":[{"@type":"Event","name":"A"},{"@graph":[{"@type":"Event","name":"B"}]}]},{"@type":"Event","name":"C"}]"#) + &ld(r#"{"@type":"Event","name":"D"}"#)))
        .into_iter()
        .map(|i| i["name"].clone())
        .collect();
    assert_eq!(names, [json!("A"), json!("B"), json!("C"), json!("D")]);
    // The same @id in two blocks is one node, its properties together.
    let got = items(
        &(ld(r#"{"@type":"Event","@id":"https://v.example/1","name":"A"}"#)
            + &ld(r#"{"@id":"https://v.example/1","startDate":"x"}"#)),
    );
    assert_eq!(got[0]["name"], "A");
    assert_eq!(got[0]["startDate"], "x");
    // A cycle is cut at the node already open.
    let got = items(&ld(
        r##"{"@graph":[{"@type":"Event","@id":"#a","superEvent":{"@id":"#b"}},{"@type":"Event","@id":"#b","subEvent":{"@id":"#a"}}]}"##,
    ));
    assert_eq!(got[0]["superEvent"]["subEvent"], "https://v.example/e/#a");
}

#[test]
fn json_ld_script_types() {
    for t in [
        "application/ld+json",
        " Application/LD+JSON ",
        "application/ld+json; charset=utf-8",
        "text/ld+json",
        "application/json+ld",
        "application/jsonld",
    ] {
        same(
            &format!(r#"<script type="{t}">{{"@type":"Event","name":"A"}}</script>"#),
            json!({ "@type": "Event", "name": "A" }),
        );
    }
    for t in ["application/json", "text/javascript", "text/template"] {
        assert_eq!(
            items(&format!(
                r#"<script type="{t}">{{"@type":"Event","name":"A"}}</script>"#
            )),
            Vec::<Value>::new(),
            "{t}"
        );
    }
}

// ── Microdata ────────────────────────────────────────────────────────────

#[test]
fn microdata_mistakes() {
    let want = json!({ "@type": "Event", "name": "A" });
    for t in [
        "https://schema.org/Event",
        "http://schema.org/Event",
        "https://schema.org/Event/",
        "schema.org/Event",
        "//schema.org/Event",
        "http://www.schema.org/Event",
        "https://schema.org/event",
        "  https://schema.org/Event  ",
    ] {
        same(
            &format!(r#"<div itemscope itemtype="{t}"><span itemprop="name">A</span></div>"#),
            want.clone(),
        );
    }
    // `itemtype` without `itemscope`, names in other cases or as IRIs, two names at once.
    same(
        r#"<div itemtype="https://schema.org/Event"><span itemprop="Name">A</span></div>"#,
        want.clone(),
    );
    same(
        r#"<div itemscope itemtype="https://schema.org/Event"><span itemprop="http://schema.org/name">A</span></div>"#,
        want.clone(),
    );
    same(
        r#"<div itemscope itemtype="https://schema.org/Event"><span itemprop="name alternateName">A</span></div>"#,
        json!({ "@type": "Event", "name": "A", "alternateName": "A" }),
    );
    same(
        r#"<div itemscope itemtype="https://schema.org/Event https://schema.org/MusicEvent"><b itemprop="name">A</b></div>"#,
        json!({ "@type": ["Event", "MusicEvent"], "name": "A" }),
    );
    // data-vocabulary.org, Google's vocabulary before schema.org.
    same(
        r#"<div itemscope itemtype="http://data-vocabulary.org/Event"><span itemprop="summary">A</span><img itemprop="photo" src="/a.jpg"></div>"#,
        json!({ "@type": "Event", "name": "A", "image": "https://v.example/a.jpg" }),
    );
}

#[test]
fn microdata_values() {
    same(
        r#"<div itemscope itemtype="https://schema.org/Event">
            <a itemprop="url">no href</a><img itemprop="image"><span itemprop="description" content=""></span>
            <span itemprop="name">  A
              B  </span><time itemprop="startDate">za 14 nov</time><time itemprop="endDate" datetime="2026-11-14T23:00">late</time>
            <data itemprop="duration" value="PT3H">3 hours</data><meter itemprop="x" value="3">three</meter>
            <span itemprop="price" content="25">€ 25,-</span><meta itemprop="image" content="/a.jpg"><link itemprop="sameAs" href="b">
          </div>"#,
        json!({ "@type": "Event", "name": "A B", "startDate": "za 14 nov", "endDate": "2026-11-14T23:00", "duration": "PT3H", "x": "3", "price": "25",
                "image": "https://v.example/a.jpg", "sameAs": "https://v.example/e/b" }),
    );
}

#[test]
fn microdata_structure_mistakes() {
    // A property outside any item, an item inside a property-less item, an empty type.
    same(
        r#"<span itemprop="name">orphan</span><div itemscope itemtype="https://schema.org/Event"><span itemprop="name">A</span></div>"#,
        json!({ "@type": "Event", "name": "A" }),
    );
    same(
        r#"<div itemscope itemtype="https://schema.org/Event"><span itemprop="name">A</span><div itemscope itemtype="https://schema.org/Place"><span itemprop="name">P</span></div></div>"#,
        json!([{ "@type": "Event", "name": "A" }, { "@type": "Place", "name": "P" }]),
    );
    same(
        r#"<div itemscope itemtype=""><span itemprop="name">A</span></div>"#,
        json!({ "name": "A" }),
    );
    // itemref: to an element after, to nothing, to itself, in a loop.
    same(
        r#"<div itemscope itemtype="https://schema.org/Event" itemref="x"></div><p id="x"><b itemprop="name">A</b></p>"#,
        json!({ "@type": "Event", "name": "A" }),
    );
    same(
        r#"<div itemscope itemtype="https://schema.org/Event" itemref="nope"><b itemprop="name">A</b></div>"#,
        json!({ "@type": "Event", "name": "A" }),
    );
    let _ = items(
        r#"<div id="a" itemscope itemtype="https://schema.org/Event" itemref="a b"><div id="b" itemprop="sub" itemscope itemref="a"><b itemprop="name">A</b></div></div>"#,
    );
    // Unclosed tags close where HTML closes them.
    same(
        r#"<div itemscope itemtype="https://schema.org/Event"><p itemprop="name">A<p itemprop="description">B</div>"#,
        json!({ "@type": "Event", "name": "A", "description": "B" }),
    );
}

// ── RDFa ─────────────────────────────────────────────────────────────────

#[test]
fn rdfa_mistakes() {
    let want = json!({ "@type": "Event", "name": "A" });
    for open in [
        r#"<div vocab="https://schema.org/" typeof="Event">"#,
        r#"<div vocab="http://schema.org/" typeof="event">"#,
        r#"<div vocab=" https://schema.org/ " typeof="Event">"#,
        r#"<div typeof="Event">"#,
        r#"<div typeof="schema:Event">"#,
        r#"<div prefix="s: https://schema.org/" typeof="s:Event">"#,
        r#"<div typeof="https://schema.org/Event">"#,
    ] {
        same(&format!(r#"{open}<span property="name">A</span></div>"#), want.clone());
    }
    // An unknown prefix names nothing; a `rel` link type is no data.
    same(
        r#"<div typeof="Event"><span property="foo:bar">x</span><span property="name">A</span><a rel="nofollow" href="/x">x</a></div>"#,
        want.clone(),
    );
}

#[test]
fn rdfa_values() {
    same(
        r#"<div typeof="Event"><span property="name" content="A">shown</span><a property="url" href="/e/1">x</a><img property="image" src="i.jpg">
           <meta property="startDate" content="2026-11-14"><span property="description" content=""></span></div>"#,
        json!({ "@type": "Event", "name": "A", "url": "https://v.example/e/1", "image": "https://v.example/e/i.jpg", "startDate": "2026-11-14" }),
    );
    // OpenGraph: declared or not, http or https, relative image.
    for head in [
        "",
        r#"<html prefix="og: http://ogp.me/ns#">"#,
        r#"<html prefix="og: https://ogp.me/ns#">"#,
    ] {
        same(
            &format!(
                r#"{head}<meta property="og:title" content="T"><meta property="og:image" content="/i.jpg"><link rel="stylesheet" href="/s.css"><link rel="icon" href="/f.ico">"#
            ),
            json!({ "@context": ["https://schema.org", { "og": "http://ogp.me/ns#" }], "@id": "https://v.example/e/", "og:title": "T", "og:image": "https://v.example/i.jpg" }),
        );
    }
}

// ── All formats ──────────────────────────────────────────────────────────

#[test]
fn context_lists_the_prefixes_an_item_uses() {
    let got = read(br#"<meta property="og:title" content="T"><meta property="dc:creator" content="C">"#).items;
    assert_eq!(
        got[0]["@context"],
        json!(["https://schema.org", { "og": "http://ogp.me/ns#", "dcterms": "http://purl.org/dc/terms/" }])
    );
    let got = read(ld(r#"{"@type":"Event","name":"A"}"#).as_bytes()).items;
    assert_eq!(got[0]["@context"], "https://schema.org");
    assert_eq!(got[0]["@format"], "json-ld");
}

#[test]
fn pages_that_are_not_html() {
    for page in [
        &b""[..],
        b"just text, no markup at all",
        b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n1 0 obj << /Type /Catalog >> endobj",
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR",
        b"\x00\x00\x00\x00\x00",
        b"\xff\xfe<\x00h\x00t\x00m\x00l\x00>\x00",
        b"{\"@type\":\"Event\",\"name\":\"a JSON file, not a page\"}",
        b"<<<<<<>>>>>>>><</////>",
    ] {
        let s = read(page);
        assert!(
            s.items.is_empty() || page.starts_with(b"{"),
            "{:?}",
            String::from_utf8_lossy(page)
        );
    }
}

// ── Inputs meant to break it ─────────────────────────────────────────────

#[track_caller]
fn bounded(html: &[u8]) -> Structured {
    let t = Instant::now();
    let s = read(html);
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "took {:?} on {} bytes",
        t.elapsed(),
        html.len()
    );
    s
}

#[test]
fn deep_nesting() {
    let n = 50_000;
    bounded((ld(&("[".repeat(n) + &"]".repeat(n)))).as_bytes());
    bounded((ld(&r#"{"@type":"Event","x":"#.repeat(n))).as_bytes());
    bounded(
        format!(
            "{}{}",
            r#"<div itemscope itemtype="https://schema.org/Event" itemprop="sub">"#.repeat(n),
            "</div>".repeat(n)
        )
        .as_bytes(),
    );
    bounded(
        format!(
            "{}{}",
            r#"<div typeof="Event" property="sub">"#.repeat(n),
            "</div>".repeat(n)
        )
        .as_bytes(),
    );
}

#[test]
fn many_and_large() {
    // 20,000 items; one value of 5 MB; 300 JSON-LD blocks (more than read).
    let many = r#"<div itemscope itemtype="https://schema.org/Event"><b itemprop="name">A</b></div>"#.repeat(20_000);
    assert_eq!(bounded(many.as_bytes()).items.len(), 20_000);
    let big = format!(
        r#"<div itemscope itemtype="https://schema.org/Event"><b itemprop="name">{}</b></div>"#,
        "x".repeat(5 << 20)
    );
    let s = bounded(big.as_bytes());
    assert!(s.items[0]["name"].as_str().unwrap().len() <= 64 * 1024);
    let blocks = ld(r#"{"@type":"Event","name":"A"}"#).repeat(300);
    assert_eq!(bounded(blocks.as_bytes()).items.len(), 256);
    // A JSON-LD block of 10 MB (more than read) and one huge string in it.
    bounded(ld(&format!(r#"{{"@type":"Event","name":"{}"}}"#, "y".repeat(10 << 20))).as_bytes());
}

#[test]
fn reference_bombs() {
    // Each node names the next twice: written out, it would double at every level.
    let mut graph = Vec::new();
    for i in 0..60 {
        graph.push(format!(
            r##"{{"@id":"#n{i}","@type":"Thing","a":{{"@id":"#n{}"}},"b":{{"@id":"#n{}"}}}}"##,
            i + 1,
            i + 1
        ));
    }
    let s = bounded(ld(&format!(r#"{{"@graph":[{}]}}"#, graph.join(","))).as_bytes());
    let size = serde_json::to_string(&s.items).unwrap().len();
    assert!(size < 64 << 20, "{size} bytes");
    // itemref fan-out.
    let refs = (0..2000).map(|i| format!("p{i}")).collect::<Vec<_>>().join(" ");
    let props: String = (0..2000)
        .map(|i| format!(r#"<b id="p{i}" itemprop="n">x</b>"#))
        .collect();
    bounded(
        format!(
            r#"<div itemscope itemtype="https://schema.org/Event" itemref="{refs}"></div>{}"#,
            props
        )
        .as_bytes(),
    );
}

/// With microformats (the other cases leave them out).
#[track_caller]
fn bounded_mf(html: &[u8]) -> Structured {
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(StructuredOptions::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let t = Instant::now();
    let s = x.extract(html, Some("https://v.example/e/")).structured.unwrap();
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "took {:?} on {} bytes",
        t.elapsed(),
        html.len()
    );
    s
}

#[test]
fn include_pattern_loops_and_bombs() {
    // Elements that include themselves, each other, their own ancestor.
    let loops = r##"<div class="vcard" id="a"><span class="fn">A</span><a class="include" href="#a"></a><a class="include" href="#b"></a></div>
        <div id="b" class="vcard" itemref="a b"><span class="fn">B</span><object class="include" data="#b"></object></div>
        <table><tr><td id="c" class="vcard" headers="c d"><span class="fn">C</span></td><td id="d" headers="c">x</td></tr></table>"##;
    let s = bounded_mf(loops.as_bytes());
    assert!(!s.items.is_empty());
    // 2,000 items, each including one element of 20,000 nodes: what they grow to is bounded.
    let target = format!(
        r#"<div id="t"><span class="org">O</span>{}</div>"#,
        "<i>x</i>".repeat(10_000)
    );
    let page = format!(
        "{}{target}",
        r##"<div class="vcard"><a class="include" href="#t"></a><span class="fn">N</span></div>"##.repeat(2000)
    );
    let s = bounded_mf(page.as_bytes());
    assert!(s.items.len() >= 2000);
    // Includes that fan out: each element includes the next one twice.
    let chain: String = (0..40)
        .map(|i| format!(r##"<div id="n{i}" class="adr"><span class="locality">L</span><a class="include" href="#n{0}"></a><a class="include" href="#n{0}"></a></div>"##, i + 1))
        .collect();
    bounded_mf(format!(r##"<div class="vcard"><a class="include" href="#n0"></a></div>{chain}"##).as_bytes());
}

#[test]
fn anchors_inside_anchors() {
    // HTML closes an open `a` at the next `a` (microformats read HTML's tree).
    let n = 20_000;
    bounded_mf(
        format!(
            r#"<div class="h-card">{}</div>"#,
            r#"<a class="h-card" href="/x"><div><span>"#.repeat(n)
        )
        .as_bytes(),
    );
    bounded_mf(
        format!(
            r#"<div class="h-entry">{}</div>"#,
            "<a href=/y><p><a href=/z>".repeat(n)
        )
        .as_bytes(),
    );
    let s = bounded_mf(br#"<a class="h-card"><div><a href="https://v.example/nora">Nora</a></div></a>"#);
    let cards: Vec<&Value> = s.items.iter().filter(|i| i["@format"] == "microformats").collect();
    assert_eq!(cards.len(), 2, "{cards:#?}");
}

#[test]
fn random_mutations_of_real_markup() {
    let seed_page = format!(
        "{}{}{}",
        ld(
            r##"{"@context":"https://schema.org","@graph":[{"@type":"MusicEvent","@id":"#e","name":"Nora & Co","startDate":"2026-11-14T20:00","location":{"@id":"#p"},"offers":[{"@type":"Offer","price":"25"}]},{"@type":"Place","@id":"#p","name":"Paradiso"}]}"##
        ),
        r#"<div itemscope itemtype="https://schema.org/Event" itemref="r"><a itemprop="url" href="/e"><b itemprop="name">A</b></a><div itemprop="location" itemscope itemtype="https://schema.org/Place"><b itemprop="name">P</b></div></div><p id="r"><meta itemprop="startDate" content="2026-11-14"></p>"#,
        r#"<html prefix="og: http://ogp.me/ns#"><meta property="og:title" content="T"><div vocab="https://schema.org/" typeof="Event"><span property="name">Lu</span><div property="location" typeof="Place"><span property="name">Tivoli</span></div></div>"#,
    );
    let bytes = seed_page.as_bytes();
    let mut seed: u64 = 0x5eed;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let junk: [&[u8]; 12] = [
        b"\"",
        b"'",
        b"<",
        b">",
        b"{",
        b"}",
        b"[",
        b",",
        b"\\",
        b"&",
        b"\xff",
        b"<div itemscope>",
    ];
    let x = extractor();
    for _ in 0..3000 {
        let mut page = bytes.to_vec();
        for _ in 0..1 + rnd(8) {
            match rnd(4) {
                0 if !page.is_empty() => {
                    let at = rnd(page.len());
                    page.remove(at);
                }
                1 => {
                    let at = rnd(page.len() + 1);
                    let j = junk[rnd(junk.len())];
                    page.splice(at..at, j.iter().copied());
                }
                2 if !page.is_empty() => {
                    let at = rnd(page.len());
                    page.truncate(at);
                }
                _ if page.len() > 2 => {
                    let (a, b) = (rnd(page.len()), rnd(page.len()));
                    page.swap(a, b);
                }
                _ => {}
            }
        }
        let s = x.extract(&page, Some("https://v.example/")).structured.unwrap();
        // Whatever it read is JSON-LD: every item has a context and is an object.
        for i in &s.items {
            assert!(i.get("@context").is_some(), "{i}");
        }
    }
}
