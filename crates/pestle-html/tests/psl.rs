//! The Public Suffix List's own tests (`tests/data/test_psl.txt`, from publicsuffix/list).

use pestle_html::psl::{public_suffix, registrable_domain};

#[test]
fn the_lists_own_tests() {
    let tests = include_str!("data/test_psl.txt");
    let quoted = |s: &str| -> Option<String> {
        let s = s.trim();
        (s != "null").then(|| s.trim_matches('\'').to_owned())
    };
    let mut n = 0;
    for line in tests.lines().map(str::trim) {
        let Some(args) = line
            .strip_prefix("checkPublicSuffix(")
            .and_then(|l| l.strip_suffix(");"))
        else {
            continue;
        };
        let (input, want) = args.split_once(',').expect("two arguments");
        let (Some(input), want) = (quoted(input), quoted(want)) else {
            continue;
        };
        let got = registrable_domain(&input);
        match want {
            Some(w) => assert_eq!(got, Some(idna::domain_to_ascii(&w).unwrap()), "{input}"),
            // The list says none for a host that is a public suffix: here it is its own.
            None => match &got {
                None => {}
                Some(g) => assert_eq!(public_suffix(&input).as_ref(), Some(g), "{input} is no suffix"),
            },
        }
        n += 1;
    }
    assert!(n > 70, "{n} tests");
}
