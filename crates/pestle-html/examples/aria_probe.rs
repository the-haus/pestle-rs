//! Accessibility snapshots of saved pages, one `.yaml` each (and `.refs` in `ai` mode), read as
//! served from `https://pestle.test/<name>`; prints each page's size and time (`REPEAT=n`: the
//! mean of n runs):
//! `cargo run --release -p pestle-html --example aria_probe -- [--ai] [--concealed] <out dir> <files…>`.
use pestle_html::aria::{AriaMode, AriaOptions};
use pestle_html::{Extractor, ExtractorOptions};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = AriaOptions::default();
    let repeat: usize = std::env::var("REPEAT").ok().and_then(|r| r.parse().ok()).unwrap_or(1);
    args.retain(|a| match a.as_str() {
        "--ai" => {
            opts.mode = AriaMode::Ai;
            false
        }
        "--concealed" => {
            opts.concealed = true;
            false
        }
        _ => true,
    });
    let out = std::path::PathBuf::from(args.remove(0));
    std::fs::create_dir_all(&out).unwrap();
    let x = Extractor::new(
        &serde_json::json!({}),
        &Default::default(),
        ExtractorOptions {
            aria: Some(opts),
            ..Default::default()
        },
    )
    .unwrap();
    for f in &args {
        let html = std::fs::read(f).unwrap();
        let name = std::path::Path::new(f)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let url = format!("https://pestle.test/{name}");
        let t = std::time::Instant::now();
        let mut s = x.extract(&html, Some(&url)).aria.unwrap();
        for _ in 1..repeat {
            s = x.extract(&html, Some(&url)).aria.unwrap();
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / repeat as f64;
        std::fs::write(out.join(format!("{name}.yaml")), &s.yaml).unwrap();
        if !s.refs.is_empty() {
            let refs: Vec<String> = s.refs.iter().map(|r| format!("{}\t{}", r.id, r.selector)).collect();
            std::fs::write(out.join(format!("{name}.refs")), refs.join("\n")).unwrap();
        }
        println!("{name}\t{}\t{ms:.3}", html.len());
    }
}
