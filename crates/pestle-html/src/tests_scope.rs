//! A field's selector under its scope (matched in the page), `closest`, and text read as lines.

use serde_json::json;

use super::{data, Grammars};
use crate::{Extractor, ExtractorOptions};

#[test]
fn a_fields_selector_is_matched_in_the_page() {
    let html = r#"<section class="card"><h3>Friday</h3></section>"#;
    let spec = json!({ "items": [{ "select": ".card", "fields": { "t": ".card h3" } }] });
    assert_eq!(data(spec, html), json!({ "items": [{ "t": "Friday" }] }));

    // Naming an ancestor above the scope; `:scope` keeps to the scope.
    let html = r#"<section id="s"><ul><li><b>A</b></li><li><i><b>B</b></i></li></ul></section><div><ul><li><b>C</b></li></ul></div>"#;
    let spec = json!({
        "rows": [{ "select": "li", "fields": {
            "inSection": "section b",
            "direct": "> b",
            "scoped": ":scope b",
            "notUnder": ":scope section b",
            "self": "li",
        } }],
    });
    assert_eq!(
        data(spec, html),
        json!({ "rows": [
            { "inSection": "A", "direct": "A", "scoped": "A", "notUnder": null, "self": null },
            { "inSection": "B", "direct": null, "scoped": "B", "notUnder": null, "self": null },
            { "inSection": null, "direct": "C", "scoped": "C", "notUnder": null, "self": null },
        ] })
    );

    // `:has()` stays relative to the element it tests.
    let html = r#"<ul id="a"><li>x</li></ul><main><ul id="b"><li>y</li></ul></main>"#;
    let spec = json!({
        "m": { "select": "body, main", "all": true, "fields": { "lists": ["ul:has(li)@id"], "inMain": ["ul:has(main li)@id"] } },
        "top": ["ul:has(li)@id"],
    });
    let html = format!("<body>{html}</body>");
    assert_eq!(
        data(spec, &html),
        json!({ "m": [{ "lists": ["a", "b"], "inMain": [] }, { "lists": ["b"], "inMain": [] }], "top": ["a", "b"] })
    );
}

#[test]
fn closest_reads_the_nearest_ancestor_with_a_value() {
    // The repro: the day on the section around the list.
    let html = r#"<section data-day="fri"><ul><li class="set"><b>A</b></li></ul></section>"#;
    let spec = json!({ "rows": [{ "select": "li.set", "fields": { "act": "b", "day": { "select": "section@data-day", "closest": true } } }] });
    assert_eq!(data(spec, html), json!({ "rows": [{ "act": "A", "day": "fri" }] }));

    // A value in an ancestor's other child (an `<h3>` beside the list), before or after it; a
    // nearer ancestor wins; the row's own wins over all. An ancestor's first match is its
    // `querySelector`'s, a later row's too: A's list holds B's `<h3>`.
    let html = r#"
      <div class="festival">
        <h3>Weekend</h3>
        <section><h3>Friday</h3><ul><li>A</li><li><h3>Own</h3>B</li></ul><ol><li>A2</li></ol></section>
        <section><ul><li>C</li></ul><h3>Saturday</h3></section>
        <section><ul><li>D</li></ul></section>
      </div>
      <ul><li>E</li></ul>"#;
    let spec = json!({ "rows": [{ "select": "li", "fields": { "act": { "text": "own" }, "day": { "select": "h3", "closest": true } } }] });
    assert_eq!(
        data(spec, html),
        json!({ "rows": [
            { "act": "A", "day": "Own" },
            { "act": "B", "day": "Own" },
            { "act": "A2", "day": "Friday" },
            { "act": "C", "day": "Saturday" },
            { "act": "D", "day": "Weekend" },
            { "act": "E", "day": null },
        ] })
    );

    // `depth` stops the walk: the section is 2 levels up, the div 3.
    let spec = |depth: u64| json!({ "rows": [{ "select": "li", "fields": { "day": { "select": "[data-day]", "attr": "data-day", "closest": { "depth": depth }, "default": "?" } } }] });
    let html = r#"<div data-day="sat"><section data-day="fri"><ul><li>A</li></ul></section><ol><li>B</li></ol></div>"#;
    assert_eq!(data(spec(1), html), json!({ "rows": [{ "day": "?" }, { "day": "?" }] }));
    assert_eq!(
        data(spec(2), html),
        json!({ "rows": [{ "day": "fri" }, { "day": "fri" }] })
    );
    assert_eq!(
        data(
            spec(2),
            r#"<div data-day="sat"><section><ul><li>A</li></ul></section><ol><li>B</li></ol></div>"#
        ),
        json!({ "rows": [{ "day": "?" }, { "day": "sat" }] })
    );
    // The first under an ancestor wins over the ancestor itself (as `querySelector`, then the element).
    assert_eq!(
        data(
            spec(3),
            r#"<div data-day="sat"><section><ul><li>A</li></ul></section><p data-day="sun"></p></div>"#
        ),
        json!({ "rows": [{ "day": "sun" }] })
    );
}

#[test]
fn closest_skips_matches_without_a_value_and_waits_in_order() {
    // A match without a value is passed over: the next one, or the next ancestor's, is read.
    let html = r#"<main data-stage="Main"><section><h4 class="stage">Tent</h4><h4 class="stage"></h4><ul><li>A</li></ul></section><section><h4 class="stage"></h4><ul><li>B</li></ul></section></main>"#;
    let spec = json!({
        "rows": [{ "select": "li", "fields": {
            "stage": { "select": ".stage", "closest": true },
            "main": { "select": "[data-stage]", "attr": "data-stage", "closest": true },
        } }],
    });
    assert_eq!(
        data(spec, html),
        json!({ "rows": [{ "stage": "Tent", "main": "Main" }, { "stage": "Tent", "main": "Main" }] })
    );

    // Rows in objects in objects: each lands in its place, whatever closes first.
    let html = r#"<div class="day" data-d="1"><div class="stage"><ul><li>A</li><li>B</li></ul></div><div class="stage"><ul><li>C</li></ul><h5>Late</h5></div></div><div class="day"><div class="stage"><ul><li>D</li></ul></div></div><h5>Last</h5>"#;
    let spec = json!({
        "days": [{ "select": ".day", "fields": {
            "stages": [{ "select": ".stage", "fields": {
                "acts": [{ "select": "li", "fields": {
                    "a": { "text": "own" },
                    "note": { "select": "h5", "closest": { "depth": 2 } },
                    "day": { "select": "[data-d]", "attr": "data-d", "closest": true },
                } }],
            } }],
        } }],
        "n": ["li"],
    });
    assert_eq!(
        data(spec, html),
        json!({
            "days": [
                { "stages": [
                    { "acts": [{ "a": "A", "note": null, "day": "1" }, { "a": "B", "note": null, "day": "1" }] },
                    { "acts": [{ "a": "C", "note": "Late", "day": "1" }] },
                ] },
                { "stages": [{ "acts": [{ "a": "D", "note": null, "day": null }] }] },
            ],
            "n": ["A", "B", "C", "D"],
        })
    );

    // A row a later condition rules out (`:has()`, `:last-child`) is left out, its place kept.
    let html = r#"<section data-day="fri"><ul><li><b>A</b></li><li>B</li><li><b>C</b></li></ul></section>"#;
    let spec = json!({
        "has": [{ "select": "li:has(b)", "fields": { "day": { "select": "section", "attr": "data-day", "closest": true } } }],
        "last": [{ "select": "li:last-child", "fields": { "day": { "select": "section", "attr": "data-day", "closest": true } } }],
    });
    assert_eq!(
        data(spec, html),
        json!({ "has": [{ "day": "fri" }, { "day": "fri" }], "last": [{ "day": "fri" }] })
    );
}

#[test]
fn closest_rows_emit_once_complete() {
    let x = Extractor::new(
        &json!({ "rows": { "select": "li", "all": true, "emit": true, "fields": { "act": "b", "day": { "select": "section", "attr": "data-day", "closest": true } } } }),
        &Grammars::new(),
        ExtractorOptions::default(),
    )
    .unwrap();
    let mut s = x.session_with(None, Some("utf-8"));
    s.write(br#"<section data-day="fri"><ul><li><b>A</b></li><li><b>B</b></li></ul>"#);
    assert!(s.take().is_empty(), "the section may yet hold a nearer match");
    s.write(b"</section><section data-day=sat><ul><li><b>C</b></li></ul></section>");
    let items: Vec<_> = s.take().into_iter().map(|(_, v)| v.to_json()).collect();
    assert_eq!(
        items,
        [
            json!({ "act": "A", "day": "fri" }),
            json!({ "act": "B", "day": "fri" }),
            json!({ "act": "C", "day": "sat" })
        ]
    );
}

#[test]
fn closest_spec_errors() {
    for (spec, error) in [
        (json!({ "d": { "select": "h3", "closest": true } }), "under a scope"),
        (
            json!({ "r": { "select": "li", "fields": { "d": [{ "select": "h3", "closest": true }] } } }),
            "not all",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "select": "h3", "all": true, "closest": true } } } }),
            "not all",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "closest": true } } } }),
            "needs a selector",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "select": "h3:last-child", "closest": true } } } }),
            "siblings",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "select": "> h3", "closest": true } } } }),
            "combinator",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "select": "h3", "closest": { "depth": 0 } } } } }),
            "at least 1",
        ),
        (
            json!({ "r": { "select": "li", "fields": { "d": { "select": "h3", "closest": 3 } } } }),
            "true or",
        ),
    ] {
        let e = Extractor::new(&spec, &Grammars::new(), ExtractorOptions::default()).unwrap_err();
        assert!(e.to_string().contains(error), "{spec}: {e}");
    }
}

#[test]
fn closest_keeps_little_on_a_long_page() {
    // Every `h3` is a candidate; once its section closes, only the first value inside is kept.
    let mut html = String::from("<main>");
    for i in 0..20_000 {
        html.push_str(&format!("<section><h3>Day {i}</h3><ul><li>{i}</li></ul></section>"));
    }
    html.push_str("</main>");
    let x = Extractor::new(
        &json!({ "rows": { "select": "li", "all": true, "emit": true, "fields": { "day": { "select": "h3", "closest": true } } } }),
        &Grammars::new(),
        ExtractorOptions::default(),
    )
    .unwrap();
    let mut s = x.session_with(None, Some("utf-8"));
    let mut n = 0;
    for chunk in html.as_bytes().chunks(4096) {
        s.write(chunk);
        for (_, v) in s.take() {
            assert_eq!(v.to_json(), json!({ "day": format!("Day {n}") }));
            n += 1;
        }
    }
    n += s.finish().items.len();
    assert_eq!(n, 20_000);
}

#[test]
fn text_as_lines() {
    let spec = json!({ "t": { "select": "body", "text": "lines" } });
    let html = "<body><h1>Title</h1><p>One</p><ul><li>a</li><li>b</li></ul></body>";
    assert_eq!(data(spec.clone(), html), json!({ "t": "Title\nOne\na\nb" }));
    let html = "<body>\n  <p>Nora<br>de <b>Nora</b>s\n  &amp; Co</p><br><br>\n<table><tr><td>Fri</td><td>20:00</td></tr><tr><th>Sat</th><td>21:00</td></table>\
                <script>if (a < b) x()</script><style>p{}</style><template><p>no</p></template><noscript>no</noscript>\
                <div><span>in</span><span>line</span></div></body>";
    assert_eq!(
        data(spec, html),
        json!({ "t": "Nora\nde Noras & Co\nFri 20:00\nSat 21:00\ninline" })
    );
    // Through a pattern, each line still its own; `collapse: false` keeps the breaks as they came.
    let html = "<div><p>a  b</p><p>c</p></div>";
    assert_eq!(
        data(
            json!({ "p": { "select": "div", "text": "lines", "pattern": "(?s)(.*)" }, "raw": { "select": "div", "text": "lines", "collapse": false } }),
            html
        ),
        json!({ "p": "a b\nc", "raw": "\na  b\n\nc\n" })
    );
}
