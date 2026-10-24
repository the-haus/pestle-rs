//! What a field read becomes: the raw string, made absolute, cut, matched,
//! and then a number, JSON or a grammar's value.

#[cfg(feature = "grammar")]
use pestle_grammar::{Tree, NO_TAG};
#[cfg(feature = "grammar")]
use serde_json::json;
use serde_json::Value;

use crate::out;
use url::Url;

use crate::number::lenient_number;
#[cfg(feature = "grammar")]
use crate::spec::ParseDef;
use crate::spec::{FieldDef, Read, Then, UrlMode};
use crate::text::{collapse, collapse_owned, cut};

/// An href as festimap's crawler reads it: absolute, http(s) only, no
/// `#…`, `webcal:` as `https:`; none for an empty one, a fragment, or
/// another scheme (`mailto:`, `javascript:`).
pub fn http_url(href: &str, base: &Url) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let href = match href.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("webcal:") => format!("https:{}", &href[7..]),
        _ => href.to_owned(),
    };
    let mut url = base.join(&href).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

/// What values are read against: the base of URLs.
pub(crate) struct Context<'a> {
    pub(crate) base: Option<&'a Url>,
}

/// The value of one match, or none (and why, when it is a problem).
pub(crate) fn finish(def: &FieldDef, raw: String, cx: &Context<'_>, errors: &mut Vec<String>) -> Option<out::Value> {
    let base = cx.base;
    if matches!(def.read, Read::Exists) {
        return Some(out::Value::Bool(true));
    }
    let mut s = if def.collapse { collapse_owned(raw) } else { raw };
    if let Some(mode) = def.url {
        let href = match &def.read {
            // A srcset's first candidate.
            Read::Attr(a) if a.ends_with("srcset") => s.split_whitespace().next().unwrap_or("").to_owned(),
            _ => s,
        };
        s = match (mode, base) {
            (UrlMode::Http, Some(base)) => http_url(&href, base)?,
            (UrlMode::Resolve, Some(base)) => base.join(href.trim()).ok()?.to_string(),
            (_, None) => Url::parse(href.trim()).ok()?.to_string(),
        };
    }
    if let Some(max) = def.max {
        let cut_at = cut(&s, max).len();
        s.truncate(cut_at);
    }
    if let Some(re) = &def.pattern {
        let caps = re.captures(&s)?;
        let m = caps.get(1).or_else(|| caps.get(0))?;
        s = if def.collapse {
            collapse(m.as_str())
        } else {
            m.as_str().to_owned()
        };
    }
    if s.is_empty() {
        return None;
    }
    match &def.then {
        Then::Nothing => Some(out::Value::String(s)),
        Then::Number => lenient_number(&s).filter(|n| n.is_finite()).map(out::Value::Number),
        Then::Json => match parse_json(&s) {
            Ok(v) => Some(out::Value::Json(v)),
            Err(e) => {
                errors.push(format!("{}: json: {e}", def.path));
                None
            }
        },
        #[cfg(feature = "grammar")]
        Then::Parse(p) => grammar_value(p, &s, &def.path, errors).map(out::Value::Json),
    }
}

/// JSON as sites embed it: in CDATA or a comment, with raw control characters in strings.
pub fn parse_json(text: &str) -> Result<Value, serde_json::Error> {
    let mut t = text.trim();
    for prefix in ["//<![CDATA[", "// <![CDATA[", "<![CDATA[", "<!--"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
        }
    }
    for suffix in ["//]]>", "// ]]>", "]]>", "-->"] {
        if let Some(rest) = t.strip_suffix(suffix) {
            t = rest.trim_end();
        }
    }
    serde_json::from_str(t).or_else(|e| {
        if !t.bytes().any(|b| b < 0x20) {
            return Err(e);
        }
        // Control characters inside strings (a pasted description): as spaces, then retry.
        let mut cleaned = String::with_capacity(t.len());
        let mut run = false;
        for c in t.chars() {
            if (c as u32) < 0x20 {
                if !run {
                    cleaned.push(' ');
                }
                run = true;
            } else {
                cleaned.push(c);
                run = false;
            }
        }
        serde_json::from_str(&cleaned)
    })
}

#[cfg(feature = "grammar")]
fn grammar_value(p: &ParseDef, s: &str, path: &str, errors: &mut Vec<String>) -> Option<Value> {
    let rule = p.rule.as_deref();
    let g = &p.grammar;
    let value = |tree: &Tree| {
        if p.tree {
            tree_json(tree, s, g.names(), g.tags())
        } else {
            g.shape_tree(tree, s, &p.shape)
        }
    };
    if p.all {
        let found = g.find_all(rule, s, p.limit).ok()?;
        return (!found.is_empty()).then(|| Value::Array(found.iter().map(|f| value(&f.tree)).collect()));
    }
    if p.find {
        return g.find(rule, s).ok().flatten().map(|f| value(&f.tree));
    }
    match g.parse(rule, s) {
        Ok(tree) => Some(value(&tree)),
        Err(e) => {
            let first = e.message.lines().last().unwrap_or("").trim().trim_start_matches("= ");
            errors.push(format!("{path}: grammar {:?} at {}:{}: {first}", p.name, e.line, e.col));
            None
        }
    }
}

/// A tree as nested `{ rule, tag?, text, children }`.
#[cfg(feature = "grammar")]
pub fn tree_json(tree: &Tree, input: &str, names: &[String], tags: &[String]) -> Value {
    fn node(tree: &Tree, i: usize, input: &str, names: &[String], tags: &[String]) -> Value {
        let n = &tree.nodes[i];
        let children: Vec<Value> = tree.children(i).map(|c| node(tree, c, input, names, tags)).collect();
        let mut o = json!({ "rule": names[n.rule as usize], "text": tree.text(i, input) });
        if n.tag != NO_TAG {
            o["tag"] = Value::String(tags[n.tag as usize].clone());
        }
        if !children.is_empty() {
            o["children"] = Value::Array(children);
        }
        o
    }
    let roots: Vec<Value> = tree.roots().map(|r| node(tree, r, input, names, tags)).collect();
    if roots.len() == 1 {
        roots.into_iter().next().unwrap_or(Value::Null)
    } else {
        Value::Array(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn http_urls_like_festimap() {
        let base = Url::parse("https://venue.example/agenda/").unwrap();
        assert_eq!(
            http_url("../e/1#top", &base).as_deref(),
            Some("https://venue.example/e/1")
        );
        assert_eq!(
            http_url("webcal://venue.example/a.ics", &base).as_deref(),
            Some("https://venue.example/a.ics")
        );
        assert_eq!(http_url("mailto:a@b", &base), None);
        assert_eq!(http_url("#x", &base), None);
        assert_eq!(http_url("  ", &base), None);
    }

    #[test]
    fn json_as_embedded() {
        assert_eq!(parse_json("//<![CDATA[\n{\"a\":1}\n//]]>").unwrap(), json!({ "a": 1 }));
        assert_eq!(
            parse_json("{\"d\":\"line\nbreak\"}").unwrap(),
            json!({ "d": "line break" })
        );
        assert!(parse_json("{nope").is_err());
    }
}
