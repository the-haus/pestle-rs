use serde_json::json;

use crate::{Grammar, Recovery, ShapeOptions, NO_TAG};

/// One pair as (rule, start, end, tag), in document order.
type Flat = Vec<(String, usize, usize, Option<String>)>;

fn ours(g: &Grammar, rule: &str, input: &str) -> Result<Flat, (usize, Vec<String>)> {
    match g.parse(Some(rule), input) {
        Ok(tree) => Ok(tree
            .nodes
            .iter()
            .map(|n| {
                let tag = (n.tag != NO_TAG).then(|| g.tags()[n.tag as usize].clone());
                (
                    g.names()[n.rule as usize].clone(),
                    n.start as usize,
                    n.end as usize,
                    tag,
                )
            })
            .collect()),
        Err(e) => {
            let mut expected = e.expected;
            expected.sort();
            Err((e.offset, expected))
        }
    }
}

fn theirs(source: &str, rule: &str, input: &str) -> Result<Flat, (usize, Vec<String>)> {
    let (_, rules) = pest_meta::parse_and_optimize(source).unwrap();
    let vm = pest_vm::Vm::new(rules);
    match vm.parse(rule, input) {
        Ok(pairs) => Ok(pairs
            .flatten()
            .map(|p| {
                let span = p.as_span();
                (
                    p.as_rule().to_owned(),
                    span.start(),
                    span.end(),
                    p.as_node_tag().map(str::to_owned),
                )
            })
            .collect()),
        Err(e) => {
            let mut expected = match e.variant {
                pest::error::ErrorVariant::ParsingError { positives, .. } => {
                    positives.into_iter().map(str::to_owned).collect()
                }
                _ => Vec::new(),
            };
            expected.sort();
            let at = match e.location {
                pest::error::InputLocation::Pos(p) | pest::error::InputLocation::Span((p, _)) => p,
            };
            Err((at, expected))
        }
    }
}

fn same(source: &str, rule: &str, input: &str) {
    let g = Grammar::new(source).unwrap();
    let a = ours(&g, rule, input);
    let b = theirs(source, rule, input);
    assert_eq!(a, b, "rule {rule} on {input:?}");
}

#[test]
fn pest_grammars_parse_as_pest_vm_does() {
    let json = include_str!("../tests/pest/json.pest");
    same(json, "json", include_str!("../tests/pest/examples.json"));
    same(json, "json", r#"{"a": [1, 2.5e3, true, null, "é"], "b": {}}"#);
    same(json, "json", r#"{"a": [1, 2,, ]}"#);

    let toml = include_str!("../tests/pest/toml.pest");
    same(toml, "toml", include_str!("../tests/pest/examples.toml"));
    same(toml, "toml", "a = [1, 2\nb = 3");

    let http = include_str!("../tests/pest/http.pest");
    same(http, "http", include_str!("../tests/pest/examples.http"));

    let sql = include_str!("../tests/pest/sql.pest");
    for q in [
        "select a, b from t where a > 1 and b = 'x'",
        "create user \"u\" with password 'p'",
        "insert into t (a, b) select a, b from s",
        "select from",
    ] {
        same(sql, "Command", q);
    }
}

#[test]
fn stack_tags_and_extras_match_pest_vm() {
    // PUSH/POP/PEEK, PEEK slices, DROP, node tags, `+`, PUSH_LITERAL, atomicity, unicode.
    let g = r##"
        WHITESPACE = _{ " " }
        raw     = ${ "r" ~ PUSH("#"*) ~ "\"" ~ body ~ "\"" ~ POP }
        body    = @{ (!("\"" ~ PEEK) ~ ANY)* }
        list    = { "[" ~ #item = word ~ ("," ~ #item = word)* ~ "]" }
        word    = @{ LETTER+ }
        lit     = { PUSH_LITERAL("x") ~ PEEK ~ DROP ~ "y" }
        slice   = { PUSH("a") ~ PUSH("b") ~ PEEK[0..2] }
        nonatom = !{ "(" ~ word ~ word ~ ")" }
        compound = ${ word ~ nonatom }
        insens  = { ^"ab" ~ 'x'..'z'+ ~ NEWLINE? ~ ASCII_HEX_DIGIT ~ EOI }
    "##;
    same(g, "raw", "r##\"a \"# b\"##");
    same(g, "list", "[ één , twee,drie ]");
    same(g, "list", "[a,]");
    same(g, "lit", "xy");
    same(g, "slice", "abab");
    same(g, "compound", "ab( c d )");
    same(g, "insens", "aBxyz\nF");
    same(g, "insens", "aBxyz\nG");
}

#[test]
fn errors_name_rules_and_positions() {
    let g = Grammar::new("time = { num ~ \":\" ~ num }\nnum = @{ ASCII_DIGIT{2} }").unwrap();
    // Like pest, a failure is reported at the furthest rule tried (literals are not rules).
    let e = g.parse(None, "20:3x").unwrap_err();
    assert_eq!((e.line, e.col, e.offset), (1, 4, 3));
    assert_eq!(e.expected, ["num"]);
    assert!(e.message.contains("expected"), "{}", e.message);

    let bad = Grammar::new("a = { b ~ }").unwrap_err();
    assert_eq!(bad.errors[0].line, 1);
    let undefined = Grammar::new("a = { nope }").unwrap_err();
    assert!(undefined.to_string().contains("nope"), "{undefined}");

    assert!(g.parse(Some("missing"), "1").unwrap_err().message.contains("missing"));
}

#[test]
fn shape_by_tags() {
    let g = Grammar::new(
        r#"
        WHITESPACE = _{ " " }
        show  = { #date = date ~ ("·" ~ #doors = time)? ~ ("€" ~ #price = amount)* }
        date  = { #day = num ~ #month = month ~ (#year = year)? }
        time  = @{ num ~ ":" ~ num }
        num   = @{ ASCII_DIGIT{1,2} }
        year  = @{ ASCII_DIGIT{4} }
        month = @{ LETTER+ }
        amount = @{ ASCII_DIGIT+ ~ ("," ~ ("-" | ASCII_DIGIT{2}))? }
    "#,
    )
    .unwrap();
    let mut options = ShapeOptions::default();
    let v = g
        .shape(None, "14 november 2026 · 20:00 € 25,- € 30,50", &options)
        .unwrap();
    assert_eq!(
        v,
        json!({
            "date": { "day": "14", "month": "november", "year": "2026" },
            "doors": "20:00",
            "price": ["25,-", "30,50"],
        })
    );
    options.numbers.insert("price".into());
    options.numbers.insert("day".into());
    options.lists.insert("doors".into());
    let v = g.shape(None, "3 mei · 21:30 € 12", &options).unwrap();
    assert_eq!(
        v,
        json!({ "date": { "day": 3.0, "month": "mei" }, "doors": ["21:30"], "price": 12.0 })
    );

    options = ShapeOptions {
        by_rule: true,
        ..Default::default()
    };
    let v = g.shape(Some("date"), "3 mei", &options).unwrap();
    assert_eq!(v, json!({ "num": "3", "month": "mei" }));
}

#[test]
fn a_tag_on_an_optional_tags_the_pair_before_as_pest_does() {
    // pest tags the last pair made: with nothing matched, the pair before.
    let src = "d = { #day = n ~ #year = y? } n = @{ ASCII_DIGIT{2} } y = @{ ASCII_DIGIT{4} }";
    same(src, "d", "14");
    let g = Grammar::new(src).unwrap();
    assert_eq!(
        g.shape(None, "14", &ShapeOptions::default()).unwrap(),
        json!({ "year": "14" })
    );
}

#[test]
fn find_searches_like_a_regex() {
    let g = Grammar::new("time = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }").unwrap();
    let text = "Deuren 19:30 · aanvang 20:15 · €";
    let found = g.find(None, text).unwrap().unwrap();
    assert_eq!(&text[found.start..found.end], "19:30");
    assert_eq!(found.tree.nodes[0].start as usize, found.start);
    let all: Vec<&str> = g
        .find_all(None, text, 10)
        .unwrap()
        .iter()
        .map(|f| &text[f.start..f.end])
        .collect();
    assert_eq!(all, ["19:30", "20:15"]);
    assert!(g.find(None, "geen tijd").unwrap().is_none());
}

#[test]
fn utf16_spans_count_like_javascript() {
    let g = Grammar::new("w = { (#x = x | \" \")* } x = @{ (!\" \" ~ ANY)+ }").unwrap();
    let input = "é 🎸 a";
    let tree = g.parse(None, input).unwrap();
    let spans = tree.utf16_spans(input);
    // "é" is one unit, "🎸" two.
    assert_eq!(spans, vec![(0, 6), (0, 1), (2, 4), (5, 6)]);
    assert_eq!(tree.utf16_end(input), 6);
}

#[test]
fn a_runaway_grammar_fails_instead_of_crashing() {
    // Left recursion is refused by the validator; deep right recursion hits the stack guard.
    assert!(Grammar::new("a = { a ~ \"x\" }").is_err());
    let g = Grammar::new("a = { \"(\" ~ a? ~ \")\" }").unwrap();
    let deep = "(".repeat(200_000) + &")".repeat(200_000);
    assert!(g.parse(None, &deep).is_err());
}

/// A tiny PRNG for mutations.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    const POOL: &[char] = &[
        '"', '\\', '{', '}', '[', ']', ',', ':', ' ', '\n', '=', '#', 'a', '1', '.', 'é', '(', ')', '\'', 'r',
    ];
    for _ in 0..1 + rng.below(3) {
        if chars.is_empty() {
            break;
        }
        let at = rng.below(chars.len());
        match rng.below(4) {
            0 => {
                chars.remove(at);
            }
            1 => chars.insert(at, POOL[rng.below(POOL.len())]),
            2 => chars[at] = POOL[rng.below(POOL.len())],
            _ => chars.truncate(at),
        }
    }
    chars.into_iter().collect()
}

#[test]
fn the_fast_engine_agrees_with_pest_on_mutated_inputs() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            include_str!("../tests/pest/json.pest"),
            "json",
            &[
                include_str!("../tests/pest/examples.json"),
                r#"{"a": [1, -2.5e3, true, null, "\u00e9"]}"#,
            ],
        ),
        (
            include_str!("../tests/pest/toml.pest"),
            "toml",
            &[include_str!("../tests/pest/examples.toml")],
        ),
        (
            include_str!("../tests/pest/http.pest"),
            "http",
            &[include_str!("../tests/pest/examples.http")],
        ),
        (
            include_str!("../tests/pest/sql.pest"),
            "Command",
            &[
                "select a, b from t where a > 1 and b = 'x'",
                "insert into t (a, b) select a, b from s",
            ],
        ),
        (
            r##"
            WHITESPACE = _{ " " }
            doc     = { SOI ~ (raw | list | lit | slice | compound)* ~ EOI }
            raw     = ${ "r" ~ PUSH("#"*) ~ "\"" ~ body ~ "\"" ~ POP }
            body    = @{ (!("\"" ~ PEEK) ~ ANY)* }
            list    = { "[" ~ #item = word ~ ("," ~ #item = word)* ~ "]" }
            word    = @{ LETTER+ }
            lit     = { PUSH_LITERAL("x") ~ PEEK ~ DROP ~ "y" }
            slice   = { "<" ~ PUSH("a") ~ PUSH("b") ~ PEEK[0..2] ~ DROP ~ DROP ~ ">" }
            compound = ${ "c" ~ word ~ nonatom }
            nonatom = !{ "(" ~ word ~ word ~ ")" }
            "##,
            "doc",
            &[r###"r##"a "# b"## [ één , twee,drie ] xy <abab> cab( c d )"###],
        ),
    ];
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let mut agreed = 0;
    for (src, rule, samples) in cases {
        let g = Grammar::new(src).unwrap();
        let id = g.rule_id(rule).unwrap().0;
        for sample in *samples {
            for i in 0..600 {
                let input = if i == 0 {
                    sample.to_string()
                } else {
                    mutate(&mut rng, sample)
                };
                let fast = g.fast.run(id, &input, None).map(|(t, _)| t);
                let pest = g.run_pest(id, &input).ok();
                assert_eq!(fast, pest, "rule {rule} on {input:?}");
                agreed += 1;
            }
        }
    }
    assert!(agreed > 4000);
}

#[test]
fn recovery_skips_to_a_sync_point_and_goes_on() {
    let g = Grammar::new(
        r##"
        WHITESPACE = _{ " " | "\n" }
        lineup = { SOI ~ act* ~ EOI }
        act    = { #name = name ~ "@" ~ #time = time ~ ";" }
        name   = @{ (ASCII_ALPHA | " ")+ }
        time   = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }
        "##,
    )
    .unwrap();
    let input = "Nora @ 20:00; Mira @ 2l:30; Lu @ 23:00;";
    assert!(g.parse(None, input).is_err(), "without recovery the whole parse fails");
    let recovery = Recovery {
        rules: vec!["act".into()],
        sync: vec![";".into()],
        consume: true,
    };
    let (tree, recovered) = g.parse_recovering(None, input, &recovery).unwrap();
    assert_eq!(recovered.len(), 1);
    let r = &recovered[0];
    assert_eq!(r.rule, "act");
    assert_eq!(&input[r.start..r.end], "Mira @ 2l:30;");
    assert_eq!(r.at, input.find("2l").unwrap() + 1, "it broke after the 2");
    assert!(r.error.message.contains("act broke here"), "{}", r.error.message);
    let names: Vec<&str> = tree.nodes.iter().map(|n| g.names()[n.rule as usize].as_str()).collect();
    assert!(names.contains(&"ERROR"));
    let v = g.shape_tree(&tree, input, &ShapeOptions::default());
    assert_eq!(v, json!({ "name": ["Nora ", "Lu "], "time": ["20:00", "23:00"] }));
    // A rule that fails at its first character is no error: that is how a repetition ends.
    let (_, none) = g.parse_recovering(None, "Nora @ 20:00;", &recovery).unwrap();
    assert!(none.is_empty());
}

#[test]
fn shape_json_is_shape_as_text() {
    let g = Grammar::new(
        r#"
        WHITESPACE = _{ " " }
        show  = { #date = date ~ ("·" ~ #doors = time)? ~ ("€" ~ #price = amount)* ~ #note = note? }
        date  = { #day = num ~ #month = month ~ (#year = year)? }
        time  = @{ num ~ ":" ~ num }
        num   = @{ ASCII_DIGIT{1,2} }
        year  = @{ ASCII_DIGIT{4} }
        month = @{ LETTER+ }
        amount = @{ ASCII_DIGIT+ ~ ("," ~ ("-" | ASCII_DIGIT{2}))? }
        note  = @{ "\"" ~ (!"\"" ~ ANY)* ~ "\"" }
    "#,
    )
    .unwrap();
    let inputs = [
        "14 november 2026 · 20:00 € 25,- € 30,50",
        "3 mei",
        "3 mei \"a \\ b\\n\"",
        "1 é",
    ];
    let mut options = ShapeOptions::default();
    for round in 0..3 {
        if round == 1 {
            options.numbers.insert("price".into());
            options.lists.insert("doors".into());
        }
        if round == 2 {
            options = ShapeOptions {
                by_rule: true,
                trim: true,
                ..Default::default()
            };
        }
        for input in inputs {
            let Ok(want) = g.shape(None, input, &options) else {
                continue;
            };
            let mut text = String::new();
            g.shape_json(None, input, &options, &mut text).unwrap();
            let got: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
            assert_eq!(got, want, "{input}: {text}");
        }
    }
}
