//! A host's public suffix and registrable domain, by the Public Suffix List
//! (<https://publicsuffix.org>), bundled: its ICANN and private sections
//! both, as browsers read them (`foo.github.io` is a site of its own). The
//! list's date is [`list_version`]; `scripts/update-psl.sh` takes a newer
//! one.
//!
//! A host is read as a URL's host is: case folded, an IDN by its punycode
//! (what both functions return), one trailing dot dropped. An IP address
//! is its own registrable domain and has no public suffix; an empty label
//! (`.com`, `a..b`) is no host.
//!
//! ```
//! use pestle_html::psl::{public_suffix, registrable_domain};
//!
//! assert_eq!(registrable_domain("tickets.example.com.mx").as_deref(), Some("example.com.mx"));
//! assert_eq!(registrable_domain("foo.github.io").as_deref(), Some("foo.github.io"));
//! assert_eq!(public_suffix("www.example.co.uk").as_deref(), Some("co.uk"));
//! // A host that is a public suffix is its own registrable domain.
//! assert_eq!(registrable_domain("co.uk").as_deref(), Some("co.uk"));
//! ```

use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

/// The list, as publicsuffix.org serves it (MPL 2.0).
const LIST: &str = include_str!("../data/public_suffix_list.dat");

struct Rules {
    /// `co.uk`.
    plain: HashSet<String>,
    /// `*.ck`, by what the `*` is under (`ck`).
    wildcard: HashSet<String>,
    /// `!www.ck`, without the `!`.
    exception: HashSet<String>,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut r = Rules {
            plain: HashSet::new(),
            wildcard: HashSet::new(),
            exception: HashSet::new(),
        };
        for line in LIST.lines() {
            let Some(rule) = line.split_whitespace().next().filter(|l| !l.starts_with("//")) else {
                continue;
            };
            let (set, name) = if let Some(n) = rule.strip_prefix('!') {
                (&mut r.exception, n)
            } else if let Some(n) = rule.strip_prefix("*.") {
                (&mut r.wildcard, n)
            } else {
                (&mut r.plain, rule)
            };
            if let Ok(ascii) = idna::domain_to_ascii(name) {
                set.insert(ascii);
            }
        }
        r
    })
}

/// The date of the bundled list (`2026-10-22_08-36-41_UTC`).
pub fn list_version() -> &'static str {
    LIST.lines()
        .find_map(|l| l.strip_prefix("// VERSION: "))
        .map_or("", str::trim)
}

enum Host {
    Ip(String),
    Name(String),
}

fn host(host: &str) -> Option<Host> {
    let h = host.trim();
    let h = h.strip_suffix('.').unwrap_or(h);
    if h.parse::<Ipv4Addr>().is_ok() {
        return Some(Host::Ip(h.to_owned()));
    }
    let bare = h.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(h);
    if let Ok(ip) = bare.parse::<Ipv6Addr>() {
        return Some(Host::Ip(format!("[{ip}]")));
    }
    if h.is_empty() || h.split('.').any(str::is_empty) {
        return None;
    }
    let ascii = idna::domain_to_ascii(h).ok()?;
    if ascii.is_empty() || ascii.split('.').any(str::is_empty) {
        return None;
    }
    Some(Host::Name(ascii))
}

/// The public suffix of a name (ASCII, lower case): the part under which anyone can register one.
fn suffix(name: &str) -> &str {
    let r = rules();
    let starts = std::iter::once(0).chain(name.match_indices('.').map(|(i, _)| i + 1));
    // An exception prevails over every other rule: its suffix is the rule less its first label.
    for at in starts.clone() {
        let candidate = &name[at..];
        if r.exception.contains(candidate) {
            return candidate.split_once('.').map_or(candidate, |(_, rest)| rest);
        }
    }
    // Else the longest rule that matches; else the last label (the rule `*`).
    for at in starts {
        let candidate = &name[at..];
        if r.plain.contains(candidate) {
            return candidate;
        }
        if candidate
            .split_once('.')
            .is_some_and(|(_, under)| r.wildcard.contains(under))
        {
            return candidate;
        }
    }
    name.rsplit('.').next().unwrap_or(name)
}

/// A host's public suffix (`co.uk` of `www.example.co.uk`); none for an IP address or no host.
pub fn public_suffix(host_name: &str) -> Option<String> {
    match host(host_name)? {
        Host::Ip(_) => None,
        Host::Name(name) => Some(suffix(&name).to_owned()),
    }
}

/// A host's registrable domain: its public suffix and the label before it (`example.co.uk` of
/// `www.example.co.uk`). A host that is a public suffix (`co.uk`), or an IP address, is its own;
/// none for no host.
pub fn registrable_domain(host_name: &str) -> Option<String> {
    match host(host_name)? {
        Host::Ip(ip) => Some(ip),
        Host::Name(name) => {
            let s = suffix(&name).len();
            if s == name.len() {
                return Some(name);
            }
            let head = &name[..name.len() - s - 1];
            let label = head.rsplit('.').next().unwrap_or(head);
            Some(name[name.len() - s - 1 - label.len()..].to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_and_domains() {
        assert_eq!(
            registrable_domain("tickets.example.com.mx").as_deref(),
            Some("example.com.mx")
        );
        assert_eq!(registrable_domain("foo.github.io").as_deref(), Some("foo.github.io"));
        assert_eq!(
            registrable_domain("a.b.foo.github.io").as_deref(),
            Some("foo.github.io")
        );
        assert_eq!(registrable_domain("github.io").as_deref(), Some("github.io"));
        assert_eq!(registrable_domain("co.uk").as_deref(), Some("co.uk"));
        assert_eq!(public_suffix("co.uk").as_deref(), Some("co.uk"));
        assert_eq!(registrable_domain("WWW.Example.COM.").as_deref(), Some("example.com"));
        // Wildcards and their exceptions.
        assert_eq!(public_suffix("a.b.ck").as_deref(), Some("b.ck"));
        assert_eq!(registrable_domain("a.www.ck").as_deref(), Some("www.ck"));
        // IDNs by their punycode, either way they come.
        assert_eq!(
            registrable_domain("www.食狮.公司.cn").as_deref(),
            Some("xn--85x722f.xn--55qx5d.cn")
        );
        assert_eq!(
            registrable_domain("www.xn--85x722f.xn--55qx5d.cn").as_deref(),
            Some("xn--85x722f.xn--55qx5d.cn")
        );
        // Addresses, and what is no host.
        assert_eq!(registrable_domain("192.168.0.1").as_deref(), Some("192.168.0.1"));
        assert_eq!(registrable_domain("[::1]").as_deref(), Some("[::1]"));
        assert_eq!(registrable_domain("::1").as_deref(), Some("[::1]"));
        assert_eq!(public_suffix("192.168.0.1"), None);
        for bad in ["", ".", ".com", "a..b", " "] {
            assert_eq!(registrable_domain(bad), None, "{bad:?}");
        }
        assert!(list_version().starts_with("20"), "{}", list_version());
    }
}
