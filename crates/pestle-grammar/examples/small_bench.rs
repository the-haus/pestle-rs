//! Short values parsed many times: the per-call cost (best of several runs).
//!
//!     cargo run --release -p pestle-grammar --example small_bench
use std::time::Instant;

fn best(label: &str, n: usize, mut f: impl FnMut()) {
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        for _ in 0..n {
            f();
        }
        best = best.min(t.elapsed().as_nanos() as f64 / n as f64);
    }
    println!("{label}: {best:.0} ns");
}

fn main() {
    let tiny = pestle_grammar::Grammar::new("a = { \"x\" }").unwrap();
    let g = pestle_grammar::Grammar::new(
        r#"
    WHITESPACE = _{ " " }
    when    = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ ("·" ~ #time = time)? }
    weekday = @{ ("ma" | "di" | "wo" | "do" | "vr" | "za" | "zo") ~ "."? }
    day     = @{ ASCII_DIGIT{1,2} }
    month   = @{ ASCII_ALPHA+ }
    year    = @{ ASCII_DIGIT{4} }
    time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }"#,
    )
    .unwrap();
    let d = "za 14 november 2026 · 20:30";
    let o = pestle_grammar::ShapeOptions::default();
    let mut sink = 0usize;
    best("trivial parse", 300_000, || {
        sink += tiny.parse(None, "x").unwrap().nodes.len()
    });
    best("date parse", 300_000, || sink += g.parse(None, d).unwrap().nodes.len());
    let mut out = String::new();
    best("date shape_json", 300_000, || {
        out.clear();
        g.shape_json(None, d, &o, &mut out).unwrap();
    });
    best("date shape (Value)", 200_000, || {
        sink += g.shape(None, d, &o).unwrap().as_object().map_or(0, |m| m.len())
    });
    println!("{out} {sink}");
}
