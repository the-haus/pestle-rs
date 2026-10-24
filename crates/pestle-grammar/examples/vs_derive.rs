//! pestle (grammar compiled at run time) against pest_derive (compiled to Rust at build time),
//! best of several runs.
//!
//!     cargo run --release -p pestle-grammar --example vs_derive
use std::time::Instant;

use pest::Parser;

#[derive(pest_derive::Parser)]
#[grammar = "../examples/grammars/json.pest"]
struct Json;

mod date {
    #[derive(pest_derive::Parser)]
    #[grammar = "../examples/grammars/date.pest"]
    pub struct Date;
}

fn best(n: usize, mut f: impl FnMut() -> usize) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        let mut s = 0;
        for _ in 0..n {
            s += f();
        }
        std::hint::black_box(s);
        b = b.min(t.elapsed().as_secs_f64() / n as f64);
    }
    b
}

fn main() {
    let mut input = String::from("[");
    for i in 0..6000 {
        if i > 0 {
            input.push(',');
        }
        input.push_str(&format!(
            r#"{{"id":{i},"name":"Event {i}","acts":["A","B","C"],"price":25.5,"soldOut":{}}}"#,
            i % 7 == 0
        ));
    }
    input.push(']');
    let g = pestle_grammar::Grammar::new(include_str!("grammars/json.pest")).unwrap();
    let ours = best(10, || g.parse(Some("json"), &input).unwrap().nodes.len());
    let derive = best(10, || Json::parse(Rule::json, &input).unwrap().flatten().count());
    let mb = input.len() as f64 / 1048576.0;
    println!(
        "JSON {:.1} MB: pestle {:.2} ms ({:.0} MB/s), pest_derive {:.2} ms ({:.0} MB/s): {:.1}×",
        mb,
        ours * 1e3,
        mb / ours,
        derive * 1e3,
        mb / derive,
        derive / ours
    );

    let dg = pestle_grammar::Grammar::new(include_str!("grammars/date.pest")).unwrap();
    let d = "za 14 november 2026 · 20:30";
    let ours = best(300_000, || dg.parse(None, d).unwrap().nodes.len());
    let derive = best(300_000, || {
        date::Date::parse(date::Rule::when, d).unwrap().flatten().count()
    });
    println!(
        "date: pestle {:.0} ns, pest_derive {:.0} ns: {:.1}×",
        ours * 1e9,
        derive * 1e9,
        derive / ours
    );
}
