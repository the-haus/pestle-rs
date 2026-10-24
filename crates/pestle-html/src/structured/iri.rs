//! IRI reference resolution as RFC 3986 § 5.2 has it (which RDFa 1.1 and
//! JSON-LD 1.1 use): strings are kept as written (no case folding, no
//! percent-encoding, no trailing slash added; `stéphane` stays `stéphane`),
//! only dot segments are removed.

/// The parts of an IRI reference (RFC 3986 Appendix B).
struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn parts(r: &str) -> Parts<'_> {
    let (r, fragment) = match r.find('#') {
        Some(i) => (&r[..i], Some(&r[i + 1..])),
        None => (r, None),
    };
    let (r, query) = match r.find('?') {
        Some(i) => (&r[..i], Some(&r[i + 1..])),
        None => (r, None),
    };
    // A scheme: letters, digits, `+-.` before the first `:`, before any `/`.
    let (scheme, r) = match r.find(':') {
        Some(i)
            if i > 0
                && r.as_bytes()[0].is_ascii_alphabetic()
                && r[..i]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')) =>
        {
            (Some(&r[..i]), &r[i + 1..])
        }
        _ => (None, r),
    };
    let (authority, path) = match r.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find('/').unwrap_or(rest.len());
            (Some(&rest[..end]), &rest[end..])
        }
        None => (None, r),
    };
    Parts {
        scheme,
        authority,
        path,
        query,
        fragment,
    }
}

/// RFC 3986 § 5.2.4.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut out = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../").or_else(|| input.strip_prefix("./")) {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input == "/.." { "/" } else { &input[3..] };
            match out.rfind('/') {
                Some(i) => out.truncate(i),
                None => out.clear(),
            }
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..].find('/').map_or(input.len(), |i| start + i);
            out.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    out
}

/// `reference` resolved against `base` (RFC 3986 § 5.2.2); as written when it is absolute and has
/// no dot segments.
pub fn resolve(base: &str, reference: &str) -> String {
    let r = parts(reference);
    let b = parts(base);
    let (scheme, authority, path, query);
    if r.scheme.is_some() {
        scheme = r.scheme;
        authority = r.authority;
        path = remove_dot_segments(r.path);
        query = r.query;
    } else {
        scheme = b.scheme;
        if r.authority.is_some() {
            authority = r.authority;
            path = remove_dot_segments(r.path);
            query = r.query;
        } else {
            authority = b.authority;
            if r.path.is_empty() {
                path = b.path.to_owned();
                query = r.query.or(b.query);
            } else {
                path = if r.path.starts_with('/') {
                    remove_dot_segments(r.path)
                } else {
                    // § 5.2.3, merge.
                    let merged = if b.authority.is_some() && b.path.is_empty() {
                        format!("/{}", r.path)
                    } else {
                        match b.path.rfind('/') {
                            Some(i) => format!("{}{}", &b.path[..=i], r.path),
                            None => r.path.to_owned(),
                        }
                    };
                    remove_dot_segments(&merged)
                };
                query = r.query;
            }
        }
    }
    // § 5.3, recomposition.
    let mut out = String::with_capacity(base.len() + reference.len());
    if let Some(s) = scheme {
        out.push_str(s);
        out.push(':');
    }
    if let Some(a) = authority {
        out.push_str("//");
        out.push_str(a);
    }
    out.push_str(&path);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = r.fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Whether `s` is an absolute IRI (it has a scheme).
pub fn is_absolute(s: &str) -> bool {
    parts(s).scheme.is_some()
}

#[cfg(test)]
mod tests {
    use super::resolve;

    /// RFC 3986 § 5.4.1 and § 5.4.2, every example.
    #[test]
    fn the_rfc_examples() {
        let base = "http://a/b/c/d;p?q";
        for (r, want) in [
            ("g:h", "g:h"),
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q#s"),
            ("g#s", "http://a/b/c/g#s"),
            ("g?y#s", "http://a/b/c/g?y#s"),
            (";x", "http://a/b/c/;x"),
            ("g;x", "http://a/b/c/g;x"),
            ("g;x?y#s", "http://a/b/c/g;x?y#s"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../", "http://a/"),
            ("../../g", "http://a/g"),
            // § 5.4.2.
            ("../../../g", "http://a/g"),
            ("../../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            (".g", "http://a/b/c/.g"),
            ("g..", "http://a/b/c/g.."),
            ("..g", "http://a/b/c/..g"),
            ("./../g", "http://a/b/g"),
            ("./g/.", "http://a/b/c/g/"),
            ("g/./h", "http://a/b/c/g/h"),
            ("g/../h", "http://a/b/c/h"),
            ("g;x=1/./y", "http://a/b/c/g;x=1/y"),
            ("g;x=1/../y", "http://a/b/c/y"),
            ("g?y/./x", "http://a/b/c/g?y/./x"),
            ("g?y/../x", "http://a/b/c/g?y/../x"),
            ("g#s/./x", "http://a/b/c/g#s/./x"),
            ("g#s/../x", "http://a/b/c/g#s/../x"),
            ("http:g", "http:g"),
        ] {
            assert_eq!(resolve(base, r), want, "{r}");
        }
        // As written: no trailing slash, no percent-encoding.
        assert_eq!(resolve("http://x/", "http://www.example.org"), "http://www.example.org");
        assert_eq!(
            resolve("http://x/", "http://example.com/stéphane/#me"),
            "http://example.com/stéphane/#me"
        );
        assert_eq!(resolve("http://example.com", "a"), "http://example.com/a");
    }
}
