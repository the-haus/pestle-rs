//! robots.txt (RFC 9309), read for one product token: the groups whose
//! `User-agent` names it (its product token, `[A-Za-z_-]+`, compared without
//! case: `FestimapBot/2.1` names `festimapbot`), else the `*` groups, merged; the longest matching rule wins, `Allow`
//! on a tie; `*` and a closing `$` as the RFC has them; paths compared with
//! their percent-encoding made one (`%7E` is `~`, `ü` is `%C3%BC`).
//! `Crawl-delay` (no RFC, but widely stated) is the largest of the chosen
//! groups'. `Sitemap:` lines belong to no group: every one is kept.
//!
//! What sites get wrong is read as they mean it, as Google reads it:
//! misspelled fields (`Dissallow`, `User agent`, `useragent`), a byte order
//! mark, any line ending, comments anywhere, `Disallow` before any
//! `User-agent` (ignored), a file larger than 500 KiB (the rest ignored).
//!
//! ```
//! use pestle_html::robots::Robots;
//! let r = Robots::parse("User-agent: *\nDisallow: /private\nAllow: /private/open$\n", "festimapbot");
//! assert!(r.allows("https://venue.example/agenda"));
//! assert!(!r.allows("/private/x"));
//! assert!(r.allows("/private/open"));
//! ```

use serde::Serialize;
use url::Url;

/// How much of a file is read (RFC 9309: at least 500 KiB).
pub const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub allow: bool,
    /// The path pattern, its percent-encoding normalised.
    pub pattern: String,
}

/// The rules for one product token.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Robots {
    pub rules: Vec<Rule>,
    /// Seconds, when a chosen group states one.
    #[serde(rename = "crawlDelay", skip_serializing_if = "Option::is_none")]
    pub crawl_delay: Option<f64>,
    /// Every `Sitemap:` URL of the file (absolute http(s), in file order, once each).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sitemaps: Vec<String>,
}

#[derive(Default)]
struct Group {
    agents: Vec<String>,
    rules: Vec<Rule>,
    delay: Option<f64>,
}

enum Field {
    Agent,
    Allow,
    Disallow,
    Delay,
    Sitemap,
    Other,
}

fn field(name: &str) -> Field {
    let n: String = name
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    match n.as_str() {
        "useragent" | "user" | "useragents" => Field::Agent,
        "allow" => Field::Allow,
        // As Google accepts them.
        "disallow" | "dissallow" | "dissalow" | "disalow" | "diasllow" | "disallaw" | "disallows" => Field::Disallow,
        "crawldelay" => Field::Delay,
        "sitemap" | "sitemaps" | "sitemapurl" => Field::Sitemap,
        _ => Field::Other,
    }
}

impl Robots {
    /// Allows everything: a robots.txt that is absent (4xx).
    pub fn allow_all() -> Robots {
        Robots::default()
    }

    /// Allows nothing: a robots.txt that could not be read (5xx, no answer).
    pub fn disallow_all() -> Robots {
        Robots {
            rules: vec![Rule {
                allow: false,
                pattern: "/".into(),
            }],
            ..Robots::default()
        }
    }

    /// The rules of the groups for `token` (else `*`) in `text`.
    pub fn parse(text: &str, token: &str) -> Robots {
        let text = crate::text::cut_bytes(text, MAX_BYTES);
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let token = product_token(token);
        let mut groups: Vec<Group> = Vec::new();
        let mut sitemaps: Vec<String> = Vec::new();
        let mut in_rules = false;
        for raw in text.split(['\n', '\r']) {
            let line = raw.split('#').next().unwrap_or("").trim();
            // `Disallow /x` (no colon), as some write it: the first space separates.
            let Some(sep) = line.find(':').or_else(|| line.find(char::is_whitespace)) else {
                continue;
            };
            let (name, value) = (&line[..sep], line[sep + 1..].trim());
            match field(name) {
                Field::Sitemap => {
                    if let Ok(u) = Url::parse(value) {
                        if matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.trim().is_empty())
                        {
                            let u = u.to_string();
                            if !sitemaps.contains(&u) {
                                sitemaps.push(u);
                            }
                        }
                    }
                }
                Field::Agent => open_group(&mut groups, &mut in_rules, value.to_ascii_lowercase()),
                Field::Allow | Field::Disallow => {
                    let Some(g) = groups.last_mut() else { continue };
                    in_rules = true;
                    // An empty `Disallow` allows everything: no rule. A pattern starts with `/` or `*`.
                    if value.is_empty() {
                        continue;
                    }
                    let pattern = if value.starts_with(['/', '*']) {
                        value.to_owned()
                    } else {
                        format!("/{value}")
                    };
                    g.rules.push(Rule {
                        allow: matches!(field(name), Field::Allow),
                        pattern: normalise(&pattern).into_owned(),
                    });
                }
                Field::Delay => {
                    let Some(g) = groups.last_mut() else { continue };
                    in_rules = true;
                    if let Ok(d) = value.parse::<f64>() {
                        if d.is_finite() && d >= 0.0 {
                            g.delay = Some(d);
                        }
                    }
                }
                Field::Other => {}
            }
        }
        // The groups that name the token (by its start: `festimap` names `festimapbot`), else `*`.
        let names = |a: &str| {
            let a = product_token(a);
            !a.is_empty() && a == token
        };
        let named: Vec<&Group> = groups.iter().filter(|g| g.agents.iter().any(|a| names(a))).collect();
        let chosen: Vec<&Group> = if named.is_empty() {
            groups
                .iter()
                .filter(|g| g.agents.iter().any(|a| product_token(a) == "*"))
                .collect()
        } else {
            named
        };
        let rules: Vec<Rule> = chosen.iter().flat_map(|g| g.rules.iter().cloned()).collect();
        let crawl_delay = chosen
            .iter()
            .filter_map(|g| g.delay)
            .fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d))));
        Robots {
            rules,
            crawl_delay,
            sitemaps,
        }
    }

    /// Whether `url` (a URL, or a path with its query) may be fetched.
    pub fn allows(&self, url: &str) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let path: std::borrow::Cow<'_, str> = match simple_path(url) {
            Some(p) => std::borrow::Cow::Borrowed(p),
            None => match Url::parse(url) {
                Ok(u) => {
                    let mut p = u.path().to_owned();
                    if let Some(q) = u.query() {
                        p.push('?');
                        p.push_str(q);
                    }
                    std::borrow::Cow::Owned(p)
                }
                Err(_) => std::borrow::Cow::Borrowed(url),
            },
        };
        // RFC 9309 § 2.2.2: the robots.txt itself is always allowed.
        if path == "/robots.txt" {
            return true;
        }
        let path = normalise_uri(&path);
        let mut best: Option<&Rule> = None;
        for rule in &self.rules {
            if !matches(&rule.pattern, &path) {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => {
                    rule.pattern.len() > b.pattern.len()
                        || (rule.pattern.len() == b.pattern.len() && rule.allow && !b.allow)
                }
            };
            if better {
                best = Some(rule);
            }
        }
        best.is_none_or(|r| r.allow)
    }
}

/// The path and query of a URL that needs no parsing to find them: a path (`/a?b`), or
/// `http(s)://host/…` of plain characters, without a fragment, dot segments or escapes.
/// A product token as RFC 9309 has it: the leading `[A-Za-z_-]` of a user-agent, lowercase
/// (`FestimapBot/2.1 (+https://…)` → `festimapbot`); `*` stays `*`.
fn product_token(agent: &str) -> String {
    let a = agent.trim();
    if a.starts_with('*') {
        return "*".into();
    }
    a.chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_' || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A URI path made comparable with patterns: as [`normalise`], and a literal `*` or `$` (which a
/// pattern can only mean literally as `%2A`, `%24`) escaped (RFC 9309 § 2.2.3).
fn normalise_uri(path: &str) -> std::borrow::Cow<'_, str> {
    let n = normalise(path);
    if n.contains(['*', '$']) {
        std::borrow::Cow::Owned(n.replace('*', "%2A").replace('$', "%24"))
    } else {
        n
    }
}

fn simple_path(url: &str) -> Option<&str> {
    let tail = if url.starts_with('/') {
        url
    } else {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        &rest[rest.find('/')?..]
    };
    let plain = tail.bytes().all(|c| {
        c.is_ascii_graphic()
            && !matches!(
                c,
                b'#' | b'%' | b'\\' | b'"' | b'<' | b'>' | b'`' | b'{' | b'}' | b'|' | b'^' | b'\''
            )
    }) && !tail.contains("/.")
        && !tail.starts_with("//");
    plain.then_some(tail)
}

fn open_group(groups: &mut Vec<Group>, in_rules: &mut bool, agent: String) {
    // Consecutive `User-agent` lines open one group.
    if groups.is_empty() || *in_rules {
        groups.push(Group::default());
        *in_rules = false;
    }
    if let Some(g) = groups.last_mut() {
        g.agents.push(agent);
    }
}

/// Percent-encoding made one: unreserved characters decoded (`%7E` → `~`), other escapes in
/// upper case (`%2f` → `%2F`), bytes beyond ASCII and spaces encoded (`ü` → `%C3%BC`).
fn normalise(path: &str) -> std::borrow::Cow<'_, str> {
    if path.bytes().all(|c| c > 0x20 && c < 0x80 && c != b'%') {
        return std::borrow::Cow::Borrowed(path);
    }
    std::borrow::Cow::Owned(normalise_escapes(path))
}

fn normalise_escapes(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' && i + 2 < b.len() + 1 {
            let hex = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                if v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.' | b'_' | b'~') {
                    out.push(v as char);
                } else {
                    out.push_str(&format!("%{v:02X}"));
                }
                i += 3;
                continue;
            }
        }
        if c >= 0x80 || c == b' ' || c < 0x20 {
            out.push_str(&format!("%{c:02X}"));
        } else {
            out.push(c as char);
        }
        i += 1;
    }
    out
}

/// Whether a pattern (with `*` and a closing `$`) matches a path from its start: a linear
/// wildcard match (no regex, no backtracking blow-up).
fn matches(pattern: &str, path: &str) -> bool {
    let (body, anchored) = match pattern.strip_suffix('$') {
        Some(b) => (b, true),
        None => (pattern, false),
    };
    let p = body.as_bytes();
    let s = path.as_bytes();
    if !anchored && !body.contains('*') {
        return s.starts_with(p);
    }
    // Greedy match with the last `*` to retry from.
    let (mut i, mut j) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    loop {
        if i < p.len() && p[i] == b'*' {
            star = Some((i, j));
            i += 1;
            continue;
        }
        if i == p.len() {
            if !anchored || j == s.len() {
                return true;
            }
        } else if j < s.len() && p[i] == s[j] {
            i += 1;
            j += 1;
            continue;
        }
        match star {
            Some((si, sj)) if sj < s.len() => {
                star = Some((si, sj + 1));
                i = si + 1;
                j = sj + 1;
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_512_kib_are_read() {
        // 512 KiB of comments in a two-byte character, then a rule: past the limit in bytes.
        let pad = format!("# {}\n", "é".repeat(1000));
        let filler = pad.repeat(MAX_BYTES / pad.len() + 1);
        let r = Robots::parse(&format!("User-agent: *\n{filler}Disallow: /late\n"), "bot");
        assert!(r.allows("/late"));
        let r = Robots::parse(&format!("User-agent: *\nDisallow: /early\n{filler}"), "bot");
        assert!(!r.allows("/early"));
    }

    #[test]
    fn groups_and_precedence() {
        let txt = "User-agent: *\nDisallow: /\n\nUser-agent: Googlebot\nUser-agent: festimap\nDisallow: /tickets\nAllow: /tickets/info\nCrawl-delay: 2\n\nUser-agent: festimapbot\nCrawl-delay: 5\nDisallow: /*.pdf$\n";
        let r = Robots::parse(txt, "festimapbot");
        assert_eq!(r.crawl_delay, Some(5.0));
        assert!(r.allows("/agenda"), "the * group is not ours");
        assert!(r.allows("/tickets/1"), "`festimap` is another product token, not ours");
        assert!(!r.allows("/files/a.pdf"));
        assert!(r.allows("/files/a.pdf?x=1"), "$ anchors the end");
        // Two groups naming one token are merged; the longer rule wins.
        let m = Robots::parse(txt, "festimap");
        assert_eq!(m.crawl_delay, Some(2.0));
        assert!(!m.allows("/tickets/1") && m.allows("/tickets/info/x"));
        let other = Robots::parse(txt, "otherbot");
        assert!(!other.allows("/agenda"));
    }

    #[test]
    fn ties_wildcards_and_encoding() {
        let r = Robots::parse("user-agent: *\ndisallow: /page\nallow: /page\n", "x");
        assert!(r.allows("/page"), "Allow wins a tie");
        let r = Robots::parse(
            "User-agent: *\nDisallow: /*/private/*.html\nDisallow: /%7Euser\nDisallow: /caf%c3%a9\nDisallow: /a b\n",
            "x",
        );
        assert!(!r.allows("/x/private/y/z.html"));
        assert!(r.allows("/x/public/z.html"));
        assert!(!r.allows("/~user/x"));
        assert!(!r.allows("https://v.example/café/menu"));
        assert!(!r.allows("/a%20b"));
        // `*` everywhere does not blow up.
        let r = Robots::parse(&format!("User-agent: *\nDisallow: /{}x$\n", "*a".repeat(200)), "x");
        assert!(r.allows(&format!("/{}", "a".repeat(10_000))));
    }

    /// RFC 9309 § 5.1 and § 5.2.
    #[test]
    fn the_rfc_examples() {
        let txt = "User-Agent: *\nDisallow: *.gif$\nDisallow: /example/\nAllow: /publications/\n\nUser-Agent: foobot\nDisallow:/\nAllow:/example/page.html\nAllow:/example/allowed.gif\n\nUser-Agent: barbot\nUser-Agent: bazbot\nDisallow: /example/page.html\n\nUser-Agent: quxbot\n\nEOF";
        let foo = Robots::parse(txt, "foobot");
        assert!(foo.allows("/example/page.html") && foo.allows("/example/allowed.gif"));
        assert!(!foo.allows("/x") && !foo.allows("/example/other.gif"));
        for bot in ["barbot", "BazBot/1.0"] {
            let b = Robots::parse(txt, bot);
            assert!(!b.allows("/example/page.html"), "{bot}");
            assert!(b.allows("/example/other.html") && b.allows("/a.gif"), "{bot}");
        }
        let qux = Robots::parse(txt, "quxbot");
        assert!(
            qux.allows("/example/page.html") && qux.allows("/a.gif"),
            "an empty group allows all"
        );
        let other = Robots::parse(txt, "otherbot");
        assert!(
            !other.allows("/a.gif")
                && other.allows("/a.gif?x")
                && !other.allows("/example/x")
                && other.allows("/publications/x")
        );
        let longest = Robots::parse(
            "User-Agent: foobot\nAllow: /example/page/\nDisallow: /example/page/disallowed.gif\n",
            "foobot",
        );
        assert!(longest.allows("/example/page/") && longest.allows("/example/page/ok.html"));
        assert!(!longest.allows("/example/page/disallowed.gif"));
        // The product token is a whole token: `foo` does not name `foobot`.
        assert!(Robots::parse("User-agent: foo\nDisallow: /\n", "foobot").allows("/x"));
        // robots.txt itself is always allowed.
        assert!(Robots::parse("User-agent: *\nDisallow: /\n", "x").allows("https://v.example/robots.txt"));
    }

    /// RFC 9309 § 2.2.2 (percent-encoding) and § 2.2.3 (special characters).
    #[test]
    fn the_rfc_tables() {
        for (pattern, uri) in [
            ("/foo/bar?baz=quz", "/foo/bar?baz=quz"),
            ("/foo/bar/ツ", "https://www.example.com/foo/bar/%E3%83%84"),
            ("/foo/bar/%E3%83%84", "https://www.example.com/foo/bar/%E3%83%84"),
            ("/foo/bar/%62%61%7A", "https://www.example.com/foo/bar/baz"),
            (
                "/path/file-with-a-%2A.html",
                "https://www.example.com/path/file-with-a-*.html",
            ),
            ("/path/foo-%24", "https://www.example.com/path/foo-$"),
        ] {
            let r = Robots::parse(&format!("User-agent: *\nDisallow: {pattern}\n"), "x");
            assert!(!r.allows(uri), "{pattern} should match {uri}");
        }
        // `*` in a URI is literal: `/a*` (a wildcard) matches it, `/a%2A` only it.
        let r = Robots::parse("User-agent: *\nDisallow: /a%2A\n", "x");
        assert!(r.allows("/abc") && !r.allows("/a*"));
        // Paths are case-sensitive; fields and agents are not.
        let r = Robots::parse("USER-AGENT: X\nDISALLOW: /Private\n", "x");
        assert!(!r.allows("/Private/1") && r.allows("/private/1"));
    }

    #[test]
    fn what_sites_get_wrong() {
        let txt = "\u{feff}Disallow: /zz-before-any-group\r\nUser agent: *\r\nDissallow: /a\r\nuseragent: festimapbot\r\nDisallow: b\rSitemap: https://v.example/sitemap.xml # a comment\nSITEMAP:https://v.example/sitemap.xml\nSitemap: http:// /broken.xml\nSitemap: /relative.xml\n";
        let r = Robots::parse(txt, "festimapbot");
        assert_eq!(r.sitemaps, ["https://v.example/sitemap.xml"]);
        assert!(
            r.allows("/a/1"),
            "after a rule, a User-agent opens a new group: /a is the * group's"
        );
        assert!(!Robots::parse(txt, "otherbot").allows("/a/1"), "a misspelled Disallow");
        assert!(!r.allows("/b"), "a pattern without its slash");
        assert!(r.allows("/zz-before-any-group"));
        // Empty Disallow allows all; nothing at all allows all.
        assert!(Robots::parse("User-agent: *\nDisallow:\n", "x").allows("/x"));
        assert!(Robots::parse("", "x").allows("/x"));
        assert!(!Robots::disallow_all().allows("/x"));
        assert!(Robots::allow_all().allows("/x"));
    }
}
