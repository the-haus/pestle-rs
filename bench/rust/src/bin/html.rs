//! The same extraction with every HTML parser: links (href and text),
//! headings (h2, h3), images (src) and the title (h1), on saved pages.
//!
//!     cargo run --release -p pestle-bench --bin html [pages dir]
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use serde_json::json;

#[derive(Default, Debug, PartialEq)]
struct Found {
    links: usize,
    headings: usize,
    images: usize,
    title: bool,
}

fn pestle(x: &pestle_html::Extractor, html: &[u8]) -> Found {
    let d = x.extract(html, None).data;
    let len = |k: &str| d.get(k).and_then(|v| v.as_array()).map_or(0, |a| a.len());
    Found {
        links: len("links"),
        headings: len("headings"),
        images: len("images"),
        title: d.get("title").is_some_and(|t| !t.is_null() && t.as_str() != Some("")),
    }
}

fn scraper(html: &str) -> Found {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let a = Selector::parse("a[href]").unwrap();
    let h = Selector::parse("h2, h3").unwrap();
    let img = Selector::parse("img[src]").unwrap();
    let h1 = Selector::parse("h1").unwrap();
    let links: Vec<(String, String)> = doc
        .select(&a)
        .map(|e| {
            (
                e.value().attr("href").unwrap_or("").to_owned(),
                e.text().collect::<String>(),
            )
        })
        .collect();
    let headings: Vec<String> = doc.select(&h).map(|e| e.text().collect::<String>()).collect();
    let images: Vec<String> = doc
        .select(&img)
        .map(|e| e.value().attr("src").unwrap_or("").to_owned())
        .collect();
    let title = doc.select(&h1).next().map(|e| e.text().collect::<String>());
    Found {
        links: links.len(),
        headings: headings.len(),
        images: images.len(),
        title: title.is_some(),
    }
}

fn lol(html: &[u8]) -> Found {
    use lol_html::{element, text, HtmlRewriter, Settings};
    let links = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let headings = Rc::new(RefCell::new(Vec::<String>::new()));
    let images = Rc::new(RefCell::new(Vec::<String>::new()));
    let title = Rc::new(RefCell::new(None::<String>));
    let (l1, l2, h1, h2, i1, t1, t2) = (
        links.clone(),
        links.clone(),
        headings.clone(),
        headings.clone(),
        images.clone(),
        title.clone(),
        title.clone(),
    );
    let settings = Settings::new()
        .append_element_content_handler(element!("a[href]", move |el| {
            l1.borrow_mut()
                .push((el.get_attribute("href").unwrap_or_default(), String::new()));
            Ok(())
        }))
        .append_element_content_handler(text!("a[href]", move |t| {
            if let Some(last) = l2.borrow_mut().last_mut() {
                last.1.push_str(t.as_str());
            }
            Ok(())
        }))
        .append_element_content_handler(element!("h2, h3", move |_| {
            h1.borrow_mut().push(String::new());
            Ok(())
        }))
        .append_element_content_handler(text!("h2, h3", move |t| {
            if let Some(last) = h2.borrow_mut().last_mut() {
                last.push_str(t.as_str());
            }
            Ok(())
        }))
        .append_element_content_handler(element!("img[src]", move |el| {
            i1.borrow_mut().push(el.get_attribute("src").unwrap_or_default());
            Ok(())
        }))
        .append_element_content_handler(element!("h1", move |_| {
            t1.borrow_mut().get_or_insert_with(String::new);
            Ok(())
        }))
        .append_element_content_handler(text!("h1", move |t| {
            if let Some(s) = t2.borrow_mut().as_mut() {
                s.push_str(t.as_str());
            }
            Ok(())
        }));
    let mut rw = HtmlRewriter::new(settings, |_: &[u8]| {});
    rw.write(html).unwrap();
    rw.end().unwrap();
    let found = Found {
        links: links.borrow().len(),
        headings: headings.borrow().len(),
        images: images.borrow().len(),
        title: title.borrow().is_some(),
    };
    found
}

fn tl_parse(html: &str) -> Found {
    let dom = tl::parse(html, tl::ParserOptions::default()).unwrap();
    let parser = dom.parser();
    let text = |h: tl::NodeHandle| {
        h.get(parser)
            .map(|n| n.inner_text(parser).into_owned())
            .unwrap_or_default()
    };
    let attr = |h: tl::NodeHandle, a: &str| {
        h.get(parser)
            .and_then(|n| n.as_tag())
            .and_then(|t| t.attributes().get(a).flatten().map(|v| v.as_utf8_str().into_owned()))
    };
    let links: Vec<(String, String)> = dom
        .query_selector("a[href]")
        .map(|q| q.map(|h| (attr(h, "href").unwrap_or_default(), text(h))).collect())
        .unwrap_or_default();
    let mut headings: Vec<String> = dom
        .query_selector("h2")
        .map(|q| q.map(text).collect())
        .unwrap_or_default();
    headings.extend(
        dom.query_selector("h3")
            .map(|q| q.map(text).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let images: Vec<String> = dom
        .query_selector("img[src]")
        .map(|q| q.filter_map(|h| attr(h, "src")).collect())
        .unwrap_or_default();
    let title = dom.query_selector("h1").and_then(|mut q| q.next()).map(text);
    Found {
        links: links.len(),
        headings: headings.len(),
        images: images.len(),
        title: title.is_some(),
    }
}

fn best(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_secs_f64());
    }
    b
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../.work").into());
    let mut pages: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("a directory of saved pages")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap_or_default(),
            )
        })
        .filter(|(_, b)| b.len() > 10_000)
        .collect();
    pages.sort();
    let x = pestle_html::Extractor::new(
        &json!({
            "links": { "select": "a[href]", "all": true, "fields": { "href": { "attr": "href", "collapse": false }, "text": { "text": "raw" } } },
            "headings": { "select": "h2, h3", "all": true, "text": "raw", "default": [] },
            "images": { "select": "img[src]", "attr": "src", "all": true, "collapse": false },
            "title": { "select": "h1", "text": "raw", "default": "" },
        }),
        &Default::default(),
        Default::default(),
    )
    .unwrap();
    let runs = 15;
    let mut totals = [0.0f64; 4];
    let mut bytes = 0;
    println!(
        "{:28} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "page", "size", "pestle", "lol_html", "tl", "scraper"
    );
    for (name, html) in &pages {
        let want = scraper(html);
        let t = [
            best(runs, || {
                std::hint::black_box(pestle(&x, html.as_bytes()));
            }),
            best(runs, || {
                std::hint::black_box(lol(html.as_bytes()));
            }),
            best(runs, || {
                std::hint::black_box(tl_parse(html));
            }),
            best(runs, || {
                std::hint::black_box(scraper(html));
            }),
        ];
        let agree = [pestle(&x, html.as_bytes()), lol(html.as_bytes()), tl_parse(html)];
        let marks: Vec<&str> = agree
            .iter()
            .map(|f| {
                if f.links == want.links && f.images == want.images {
                    ""
                } else {
                    "*"
                }
            })
            .collect();
        for (i, v) in t.iter().enumerate() {
            totals[i] += v;
        }
        bytes += html.len();
        println!(
            "{name:28} {:>6}kB {:>8.2}{:1} {:>8.2}{:1} {:>8.2}{:1} {:>8.2}ms",
            html.len() / 1024,
            t[0] * 1e3,
            marks[0],
            t[1] * 1e3,
            marks[1],
            t[2] * 1e3,
            marks[2],
            t[3] * 1e3
        );
    }
    let mb = bytes as f64 / 1048576.0;
    println!(
        "total MB/s: pestle {:.0}, lol_html {:.0}, tl {:.0}, scraper (html5ever) {:.0}   (* counts differ from html5ever's)",
        mb / totals[0],
        mb / totals[1],
        mb / totals[2],
        mb / totals[3]
    );
}
