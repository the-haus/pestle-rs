use pestle_html::structured::StructuredOptions;
use pestle_html::{Extractor, ExtractorOptions};
use serde_json::{json, Value};

const PAGE: &str = r##"<!doctype html>
<html lang="nl" prefix="og: http://ogp.me/ns#"><head>
<base href="https://venue.example/nl/">
<title>Nora</title>
<meta property="og:title" content="Nora &amp; Co live">
<meta property="og:type" content="music.event">
<meta name="description" content="Een avond">
<script type="application/ld+json">
// a comment the template left
{
  "@context": "https://schema.org",
  "@graph": [
    { "@type": "MusicEvent", "@id": "#nora", "name": "Nora & Co", "startDate": "2026-11-14T20:00+01:00",
      "eventStatus": "https://schema.org/EventScheduled",
      "location": { "@id": "#paradiso" },
      "performer": [{ "@type": "MusicGroup", "name": "Nora", "sameAs": "https://www.wikidata.org/wiki/Q1" }],
      "offers": { "@type": "Offer", "price": "25.00", "priceCurrency": "EUR", "url": "tickets/nora", "availability": "http://schema.org/InStock" }, },
    { "@type": "Place", "@id": "#paradiso", "name": "Paradiso",
      "address": { "@type": "PostalAddress", "streetAddress": "Weteringschans 6", "addressLocality": "Amsterdam", "postalCode": "1017 SG", "addressCountry": "NL" },
      "geo": { "@type": "GeoCoordinates", "latitude": 52.36, "longitude": 4.88 } },
  ]
}
</script>
</head><body>
<div itemscope itemtype="https://schema.org/Event" itemref="shared">
  <h2 itemprop="name">Mira</h2>
  <time itemprop="startDate" datetime="2026-11-15T21:30">zo 15 nov</time>
  <div itemprop="location" itemscope itemtype="https://schema.org/Place"><span itemprop="name">Melkweg</span></div>
  <span itemprop="offers" itemscope itemtype="https://schema.org/Offer"><span itemprop="price" content="18">€ 18,-</span><meta itemprop="priceCurrency" content="EUR"></span>
</div>
<p id="shared"><a itemprop="url" href="/e/mira">Info</a></p>
<div vocab="https://schema.org/" typeof="Event">
  <span property="name">Lu</span> <span property="startDate" content="2026-11-16">16 nov</span>
  <div property="location" typeof="Place"><span property="name">Tivoli</span></div>
</div>
<div class="h-event">
  <a class="p-name u-url" href="/e/zea">Zea</a>
  <time class="dt-start" datetime="2026-11-17 20:00">17 nov</time> – <span class="dt-end">23:00</span>
  <p class="p-location h-card"><span class="p-name">Doornroosje</span>, <span class="p-locality">Nijmegen</span></p>
</div>
<div class="vevent"><span class="summary">Old Format Night</span> <abbr class="dtstart" title="2026-11-18T19:00">18 nov</abbr> <span class="location">De Pul</span></div>
</body></html>"##;

fn run(options: StructuredOptions) -> pestle_html::structured::Structured {
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(options),
            ..Default::default()
        },
    )
    .unwrap();
    let whole = x
        .extract(PAGE.as_bytes(), Some("https://venue.example/agenda"))
        .structured
        .unwrap();
    // The same in small chunks.
    for size in [1usize, 7, 64] {
        let mut s = x.session(Some("https://venue.example/agenda"));
        for c in PAGE.as_bytes().chunks(size) {
            s.write(c);
        }
        let chunked = s.finish().structured.unwrap();
        assert_eq!(
            serde_json::to_value(&chunked).unwrap(),
            serde_json::to_value(&whole).unwrap(),
            "chunks of {size}"
        );
    }
    whole
}

fn names(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["name"].as_str()).collect()
}

#[test]
fn every_format_gives_its_events() {
    let s = run(StructuredOptions::default());
    assert_eq!(
        names(&s.events),
        ["Nora & Co", "Mira", "Lu", "Zea", "Old Format Night"],
        "{:#?}",
        s.events
    );
    let nora = &s.events[0];
    assert_eq!(nora["startDate"], "2026-11-14T20:00+01:00");
    assert_eq!(nora["status"], "EventScheduled");
    assert_eq!(nora["location"][0]["name"], "Paradiso");
    assert_eq!(nora["location"][0]["locality"], "Amsterdam");
    assert_eq!(nora["location"][0]["lat"], "52.36");
    assert_eq!(nora["performers"][0]["name"], "Nora");
    assert_eq!(nora["offers"][0]["price"], "25.00");
    assert_eq!(nora["offers"][0]["currency"], "EUR");
    assert_eq!(nora["offers"][0]["url"], "https://venue.example/nl/tickets/nora");
    assert_eq!(nora["offers"][0]["availability"], "InStock");
    assert_eq!(nora["format"], "json-ld");
    let mira = &s.events[1];
    assert_eq!(mira["startDate"], "2026-11-15T21:30");
    assert_eq!(mira["url"], "https://venue.example/e/mira", "itemref brings the url in");
    assert_eq!(mira["location"][0]["name"], "Melkweg");
    assert_eq!(mira["offers"][0]["price"], "18");
    assert_eq!(s.events[2]["location"][0]["name"], "Tivoli");
    assert_eq!(s.events[2]["format"], "rdfa");
    let zea = &s.events[3];
    assert_eq!(zea["startDate"], "2026-11-17 20:00");
    assert_eq!(zea["endDate"], "2026-11-17 23:00", "the end takes the start's date");
    assert_eq!(zea["url"], "https://venue.example/e/zea");
    assert_eq!(zea["location"][0]["name"], "Doornroosje");
    assert_eq!(zea["location"][0]["locality"], "Nijmegen");
    assert_eq!(s.events[4]["startDate"], "2026-11-18T19:00");
    assert_eq!(s.events[4]["location"][0]["name"], "De Pul");
    assert!(s.warnings.iter().any(|w| w.contains("comments")), "{:?}", s.warnings);
}

#[test]
fn items_and_open_graph() {
    let s = run(StructuredOptions::default());
    let page = s
        .items
        .iter()
        .find(|i| i["og:title"].is_string())
        .expect("the page's OpenGraph");
    assert_eq!(page["og:title"], "Nora & Co live");
    assert_eq!(page["og:type"], "music.event");
    assert_eq!(page["@id"], "https://venue.example/nl/");
}

#[test]
fn filter_by_type_and_raw_forms() {
    let mut o = StructuredOptions {
        raw: true,
        ..Default::default()
    };
    o.render.types = vec!["Place".into()];
    let s = run(o);
    let places: Vec<&str> = s.items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(places, ["Paradiso", "Melkweg", "Tivoli"]);
    let raw = s.raw.unwrap();
    assert_eq!(raw.microdata["items"][0]["type"], json!(["https://schema.org/Event"]));
    assert_eq!(raw.microformats["items"][0]["type"], json!(["h-event"]));
    assert!(raw
        .rdfa
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["predicate"] == "http://ogp.me/ns#title"));
    assert_eq!(raw.json_ld.len(), 1);
}

#[test]
fn hostile_input_never_breaks_it() {
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(Default::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let junk = [
        "<script type=application/ld+json>{{{{[[[[\"\\u",
        "<div itemscope itemref=\"a a a\" id=a itemprop=x><div itemscope itemprop=y itemref=a id=b></div></div>",
        "<div vocab=\"\" typeof=\"\" property=\"::: _:x [x] [_:]\" rel=\"_: [ ]\" inlist about=\"[\">x</div>",
        "<div class=\"h-a h-b h- p-x u-x dt-x e-x\"><div class=\"value\"><img class=u-photo alt></div></div>",
    ];
    for j in junk {
        let deep = j.repeat(200);
        let _ = x.extract(deep.as_bytes(), Some("https://x.example/"));
        let _ = x.extract(deep.as_bytes(), None);
    }
}

#[test]
fn one_event_stated_twice_is_one() {
    let page = r#"<script type="application/ld+json">[
      {"@context":"https://schema.org","@type":"MusicEvent","name":"Nora","url":"https://v.example/e/nora","startDate":"2026-11-14T20:00:00+01:00",
       "location":{"@type":"Place","name":"Paradiso"}},
      {"@context":"https://schema.org","@type":"Event","name":"Nora","url":"https://v.example/e/nora"}]</script>
      <div itemscope itemtype="https://schema.org/Event"><a itemprop="url" href="https://v.example/e/nora"><span itemprop="name">Nora</span></a>
      <time itemprop="startDate" datetime="2026-11-14">za 14 nov</time> <span itemprop="doorTime">19:00</span>
      <div itemprop="location" itemscope itemtype="https://schema.org/Place"><span itemprop="name">Paradiso</span>
        <span itemprop="address">Weteringschans 6</span></div>
      <span itemprop="offers" itemscope itemtype="https://schema.org/Offer"><span itemprop="price">€ 25,-</span></span></div>
      <div itemscope itemtype="https://schema.org/Event"><span itemprop="name">Nora</span><meta itemprop="startDate" content="2026-11-15"></div>"#;
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(Default::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let s = x
        .extract(page.as_bytes(), Some("https://v.example/"))
        .structured
        .unwrap();
    assert_eq!(s.events.len(), 2, "{:#?}", s.events);
    let e = &s.events[0];
    assert_eq!(
        e["startDate"], "2026-11-14T20:00:00+01:00",
        "the more precise of the two"
    );
    assert_eq!(e["doorTime"], "19:00", "as written");
    assert_eq!(e["formats"], json!(["json-ld", "microdata"]));
    assert_eq!(e["location"].as_array().unwrap().len(), 1);
    assert_eq!(e["location"][0]["address"], "Weteringschans 6");
    assert_eq!(e["offers"][0]["price"], "€ 25,-", "as written");
    assert_eq!(s.events[1]["startDate"], "2026-11-15", "another day is another event");

    let apart = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(StructuredOptions {
                merge_events: false,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(apart.extract(page.as_bytes(), None).structured.unwrap().events.len(), 4);
}

/// Microdata, RDFa and JSON-LD, written right or the ways sites get them wrong, all give the same
/// JSON-LD item.
#[test]
fn every_format_and_every_mistake_gives_the_same_json_ld() {
    let x = Extractor::new(
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
    .unwrap();
    let want = json!({
        "@context": "https://schema.org", "@type": "MusicEvent", "name": "Nora & Co",
        "startDate": "2026-11-14T20:00", "image": "https://v.example/nora.jpg",
    });
    let ld = |j: &str| format!(r#"<script type="application/ld+json">{j}</script>"#);
    let pages = [
        // Right.
        ld(r#"{"@context":"https://schema.org","@type":"MusicEvent","name":"Nora & Co","startDate":"2026-11-14T20:00","image":"https://v.example/nora.jpg"}"#),
        r#"<div itemscope itemtype="https://schema.org/MusicEvent"><h1 itemprop="name">Nora &amp; Co</h1><time itemprop="startDate" datetime="2026-11-14T20:00">za</time><img itemprop="image" src="/nora.jpg"></div>"#.into(),
        r#"<div vocab="https://schema.org/" typeof="MusicEvent"><h1 property="name">Nora &amp; Co</h1><time property="startDate" datetime="2026-11-14T20:00">za</time><img property="image" src="/nora.jpg"></div>"#.into(),
        // JSON-LD as sites write it.
        ld(r#"{"@context":"http://schema.org/","@type":"musicevent","Name":"Nora &amp; Co","start_date":"2026-11-14T20:00","image":"/nora.jpg","url":""}"#),
        ld(r#"{type:'MusicEvent', name:'<b>Nora</b> &amp; Co', startDate:"2026-11-14T20:00", image:"/nora.jpg",}"#),
        ld(r#"[{"@context":{"@vocab":"http://schema.org/"},"@graph":[{"@type":"http://schema.org/MusicEvent","schema:name":"Nora & Co","startDate":"2026-11-14T20:00","image":"https:\/\/v.example\/nora.jpg"}]}]"#),
        ld(r#"<!--{"@type":"MusicEvent","name":"Nora & Co","startDate":"2026-11-14T20:00","image":"/nora.jpg"}-->"#),
        r#"<script type="text/ld+json">{"@type":"MusicEvent","name":"Nora & Co","startDate":"2026-11-14T20:00","image":"/nora.jpg"}</script>"#.into(),
        // Microdata as sites write it.
        r#"<div itemscope itemtype="schema.org/MusicEvent/"><h1 itemprop="Name">Nora &amp; Co</h1><meta itemprop="startdate" content="2026-11-14T20:00"><meta itemprop="image" content="/nora.jpg"><span itemprop="url" content=""></span></div>"#.into(),
        r#"<div itemtype="http://schema.org/MusicEvent"><h1 itemprop="http://schema.org/name">Nora &amp; Co</h1><span itemprop="startDate" content="2026-11-14T20:00">za 14 nov</span><link itemprop="image" href="nora.jpg"></div>"#.into(),
        // RDFa as sites write it.
        r#"<div typeof="MusicEvent"><h1 property="name">Nora &amp; Co</h1><meta property="startDate" content="2026-11-14T20:00"><meta property="image" content="/nora.jpg"></div>"#.into(),
        r#"<div vocab="http://schema.org/" typeof="musicevent"><h1 property="Name">Nora &amp; Co</h1><span property="schema:startDate" content="2026-11-14T20:00"></span><img property="image" src="nora.jpg"></div>"#.into(),
    ];
    for page in pages {
        let s = x
            .extract(page.as_bytes(), Some("https://v.example/"))
            .structured
            .unwrap();
        let mut items: Vec<Value> = s
            .items
            .into_iter()
            .map(|mut i| {
                let o = i.as_object_mut().unwrap();
                o.remove("@format");
                o.remove("@id");
                i
            })
            .collect();
        assert_eq!(items.len(), 1, "{page}\n{items:#?}");
        assert_eq!(items.remove(0), want, "{page}");
    }
}

#[test]
fn strict_json_ld_is_json_ld_1_1() {
    let mut opts = StructuredOptions::default();
    opts.json_ld_options.strict = true;
    let x = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(opts),
            ..Default::default()
        },
    )
    .unwrap();
    let read = |html: &str| {
        x.extract(html.as_bytes(), Some("https://venue.example/agenda"))
            .structured
            .unwrap()
    };
    let ld = |j: &str| format!(r#"<script type="application/ld+json">{j}</script>"#);

    // The page above: strict reads its block as the lenient reader does (once its trailing commas
    // and comment are gone).
    let block = r##"{"@context": "https://schema.org", "@graph": [
        {"@type": "MusicEvent", "@id": "#nora", "name": "Nora & Co", "location": {"@id": "#paradiso"},
         "performer": [{"@type": "MusicGroup", "name": "Nora"}], "offers": {"@type": "Offer", "url": "tickets/nora", "price": 25}},
        {"@type": "Place", "@id": "#paradiso", "name": "Paradiso"}]}"##;
    let strict = read(&format!(r#"<base href="https://venue.example/nl/">{}"#, ld(block)));
    assert!(strict.warnings.is_empty(), "{:?}", strict.warnings);
    let event = &strict.items[0];
    assert_eq!(event["@type"], "MusicEvent");
    assert_eq!(event["@id"], "https://venue.example/nl/#nora");
    assert_eq!(event["location"]["name"], "Paradiso");
    assert_eq!(event["performer"]["name"], "Nora");
    // `url` is `@id` in schema.org's context: resolved against the base.
    assert_eq!(event["offers"]["url"], "https://venue.example/nl/tickets/nora");
    assert_eq!(event["offers"]["price"], 25);

    // What the spec refuses is left out, with the spec's error code.
    for (block, code) in [
        (
            r#"{"@context": "https://schema.org", "@id": 5, "name": "x"}"#,
            "invalid @id value",
        ),
        (
            r#"{"@context": "https://example.com/unknown.jsonld", "name": "x"}"#,
            "loading remote context failed",
        ),
        (r#"{"@context": {"@vocab": 5}, "name": "x"}"#, "invalid vocab mapping"),
        (r#"{"@type": "Event", "name": "x",}"#, "it is not JSON"),
    ] {
        let s = read(&ld(block));
        assert!(s.items.is_empty(), "{block}: {:?}", s.items);
        assert_eq!(
            s.warnings,
            [format!("json-ld: a block was left out: {code}")],
            "{block}"
        );
    }
    // No context: no vocabulary, so `name` means nothing and is dropped, and the type is an IRI
    // relative to the page.
    let s = read(&ld(r#"{"@type": "Event", "name": "x"}"#));
    assert_eq!(s.items.len(), 1);
    assert_eq!(s.items[0]["@type"], "https://venue.example/Event");
    assert!(s.items[0].get("name").is_none());
    // The lenient reader reads all of these as meant.
    let lenient = Extractor::new(
        &json!({}),
        &Default::default(),
        ExtractorOptions {
            structured: Some(StructuredOptions::default()),
            ..Default::default()
        },
    )
    .unwrap();
    let s = lenient
        .extract(ld(r#"{"@type": "Event", "name": "x",}"#).as_bytes(), None)
        .structured
        .unwrap();
    assert_eq!(s.items[0]["name"], "x");
}
