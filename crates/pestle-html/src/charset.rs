//! Which encoding a page is in, found as browsers find it, and its bytes as
//! UTF-8.
//!
//! In order: a byte order mark; the charset the server stated (the
//! `Content-Type` header, given by the caller); a `<meta charset>` or
//! `<meta http-equiv="Content-Type" content="…; charset=…">` in the first
//! 1024 bytes; an XML declaration's `encoding`. With none of these, UTF-8,
//! unless the bytes plainly are not (more broken sequences than whole ones),
//! then windows-1252, the web's legacy default. Labels are the WHATWG
//! Encoding Standard's (`latin1` and `iso-8859-1` are windows-1252,
//! `utf-16` in a meta tag is UTF-8, `x-sjis` is Shift_JIS).

use encoding_rs::{Decoder, Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};

/// How many bytes the meta prescan reads (as the HTML standard's).
pub const PRESCAN: usize = 1024;

/// How the encoding was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Bom,
    /// The caller's (the HTTP header's).
    Declared,
    Meta,
    Xml,
    /// None stated: UTF-8 or, when the bytes are not, windows-1252.
    Guessed,
}

/// The encoding of a page from its first bytes (at least [`PRESCAN`] of them, or all there are)
/// and the charset the server stated.
pub fn sniff(prefix: &[u8], declared: Option<&str>) -> (&'static Encoding, Source) {
    if let Some((enc, _)) = Encoding::for_bom(prefix) {
        return (enc, Source::Bom);
    }
    if let Some(enc) = declared.and_then(label) {
        return (enc, Source::Declared);
    }
    let head = &prefix[..prefix.len().min(PRESCAN)];
    if let Some(enc) = prescan(head) {
        return (enc, Source::Meta);
    }
    if let Some(enc) = xml_declaration(head) {
        return (enc, Source::Xml);
    }
    (guess(prefix), Source::Guessed)
}

/// An encoding by its label; a page cannot say it is UTF-16 in its own (ASCII) bytes, so that is UTF-8.
fn label(l: &str) -> Option<&'static Encoding> {
    let l = l.trim().trim_matches(['"', '\'']);
    let enc = Encoding::for_label(l.as_bytes())?;
    Some(if enc == UTF_16LE || enc == UTF_16BE { UTF_8 } else { enc })
}

/// UTF-8 unless the bytes have more broken sequences than whole multibyte ones.
fn guess(bytes: &[u8]) -> &'static Encoding {
    let sample = &bytes[..bytes.len().min(64 * 1024)];
    if simdutf8::basic::from_utf8(sample).is_ok() {
        return UTF_8;
    }
    let (mut whole, mut broken) = (0usize, 0usize);
    let mut i = 0;
    while i < sample.len() {
        let b = sample[i];
        if b < 0x80 {
            i += 1;
            continue;
        }
        let len = match b {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        };
        if len == 0 {
            broken += 1;
            i += 1;
            continue;
        }
        let end = i + len;
        if end > sample.len() {
            // Cut at the end of the sample: no evidence either way.
            break;
        }
        if std::str::from_utf8(&sample[i..end]).is_ok() {
            whole += 1;
            i = end;
        } else {
            broken += 1;
            i += 1;
        }
    }
    if broken > whole {
        WINDOWS_1252
    } else {
        UTF_8
    }
}

/// The HTML standard's "prescan a byte stream to determine its encoding" (§ 13.2.3.2): comments
/// and other tags (with their attribute values) are stepped over, a `<meta>`'s `charset`, or its
/// `content` when `http-equiv="content-type"` says so, decides.
fn prescan(head: &[u8]) -> Option<&'static Encoding> {
    let n = head.len();
    let mut i = 0;
    while i < n {
        let rest = &head[i..];
        if rest.starts_with(b"<!--") {
            // To the end of the comment (`-->`, which may share the `--` of its start).
            i = (i + 2..n.saturating_sub(2))
                .find(|&k| &head[k..k + 3] == b"-->")
                .map_or(n, |k| k + 3);
            continue;
        }
        let is_meta = rest.len() > 5
            && rest[..5].eq_ignore_ascii_case(b"<meta")
            && matches!(rest[5], b'\t' | b'\n' | 0x0c | b'\r' | b' ' | b'/');
        if is_meta {
            i += 5;
            let mut seen: Vec<Vec<u8>> = Vec::new();
            let (mut got_pragma, mut need_pragma, mut charset) = (false, None::<bool>, None);
            while let Some((name, value, next)) = get_attribute(head, i) {
                i = next;
                if seen.contains(&name) {
                    continue;
                }
                seen.push(name.clone());
                match name.as_slice() {
                    b"http-equiv" => {
                        if value.eq_ignore_ascii_case(b"content-type") {
                            got_pragma = true;
                        }
                    }
                    b"content" => {
                        if charset.is_none() {
                            if let Some(e) = charset_in_content(&String::from_utf8_lossy(&value)) {
                                charset = Some(e);
                                need_pragma = Some(true);
                            }
                        }
                    }
                    b"charset" => {
                        charset = Encoding::for_label(trim_ws(&value));
                        need_pragma = Some(false);
                    }
                    _ => {}
                }
            }
            let Some(need) = need_pragma else { continue };
            if need && !got_pragma {
                continue;
            }
            let Some(mut enc) = charset else { continue };
            if enc == UTF_16LE || enc == UTF_16BE {
                enc = UTF_8;
            }
            if enc.name() == "x-user-defined" {
                enc = WINDOWS_1252;
            }
            return Some(enc);
        }
        let is_tag = rest.len() > 2
            && rest[0] == b'<'
            && (rest[1].is_ascii_alphabetic() || (rest[1] == b'/' && rest[2].is_ascii_alphabetic()));
        if is_tag {
            // Past the tag name, then past its attributes (a `<meta` in a value is no meta).
            i += 1;
            while i < n && !matches!(head[i], b'\t' | b'\n' | 0x0c | b'\r' | b' ' | b'>') {
                i += 1;
            }
            while let Some((_, _, next)) = get_attribute(head, i) {
                i = next;
            }
            i += 1;
            continue;
        }
        if rest.starts_with(b"<!") || rest.starts_with(b"</") || rest.starts_with(b"<?") {
            i = head[i..].iter().position(|&c| c == b'>').map_or(n, |k| i + k + 1);
            continue;
        }
        i += 1;
    }
    None
}

fn trim_ws(v: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b'\t' | b'\n' | 0x0c | b'\r' | b' ');
    let start = v.iter().position(|c| !ws(c)).unwrap_or(v.len());
    let end = v.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &v[start..end]
}

/// The HTML standard's "get an attribute" (§ 13.2.3.2), from `i`: the attribute's name and value
/// (lowercase name, value as written with ASCII upper case lowered) and where to go on; none at
/// the tag's end.
fn get_attribute(head: &[u8], mut i: usize) -> Option<(Vec<u8>, Vec<u8>, usize)> {
    let n = head.len();
    let ws = |c: u8| matches!(c, b'\t' | b'\n' | 0x0c | b'\r' | b' ');
    while i < n && (ws(head[i]) || head[i] == b'/') {
        i += 1;
    }
    if i >= n || head[i] == b'>' {
        return None;
    }
    let mut name = Vec::new();
    let mut value = Vec::new();
    loop {
        if i >= n {
            return Some((name, value, n));
        }
        let c = head[i];
        if c == b'=' && !name.is_empty() {
            i += 1;
            break;
        }
        if ws(c) {
            while i < n && ws(head[i]) {
                i += 1;
            }
            if i < n && head[i] == b'=' {
                i += 1;
                break;
            }
            return Some((name, value, i));
        }
        if c == b'/' || c == b'>' {
            return Some((name, value, i));
        }
        name.push(c.to_ascii_lowercase());
        i += 1;
    }
    while i < n && ws(head[i]) {
        i += 1;
    }
    if i >= n {
        return Some((name, value, n));
    }
    let c = head[i];
    if c == b'"' || c == b'\'' {
        i += 1;
        while i < n && head[i] != c {
            value.push(head[i].to_ascii_lowercase());
            i += 1;
        }
        return Some((name, value, (i + 1).min(n)));
    }
    if c == b'>' {
        return Some((name, value, i));
    }
    while i < n && !ws(head[i]) && head[i] != b'>' {
        value.push(head[i].to_ascii_lowercase());
        i += 1;
    }
    Some((name, value, i))
}

/// The HTML standard's "extracting a character encoding from a meta element" (§ 2.5.6):
/// `text/html; charset=ISO-8859-1` → windows-1252.
pub fn charset_in_content(content: &str) -> Option<&'static Encoding> {
    let b = content.as_bytes();
    let mut from = 0;
    loop {
        let at = from + b[from..].windows(7).position(|w| w.eq_ignore_ascii_case(b"charset"))?;
        let mut i = at + 7;
        while i < b.len() && matches!(b[i], b'\t' | b'\n' | 0x0c | b'\r' | b' ') {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            from = i;
            continue;
        }
        i += 1;
        while i < b.len() && matches!(b[i], b'\t' | b'\n' | 0x0c | b'\r' | b' ') {
            i += 1;
        }
        let value = match b.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let end = b[i + 1..].iter().position(|&c| c == q)?;
                &b[i + 1..i + 1 + end]
            }
            Some(_) => {
                let end = b[i..]
                    .iter()
                    .position(|&c| matches!(c, b'\t' | b'\n' | 0x0c | b'\r' | b' ' | b';'))
                    .unwrap_or(b.len() - i);
                &b[i..i + end]
            }
            None => return None,
        };
        return Encoding::for_label(value);
    }
}

/// `<?xml version="1.0" encoding="iso-8859-1"?>`.
fn xml_declaration(head: &[u8]) -> Option<&'static Encoding> {
    let s = head.strip_prefix(b"<?xml")?;
    let end = s.windows(2).position(|w| w == b"?>")?;
    let attrs = attributes(&s[..end]);
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("encoding"))
        .and_then(|(_, v)| label(v))
}

/// `name="value"`, `name='value'`, `name=value` and bare names, from inside a tag.
fn attributes(tag: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = tag.len();
    while i < n {
        while i < n && (tag[i].is_ascii_whitespace() || tag[i] == b'/') {
            i += 1;
        }
        let k0 = i;
        while i < n && !tag[i].is_ascii_whitespace() && !matches!(tag[i], b'=' | b'/' | b'>') {
            i += 1;
        }
        if i == k0 {
            i += 1;
            continue;
        }
        let key = String::from_utf8_lossy(&tag[k0..i]).into_owned();
        while i < n && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < n && tag[i] == b'=' {
            i += 1;
            while i < n && tag[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < n && matches!(tag[i], b'"' | b'\'') {
                let q = tag[i];
                i += 1;
                let v0 = i;
                while i < n && tag[i] != q {
                    i += 1;
                }
                value = String::from_utf8_lossy(&tag[v0..i]).into_owned();
                i += 1;
            } else {
                let v0 = i;
                while i < n && !tag[i].is_ascii_whitespace() && tag[i] != b'>' {
                    i += 1;
                }
                value = String::from_utf8_lossy(&tag[v0..i]).into_owned();
            }
        }
        out.push((key, value));
    }
    out
}

/// Whether the first bytes already decide the encoding, so the page streams on without waiting
/// for [`PRESCAN`] bytes: a byte order mark, or a charset in a `<meta>`. (A page that states none
/// waits, as in browsers: its first kilobyte decides whether it is UTF-8.)
fn settled(prefix: &[u8]) -> bool {
    Encoding::for_bom(prefix).is_some() || prescan(prefix).is_some()
}

/// A page's bytes as UTF-8, chunk by chunk: passed through when it is UTF-8 already.
pub(crate) struct Transcoder {
    /// What arrived before the encoding was decided.
    pending: Option<Vec<u8>>,
    declared: Option<String>,
    decoder: Option<Decoder>,
    pub(crate) encoding: &'static Encoding,
    pub(crate) source: Source,
    /// A UTF-8 byte order mark still to drop.
    skip_bom: bool,
}

impl Transcoder {
    pub(crate) fn new(declared: Option<&str>) -> Transcoder {
        Transcoder {
            pending: Some(Vec::new()),
            declared: declared.map(str::to_owned),
            decoder: None,
            encoding: UTF_8,
            source: Source::Guessed,
            skip_bom: false,
        }
    }

    fn decide(&mut self, prefix: &[u8]) {
        let (enc, source) = sniff(prefix, self.declared.as_deref());
        self.encoding = enc;
        self.source = source;
        if enc == UTF_8 {
            self.skip_bom = prefix.starts_with(b"\xef\xbb\xbf");
        } else {
            self.decoder = Some(enc.new_decoder_with_bom_removal());
        }
    }

    /// The UTF-8 for `chunk`, through `out` (called with nothing until the encoding is known).
    pub(crate) fn write(&mut self, chunk: &[u8], out: &mut dyn FnMut(&[u8])) {
        if let Some(p) = &mut self.pending {
            p.extend_from_slice(chunk);
            // A charset the server stated decides at once (but for a byte order mark: three bytes).
            let declared = self.declared.is_some() && p.len() >= 3;
            if p.len() < PRESCAN && !declared && !settled(p) {
                return;
            }
            let buffered = self.pending.take().unwrap_or_default();
            self.decide(&buffered);
            self.pass(&buffered, false, out);
            return;
        }
        self.pass(chunk, false, out);
    }

    /// Text already decoded (a JavaScript string): the page is UTF-8, whatever it says.
    pub(crate) fn text(&mut self, text: &str, out: &mut dyn FnMut(&[u8])) {
        if let Some(buffered) = self.pending.take() {
            self.encoding = UTF_8;
            self.source = Source::Declared;
            // Bytes before it (unusual: a page fed both ways) as their sniffed encoding would decide.
            if !buffered.is_empty() {
                let (enc, _) = sniff(&buffered, self.declared.as_deref());
                let (t, _) = enc.decode_with_bom_removal(&buffered);
                out(t.as_bytes());
            }
        }
        if !text.is_empty() {
            out(text.as_bytes());
        }
    }

    /// The end of the page.
    pub(crate) fn end(&mut self, out: &mut dyn FnMut(&[u8])) {
        if let Some(buffered) = self.pending.take() {
            self.decide(&buffered);
            self.pass(&buffered, true, out);
            return;
        }
        self.pass(&[], true, out);
    }

    fn pass(&mut self, mut bytes: &[u8], last: bool, out: &mut dyn FnMut(&[u8])) {
        if self.skip_bom && bytes.len() >= 3 {
            bytes = &bytes[3..];
            self.skip_bom = false;
        }
        match &mut self.decoder {
            None => {
                if !bytes.is_empty() {
                    out(bytes);
                }
            }
            Some(d) => {
                let mut s = String::with_capacity(d.max_utf8_buffer_length(bytes.len()).unwrap_or(bytes.len() * 3));
                let (_, _, _) = d.decode_to_string(bytes, &mut s, last);
                if !s.is_empty() {
                    out(s.as_bytes());
                }
            }
        }
    }
}

/// A whole page as UTF-8: borrowed when it is UTF-8 already.
pub(crate) fn whole<'b>(
    bytes: &'b [u8],
    declared: Option<&str>,
) -> (std::borrow::Cow<'b, [u8]>, &'static Encoding, Source) {
    let (enc, source) = sniff(bytes, declared);
    if enc == UTF_8 {
        let body = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        return (std::borrow::Cow::Borrowed(body), enc, source);
    }
    let (text, _) = enc.decode_with_bom_removal(bytes);
    (std::borrow::Cow::Owned(text.into_owned().into_bytes()), enc, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(prefix: &[u8], declared: Option<&str>) -> (&'static str, Source) {
        let (e, s) = sniff(prefix, declared);
        (e.name(), s)
    }

    #[test]
    fn found_as_browsers_find_it() {
        assert_eq!(name(b"\xef\xbb\xbf<html>", Some("latin1")), ("UTF-8", Source::Bom));
        assert_eq!(name(b"\xff\xfe<\x00", None), ("UTF-16LE", Source::Bom));
        assert_eq!(name(b"<html>", Some("ISO-8859-1")), ("windows-1252", Source::Declared));
        assert_eq!(
            name(b"<html><meta charset=\"Shift_JIS\">", None),
            ("Shift_JIS", Source::Meta)
        );
        assert_eq!(name(b"<html><META CHARSET=euc-kr>", None), ("EUC-KR", Source::Meta));
        assert_eq!(
            name(
                b"<meta http-equiv='Content-Type' content='text/html; charset=windows-1251'>",
                None
            ),
            ("windows-1251", Source::Meta)
        );
        assert_eq!(
            name(
                b"<meta content=\"text/html;charset=gb2312\" http-equiv=\"content-type\">",
                None
            ),
            ("GBK", Source::Meta)
        );
        assert_eq!(name(b"<meta charset=\"utf-16\">", None), ("UTF-8", Source::Meta));
        assert_eq!(
            name(b"<?xml version=\"1.0\" encoding=\"ISO-8859-2\"?><html>", None),
            ("ISO-8859-2", Source::Xml)
        );
        assert_eq!(
            name(b"<meta charset=\"nonsense\"><p>caf\xc3\xa9</p>", None),
            ("UTF-8", Source::Guessed)
        );
        assert_eq!(
            name(b"<p>caf\xe9 cr\xe8me br\xfbl\xe9e</p>", None),
            ("windows-1252", Source::Guessed)
        );
        // A stray bad byte in a UTF-8 page leaves it UTF-8.
        let mostly = ["<p>caf\u{e9} cr\u{e8}me \u{2014} \u{20ac}5</p>".as_bytes(), b"\xff"].concat();
        assert_eq!(name(&mostly, None), ("UTF-8", Source::Guessed));
        // A meta past the first 1024 bytes is not read.
        let late = [b"<p>".repeat(400).as_slice(), b"<meta charset=\"koi8-r\">"].concat();
        assert_eq!(name(&late, None), ("UTF-8", Source::Guessed));
    }

    /// The HTML standard's prescan: comments, attribute values and pragma rules.
    #[test]
    fn the_prescan_algorithm() {
        let e = |b: &[u8]| prescan(b).map(|e| e.name());
        assert_eq!(
            e(b"<!-- <meta charset=koi8-r> --><meta charset=iso-8859-2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(
            e(b"<div title='<meta charset=koi8-r>'><meta charset=shift_jis>"),
            Some("Shift_JIS")
        );
        assert_eq!(
            e(b"<meta content='text/html; charset=koi8-r'>"),
            None,
            "content without http-equiv"
        );
        assert_eq!(
            e(b"<meta content='text/html; charset=koi8-r' http-equiv=Content-Type>"),
            Some("KOI8-R")
        );
        assert_eq!(
            e(b"<meta charset=koi8-r charset=utf-8>"),
            Some("KOI8-R"),
            "the first of a repeated attribute"
        );
        assert_eq!(e(b"<meta charset=x-user-defined>"), Some("windows-1252"));
        assert_eq!(e(b"<meta charset=utf-16le>"), Some("UTF-8"));
        assert_eq!(e(b"<metacharset=koi8-r>"), None, "not a meta tag");
        assert_eq!(e(b"<meta/charset=koi8-r>"), Some("KOI8-R"));
        assert_eq!(
            e(b"<meta charset=bogus><meta charset=gbk>"),
            Some("GBK"),
            "an unknown label: go on"
        );
        assert_eq!(
            e(b"<?xml-stylesheet?><!DOCTYPE html></x><meta charset=euc-jp>"),
            Some("EUC-JP")
        );
        assert_eq!(
            charset_in_content("text/html; charset =\"gb2312\"").map(|e| e.name()),
            Some("GBK")
        );
        assert_eq!(
            charset_in_content("charsetx; charset=windows-1250").map(|e| e.name()),
            Some("windows-1250")
        );
        assert_eq!(charset_in_content("text/html; charset='unterminated"), None);
    }

    #[test]
    fn streams_as_it_decodes() {
        let page = "<meta charset=\"windows-1252\"><p>café — 5€</p>".to_owned() + &"x".repeat(3000);
        let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(&page);
        for size in [1, 7, 1023, 1024, 5000] {
            let mut t = Transcoder::new(None);
            let mut got = Vec::new();
            for c in bytes.chunks(size) {
                t.write(c, &mut |b| got.extend_from_slice(b));
            }
            t.end(&mut |b| got.extend_from_slice(b));
            assert_eq!(String::from_utf8(got).unwrap(), page, "chunks of {size}");
            assert_eq!(t.encoding.name(), "windows-1252");
        }
        // UTF-8 passes through, without its byte order mark.
        let mut t = Transcoder::new(None);
        let mut got = Vec::new();
        t.write(b"\xef\xbb\xbf<p>ok</p>", &mut |b| got.extend_from_slice(b));
        t.end(&mut |b| got.extend_from_slice(b));
        assert_eq!(got, b"<p>ok</p>");
    }
}
