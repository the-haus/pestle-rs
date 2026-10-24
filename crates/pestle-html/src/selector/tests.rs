use super::Selector;
use crate::tokenizer::Tokenizer;
use crate::tree::{Builder, Handler, Tree, DEFAULT_MAX_DEPTH};

/// The ids of the elements a selector matches (no scope), in document order.
struct Collect<'s> {
    sel: &'s Selector,
    scope_id: Option<&'s str>,
    scope: Option<usize>,
    out: Vec<String>,
}

impl Handler for Collect<'_> {
    fn open(&mut self, tree: &Tree) {
        let top = tree.frames.len() - 1;
        let el = &tree.frames[top].el;
        if self.scope_id.is_some() && self.scope.is_none() {
            if el.attr("id") == self.scope_id {
                self.scope = Some(top);
            }
            return;
        }
        if self.scope_id.is_some() && self.scope.is_some_and(|s| top <= s) {
            return;
        }
        if self.sel.matches_now(tree, self.scope) {
            self.out.push(el.attr("id").unwrap_or(el.name()).to_owned());
        }
    }
    fn close(&mut self, tree: &Tree) {
        if self.scope == Some(tree.frames.len() - 1) {
            self.scope = None;
            self.scope_id = Some("\u{0}done");
        }
    }
    fn text(&mut self, _: &Tree, _: &crate::tree::Text<'_>) {}
    fn markup(&mut self, _: &Tree, _: &[u8]) {}
}

fn select_in(html: &str, selector: &str, scope: Option<&str>) -> Vec<String> {
    let sel = Selector::new(selector, scope.is_some()).unwrap();
    let mut b = Builder::new(
        Collect {
            sel: &sel,
            scope_id: scope,
            scope: None,
            out: Vec::new(),
        },
        sel.needs_siblings(),
        sel.needs_types(),
        DEFAULT_MAX_DEPTH,
    );
    let mut t = Tokenizer::new(crate::tokenizer::AttrFilter::all());
    t.write(html.as_bytes(), &mut b);
    t.end(&mut b);
    b.finish();
    b.handler.out
}

fn select(html: &str, selector: &str) -> Vec<String> {
    select_in(html, selector, None)
}

const PAGE: &str = r#"
<html><body>
  <main id="m" class="Event detail">
    <h1 id="t" class="title">Night</h1>
    <ul id="l" class="lineup">
      <li id="a" class="act headliner" data-slot="21:00">A</li>
      <li id="b" class="act">B</li>
      <li id="c" class="act">C</li>
      <li id="d" lang="nl-NL">D</li>
    </ul>
    <p id="p1">x</p><span id="s1"></span><p id="p2">y</p>
    <a id="k" href="https://tickets.example/e/1" rel="NoFollow">Tickets</a>
    <meta id="m1" itemprop="startDate" content="2026-11-14">
  </main>
</body></html>"#;

#[test]
fn simple_and_compound() {
    assert_eq!(select(PAGE, "li"), ["a", "b", "c", "d"]);
    assert_eq!(select(PAGE, "li.act"), ["a", "b", "c"]);
    assert_eq!(select(PAGE, ".act.headliner"), ["a"]);
    assert_eq!(select(PAGE, "#t, #k"), ["t", "k"]);
    assert_eq!(select(PAGE, "*[data-slot]"), ["a"]);
    assert_eq!(select(PAGE, ".event"), Vec::<String>::new(), "class is case-sensitive");
}

#[test]
fn attributes() {
    assert_eq!(select(PAGE, "[href^='https://tickets']"), ["k"]);
    assert_eq!(select(PAGE, "a[href$=\"/1\"]"), ["k"]);
    assert_eq!(select(PAGE, "a[href*=example]"), ["k"]);
    assert_eq!(
        select(PAGE, "[rel=nofollow]"),
        ["k"],
        "rel compares without case in HTML"
    );
    assert_eq!(select(PAGE, "[itemprop=STARTDATE]"), Vec::<String>::new());
    assert_eq!(select(PAGE, "[itemprop=STARTDATE i]"), ["m1"]);
    assert_eq!(select(PAGE, "[class~=act]"), ["a", "b", "c"]);
    assert_eq!(select(PAGE, "[lang|=nl]"), ["d"]);
    assert_eq!(select(PAGE, "li[class!=act]"), ["a", "d"]);
    assert_eq!(select(PAGE, "[href^='']"), Vec::<String>::new());
}

#[test]
fn combinators() {
    assert_eq!(select(PAGE, "main li"), ["a", "b", "c", "d"]);
    assert_eq!(select(PAGE, "main > li"), Vec::<String>::new());
    assert_eq!(select(PAGE, "ul > li.act"), ["a", "b", "c"]);
    assert_eq!(select(PAGE, "li + li"), ["b", "c", "d"]);
    assert_eq!(select(PAGE, ".headliner ~ li"), ["b", "c", "d"]);
    assert_eq!(select(PAGE, "h1 + ul > li:first-child"), ["a"]);
    assert_eq!(select(PAGE, "p + p"), Vec::<String>::new());
    assert_eq!(select(PAGE, "p ~ p"), ["p2"]);
    assert_eq!(select(PAGE, "body main > ul li#c"), ["c"]);
}

#[test]
fn structural_pseudos() {
    assert_eq!(select(PAGE, "li:first-child"), ["a"]);
    assert_eq!(select(PAGE, "li:nth-child(2n)"), ["b", "d"]);
    assert_eq!(select(PAGE, "li:nth-child(odd)"), ["a", "c"]);
    assert_eq!(select(PAGE, "li:nth-child(-n+2)"), ["a", "b"]);
    assert_eq!(select(PAGE, "main > :nth-of-type(2)"), ["p2"]);
    assert_eq!(select(PAGE, "p:first-of-type"), ["p1"]);
    assert_eq!(select(PAGE, ":root"), ["html"]);
    assert_eq!(select(PAGE, "li:not(.act)"), ["d"]);
    assert_eq!(select(PAGE, "li:not(.headliner, #d)"), ["b", "c"]);
    assert_eq!(select(PAGE, ":is(h1, a).title, :where(#k)"), ["t", "k"]);
    assert_eq!(select(PAGE, "li:not(ul > .act)"), ["d"]);
}

#[test]
fn scoped_selectors_are_relative() {
    assert_eq!(select_in(PAGE, "li", Some("l")), ["a", "b", "c", "d"]);
    assert_eq!(select_in(PAGE, "> li.act", Some("l")), ["a", "b", "c"]);
    assert_eq!(
        select_in(PAGE, "main li", Some("l")),
        Vec::<String>::new(),
        "main is outside the scope"
    );
    assert_eq!(select_in(PAGE, ":scope > h1", Some("m")), ["t"]);
    assert_eq!(select_in(PAGE, "> li", Some("m")), Vec::<String>::new());
}

#[test]
fn unsupported_and_bad_selectors_are_errors() {
    for bad in [
        "li:last-child a",
        "a::before",
        "a:hover",
        ":not(:has(a))",
        "+ li",
        "li[",
        "li:nth-child(x)",
        "",
    ] {
        assert!(Selector::new(bad, true).is_err(), "{bad:?} should not compile");
    }
    for good in [
        "li:last-child",
        "ul:has(> li.act)",
        "li:contains('B')",
        "p:empty",
        "li:nth-last-child(2)",
        "a\\:b",
        "[data-x=\"a]b\"]",
    ] {
        assert!(Selector::new(good, false).is_ok(), "{good:?} should compile");
    }
}
