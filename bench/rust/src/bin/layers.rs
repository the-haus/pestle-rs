//! pestle's layers on saved pages: tokenizer, tree, and tl for scale.
use std::time::Instant;
fn best(f: &mut dyn FnMut()) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..15 {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_secs_f64());
    }
    b
}
#[allow(clippy::type_complexity)]
fn main() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../.work");
    let pages: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect();
    let mb = pages.iter().map(|p| p.len()).sum::<usize>() as f64 / 1048576.0;
    let some: &[&str] = &["href", "src", "id", "class"];
    let rows: Vec<(&str, Box<dyn FnMut()>)> = vec![
        (
            "tokenize, all attributes",
            Box::new(|| {
                for p in &pages {
                    std::hint::black_box(pestle_html::layers::tokenize(p.as_bytes(), None));
                }
            }),
        ),
        (
            "tokenize, 4 attributes",
            Box::new(|| {
                for p in &pages {
                    std::hint::black_box(pestle_html::layers::tokenize(p.as_bytes(), Some(some)));
                }
            }),
        ),
        (
            "tokenize, no attributes",
            Box::new(|| {
                for p in &pages {
                    std::hint::black_box(pestle_html::layers::tokenize(p.as_bytes(), Some(&[])));
                }
            }),
        ),
        (
            "tree, 4 attributes",
            Box::new(|| {
                for p in &pages {
                    std::hint::black_box(pestle_html::layers::tree(p.as_bytes(), Some(some)));
                }
            }),
        ),
        (
            "tl parse",
            Box::new(|| {
                for p in &pages {
                    std::hint::black_box(tl::parse(p, tl::ParserOptions::default()).unwrap().nodes().len());
                }
            }),
        ),
    ];
    let spec = |v: serde_json::Value| pestle_html::Extractor::new(&v, &Default::default(), Default::default()).unwrap();
    let empty = spec(serde_json::json!({}));
    let flat = spec(
        serde_json::json!({ "hrefs": ["a[href]@href"], "texts": { "select": "a[href]", "all": true, "text": "raw" } }),
    );
    let nested = spec(
        serde_json::json!({ "links": { "select": "a[href]", "all": true, "fields": { "href": { "attr": "href", "collapse": false }, "text": { "text": "raw" } } } }),
    );
    let matched = spec(serde_json::json!({ "n": { "select": "a[href]", "all": true, "exists": true } }));
    let rows2: Vec<(&str, &pestle_html::Extractor)> = vec![
        ("extract {}", &empty),
        ("extract a[href] exists", &matched),
        ("extract hrefs + texts (flat)", &flat),
        ("extract links {href,text}", &nested),
    ];
    for (name, mut f) in rows {
        let t = best(&mut f);
        println!("{name:28} {:7.0} MB/s", mb / t);
    }
    for (name, x) in rows2 {
        let t = best(&mut || {
            for p in &pages {
                std::hint::black_box(x.extract(p.as_bytes(), None));
            }
        });
        println!("{name:28} {:7.0} MB/s", mb / t);
    }
}
