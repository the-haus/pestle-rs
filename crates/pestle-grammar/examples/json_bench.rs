//! Native parse speed: pestle's VM against pest_vm, on pest's JSON grammar.
//!
//!     cargo run --release -p pestle-grammar --example json_bench
use std::time::Instant;

fn main() {
    let src = include_str!("../tests/pest/json.pest");
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
    let g = pestle_grammar::Grammar::new(src).unwrap();
    let (_, rules) = pest_meta::parse_and_optimize(src).unwrap();
    let vm = pest_vm::Vm::new(rules);
    let best = |f: &dyn Fn() -> usize| {
        let mut best = f64::MAX;
        let mut n = 0;
        for _ in 0..(std::env::var("N").ok().and_then(|n| n.parse().ok()).unwrap_or(10)) {
            let t = Instant::now();
            n = f();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        (best, n)
    };
    let mb = input.len() as f64 / 1048576.0;
    let (a, n) = best(&|| g.parse(Some("json"), &input).unwrap().nodes.len());
    println!("pestle: {a:.2} ms, {:.0} MB/s, {n} pairs", mb / (a / 1000.0));
    let (b, n) = best(&|| vm.parse("json", &input).unwrap().flatten().count());
    println!("pest_vm: {b:.2} ms, {:.0} MB/s, {n} pairs", mb / (b / 1000.0));
}
