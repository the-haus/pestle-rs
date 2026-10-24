use super::*;

fn yaml(html: &str) -> String {
    snapshot(html.as_bytes(), None, AriaOptions::default()).yaml
}

#[test]
fn landmarks_links_and_headings() {
    let html = r#"<!doctype html><html><head><title>T</title></head><body>
<nav aria-label="Main"><a href="/a">Agenda</a></nav>
<main><h1>Nora &amp; Co</h1><p>Live <b>tonight</b></p></main></body></html>"#;
    assert_eq!(
        yaml(html),
        "- navigation \"Main\":\n  - link \"Agenda\":\n    - /url: /a\n- main:\n  - heading \"Nora & Co\" [level=1]\n  - paragraph: Live tonight"
    );
}
