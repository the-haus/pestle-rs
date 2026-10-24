use std::time::Instant;
fn best(label: &str, g: &pestle_grammar::Grammar, input: &str) {
    let mut b = f64::MAX;
    let mut s = 0;
    for _ in 0..7 {
        let t = Instant::now();
        for _ in 0..300_000 {
            s += g.parse(None, input).unwrap().nodes.len();
        }
        b = b.min(t.elapsed().as_nanos() as f64 / 300_000.0);
    }
    println!("{label:40} {b:6.0} ns ({s})");
}
fn main() {
    let cases = [
        ("d m", "w = { #day = day ~ #month = month }\nday     = @{ ASCII_DIGIT{1,2} }\nmonth   = @{ ASCII_ALPHA+ }\nyear    = @{ ASCII_DIGIT{4} }\ntime    = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }\nweekday = @{ (\"ma\" | \"di\" | \"wo\" | \"do\" | \"vr\" | \"za\" | \"zo\") ~ \".\"? }\nWHITESPACE = _{ \" \" }", "14 november"),
        ("wd d m", "w = { weekday? ~ #day = day ~ #month = month }\nday     = @{ ASCII_DIGIT{1,2} }\nmonth   = @{ ASCII_ALPHA+ }\nyear    = @{ ASCII_DIGIT{4} }\ntime    = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }\nweekday = @{ (\"ma\" | \"di\" | \"wo\" | \"do\" | \"vr\" | \"za\" | \"zo\") ~ \".\"? }\nWHITESPACE = _{ \" \" }", "za 14 november"),
        ("wd d m y", "w = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? }\nday     = @{ ASCII_DIGIT{1,2} }\nmonth   = @{ ASCII_ALPHA+ }\nyear    = @{ ASCII_DIGIT{4} }\ntime    = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }\nweekday = @{ (\"ma\" | \"di\" | \"wo\" | \"do\" | \"vr\" | \"za\" | \"zo\") ~ \".\"? }\nWHITESPACE = _{ \" \" }", "za 14 november 2026"),
        ("full", "w = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ (\"·\" ~ #time = time)? }\nday     = @{ ASCII_DIGIT{1,2} }\nmonth   = @{ ASCII_ALPHA+ }\nyear    = @{ ASCII_DIGIT{4} }\ntime    = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }\nweekday = @{ (\"ma\" | \"di\" | \"wo\" | \"do\" | \"vr\" | \"za\" | \"zo\") ~ \".\"? }\nWHITESPACE = _{ \" \" }", "za 14 november 2026 · 20:30"),
        ("full ascii dot", "w = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ (\".\" ~ #time = time)? }\nday     = @{ ASCII_DIGIT{1,2} }\nmonth   = @{ ASCII_ALPHA+ }\nyear    = @{ ASCII_DIGIT{4} }\ntime    = @{ ASCII_DIGIT{2} ~ \":\" ~ ASCII_DIGIT{2} }\nweekday = @{ (\"ma\" | \"di\" | \"wo\" | \"do\" | \"vr\" | \"za\" | \"zo\") ~ \".\"? }\nWHITESPACE = _{ \" \" }", "za 14 november 2026 . 20:30"),
    ];
    for (label, src, input) in cases {
        best(label, &pestle_grammar::Grammar::new(src).unwrap(), input);
    }
}
