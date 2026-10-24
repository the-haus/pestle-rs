//! A streaming HTML tokenizer with htmlparser2's rules (version 12, HTML
//! mode): fed chunks of any size, it hands over every token it has whole,
//! keeping only an unfinished one (a tag cut by the chunk's end, a trailing
//! `&` that may start a reference, half a UTF-8 sequence) for the next
//! chunk. `<script>`, `<style>`, `<xmp>`, `<iframe>`, `<noembed>` and
//! `<noframes>` hold raw text, `<title>` and `<textarea>` text with
//! references, `<plaintext>` text to the end; a start tag cut by the end of
//! the input is dropped, as htmlparser2 drops it.

use crate::decode::{decode_into_valid, utf8_valid, LOOKAHEAD};
use smallvec::SmallVec;

use crate::tree::Element;

/// What the tokenizer finds.
pub(crate) trait TokenSink {
    /// A start tag: the element (name lower-cased, attributes' names
    /// lower-cased and values decoded, the first of a name kept), and its raw markup.
    fn start_tag(&mut self, el: Element, self_closing: bool, raw: &[u8]);
    fn end_tag(&mut self, name: &str, raw: &[u8]);
    /// Text as it is in the markup, whether its references are to be decoded (not in raw
    /// text), and whether it is known to be valid UTF-8.
    fn text(&mut self, raw: &[u8], references: bool, valid: bool);
    /// A comment, a doctype or another declaration, raw.
    fn comment(&mut self, raw: &[u8]);
    /// Inside `<svg>` or `<math>`: no raw-text elements there.
    fn in_foreign(&self) -> bool;
    /// An empty buffer for the next element (one a closed element left, when there is one).
    fn buffer(&mut self) -> String {
        String::new()
    }
    /// The bytes handed over so far are about to go away: copy what still points into them.
    fn chunk_end(&mut self) {}
}

#[derive(Clone, Copy)]
enum Mode {
    Data,
    /// Inside a raw-text element until `</name`; RCDATA decodes references.
    Raw {
        name: &'static [u8],
        rcdata: bool,
    },
    Plaintext,
}

/// Which attributes to keep on elements: copying the rest (a page's
/// `style`, `data-*` blobs, inline JSON) is work no reader uses.
#[derive(Clone, Debug, Default)]
pub struct AttrFilter {
    /// Keep every attribute.
    pub all: bool,
    /// Keep every `data-*` attribute.
    pub data: bool,
    /// Keep these (lower case).
    pub names: Vec<String>,
    /// Their case-folded hashes, checked before the names ([`AttrFilter::prepare`]).
    pub(crate) hashes: Vec<u32>,
    /// Bit `n` set: some name is `n` bytes long (bit 63: one is 63 or longer).
    pub(crate) lengths: u64,
}

/// FNV-1a of a name, ASCII case folded.
#[inline]
fn folded_hash(raw: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in raw {
        h = (h ^ b.to_ascii_lowercase() as u32).wrapping_mul(0x0100_0193);
    }
    h
}

impl AttrFilter {
    #[cfg(test)]
    pub fn all() -> AttrFilter {
        AttrFilter {
            all: true,
            ..AttrFilter::default()
        }
    }

    /// Ready to filter: the names deduplicated and hashed.
    pub fn prepare(mut self) -> AttrFilter {
        self.names.sort();
        self.names.dedup();
        self.hashes = self.names.iter().map(|n| folded_hash(n.as_bytes())).collect();
        self.lengths = self.names.iter().fold(0, |m, n| m | 1u64 << n.len().min(63));
        self
    }

    #[inline]
    fn wants(&self, raw: &[u8]) -> bool {
        if self.all || (self.data && raw.len() > 5 && raw[..5].eq_ignore_ascii_case(b"data-")) {
            return true;
        }
        // Most names are ruled out by their length alone.
        if self.lengths & (1u64 << raw.len().min(63)) == 0 {
            return false;
        }
        let h = folded_hash(raw);
        self.hashes
            .iter()
            .zip(&self.names)
            .any(|(&x, n)| x == h && n.as_bytes().eq_ignore_ascii_case(raw))
    }
}

/// The tokenizer's state between chunks.
pub(crate) struct Tokenizer {
    buf: Vec<u8>,
    mode: Mode,
    /// An end tag's name, lower-cased.
    name: String,
    /// The bytes being read are valid UTF-8: slices of them need no checking again.
    valid: bool,
    filter: AttrFilter,
}

enum Step {
    /// The bytes taken.
    Took(usize),
    /// The token is not whole yet.
    Wait,
}

/// Byte classes, one lookup each: whitespace; what ends a tag name (whitespace, `/`, `>`);
/// what ends an attribute name (those and `=`); what ends an unquoted value (whitespace, `>`).
const WS: u8 = 1;
const END_NAME: u8 = 2;
const END_ATTR: u8 = 4;
const END_VALUE: u8 = 8;

static CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let ws = [b' ', b'\n', b'\t', b'\r', 0x0c];
    let mut i = 0;
    while i < ws.len() {
        t[ws[i] as usize] = WS | END_NAME | END_ATTR | END_VALUE;
        i += 1;
    }
    t[b'/' as usize] = END_NAME | END_ATTR;
    t[b'>' as usize] = END_NAME | END_ATTR | END_VALUE;
    t[b'=' as usize] = END_ATTR;
    t
};

#[inline(always)]
fn is_ws(b: u8) -> bool {
    CLASS[b as usize] & WS != 0
}

#[inline(always)]
fn ends_name(b: u8) -> bool {
    CLASS[b as usize] & END_NAME != 0
}

#[inline(always)]
fn ends_attr(b: u8) -> bool {
    CLASS[b as usize] & END_ATTR != 0
}

#[inline(always)]
fn ends_value(b: u8) -> bool {
    CLASS[b as usize] & END_VALUE != 0
}

/// `bytes` lower-cased (ASCII) onto `out`.
fn push_lower(bytes: &[u8], out: &mut String, valid: bool) {
    let at = out.len();
    out.push_str(&utf8_valid(bytes, valid));
    out[at..].make_ascii_lowercase();
}

/// The raw-text elements and whether they decode references.
fn special(kind: u8) -> Option<Mode> {
    use crate::tags as t;
    let (name, rcdata): (&'static [u8], bool) = match kind {
        t::SCRIPT => (b"script", false),
        t::STYLE => (b"style", false),
        t::XMP => (b"xmp", false),
        t::IFRAME => (b"iframe", false),
        t::NOEMBED => (b"noembed", false),
        t::NOFRAMES => (b"noframes", false),
        t::TITLE => (b"title", true),
        t::TEXTAREA => (b"textarea", true),
        t::PLAINTEXT => return Some(Mode::Plaintext),
        _ => return None,
    };
    Some(Mode::Raw { name, rcdata })
}

/// Where text may be cut at the end of a chunk: not inside a UTF-8
/// sequence, and (when references are decoded) not after an `&` that more
/// bytes could complete.
fn safe_end(text: &[u8], references: bool) -> usize {
    let mut end = text.len();
    if references {
        if let Some(amp) = memchr::memrchr(b'&', text) {
            if text.len() - amp <= LOOKAHEAD {
                end = amp;
            }
        }
    }
    utf8_boundary(&text[..end])
}

/// The longest prefix that does not end inside a UTF-8 sequence.
fn utf8_boundary(bytes: &[u8]) -> usize {
    let n = bytes.len();
    for back in 1..=3.min(n) {
        let b = bytes[n - back];
        if b & 0xC0 == 0x80 {
            continue;
        }
        let need = match b {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        return if need > back { n - back } else { n };
    }
    n
}

impl Tokenizer {
    pub(crate) fn new(filter: AttrFilter) -> Tokenizer {
        Tokenizer {
            filter,
            buf: Vec::new(),
            mode: Mode::Data,
            name: String::new(),
            valid: false,
        }
    }

    pub(crate) fn write(&mut self, chunk: &[u8], sink: &mut impl TokenSink) {
        self.write_inner(chunk, false, sink);
    }

    /// [`Tokenizer::write`] of a chunk that outlives the whole pass (the whole document,
    /// read at once): elements may keep pointing into it.
    pub(crate) fn write_pinned(&mut self, chunk: &[u8], sink: &mut impl TokenSink) {
        self.write_inner(chunk, true, sink);
    }

    fn write_inner(&mut self, chunk: &[u8], pinned: bool, sink: &mut impl TokenSink) {
        if self.buf.is_empty() {
            // Most chunks start clean: read them in place.
            let used = self.run(chunk, false, sink);
            if !pinned {
                sink.chunk_end();
            }
            self.buf.extend_from_slice(&chunk[used..]);
        } else {
            self.buf.extend_from_slice(chunk);
            let buf = std::mem::take(&mut self.buf);
            let used = self.run(&buf, false, sink);
            sink.chunk_end();
            self.buf = buf;
            self.buf.drain(..used);
        }
    }

    pub(crate) fn end(&mut self, sink: &mut impl TokenSink) {
        let buf = std::mem::take(&mut self.buf);
        self.run(&buf, true, sink);
        sink.chunk_end();
    }

    /// Tokens out of `buf`; how many bytes were taken.
    fn run(&mut self, buf: &[u8], last: bool, sink: &mut impl TokenSink) -> usize {
        // Validated once here; a token is never cut inside a character, so its bytes are valid too.
        // (Before the end, a character cut by the chunk's end is held back, never handed over.)
        let checked = if last { buf.len() } else { utf8_boundary(buf) };
        self.valid = simdutf8::basic::from_utf8(&buf[..checked]).is_ok();
        let mut pos = 0;
        while pos < buf.len() {
            let rest = &buf[pos..];
            match self.mode {
                Mode::Data => match memchr::memchr(b'<', rest) {
                    Some(0) => match self.markup(rest, last, sink) {
                        Step::Took(n) => pos += n,
                        Step::Wait => return pos,
                    },
                    Some(lt) => {
                        sink.text(&rest[..lt], true, self.valid);
                        pos += lt;
                    }
                    None => {
                        let end = if last { rest.len() } else { safe_end(rest, true) };
                        if end > 0 {
                            sink.text(&rest[..end], true, self.valid);
                        }
                        return pos + end;
                    }
                },
                Mode::Raw { name, rcdata } => match find_end_tag(rest, name) {
                    Some(at) => {
                        if at > 0 {
                            raw_text(&rest[..at], rcdata, self.valid, sink);
                        }
                        self.mode = Mode::Data;
                        pos += at;
                    }
                    None => {
                        let end = if last {
                            rest.len()
                        } else {
                            // Keep what could be the start of `</name`.
                            let keep = name.len() + 3;
                            let upto = rest.len().saturating_sub(keep);
                            safe_end(&rest[..upto], rcdata)
                        };
                        if end > 0 {
                            raw_text(&rest[..end], rcdata, self.valid, sink);
                        }
                        return pos + end;
                    }
                },
                Mode::Plaintext => {
                    let end = if last { rest.len() } else { utf8_boundary(rest) };
                    if end > 0 {
                        sink.text(&rest[..end], false, self.valid);
                    }
                    return pos + end;
                }
            }
        }
        pos
    }

    /// The markup at `s` (which starts with `<`).
    fn markup(&mut self, s: &[u8], last: bool, sink: &mut impl TokenSink) -> Step {
        let Some(&next) = s.get(1) else {
            return if last { text_lt(sink) } else { Step::Wait };
        };
        match next {
            b'!' => {
                if s.starts_with(b"<!--") {
                    return match comment_end(&s[4..]) {
                        Some(end) => {
                            sink.comment(&s[..4 + end]);
                            Step::Took(4 + end)
                        }
                        None if last => {
                            sink.comment(s);
                            Step::Took(s.len())
                        }
                        None => Step::Wait,
                    };
                }
                if !last && s.len() < 4 && b"<!--".starts_with(s) {
                    return Step::Wait;
                }
                declaration(s, 2, last, sink)
            }
            b'?' => declaration(s, 1, last, sink),
            b'/' => {
                let Some(&c) = s.get(2) else {
                    return if last { Step::Took(s.len()) } else { Step::Wait };
                };
                if c == b'>' {
                    return Step::Took(3);
                }
                if !c.is_ascii_alphabetic() {
                    return declaration(s, 2, last, sink);
                }
                let name_end = s[2..].iter().position(|&b| ends_name(b)).map(|i| i + 2);
                let gt = name_end.and_then(|e| memchr::memchr(b'>', &s[e..]).map(|i| e + i));
                match (name_end, gt) {
                    (Some(e), Some(gt)) => {
                        let raw_name = &s[2..e];
                        if self.valid && !raw_name.iter().any(u8::is_ascii_uppercase) {
                            // SAFETY: validated, cut at ASCII bytes.
                            let name = unsafe { std::str::from_utf8_unchecked(raw_name) };
                            sink.end_tag(name, &s[..gt + 1]);
                        } else {
                            self.name.clear();
                            push_lower(raw_name, &mut self.name, self.valid);
                            sink.end_tag(&self.name, &s[..gt + 1]);
                        }
                        Step::Took(gt + 1)
                    }
                    _ if last => Step::Took(s.len()),
                    _ => Step::Wait,
                }
            }
            c if c.is_ascii_alphabetic() => match match start_tag_in_place(s, self.valid, &self.filter) {
                InPlace::Tag(tag) => Some(tag),
                InPlace::Incomplete => None,
                InPlace::Owned => start_tag(s, self.valid, &self.filter, sink.buffer()),
            } {
                Some(tag) => {
                    let mode = if sink.in_foreign() { None } else { special(tag.el.kind) };
                    sink.start_tag(tag.el, tag.self_closing, &s[..tag.len]);
                    if let Some(mode) = mode {
                        self.mode = mode;
                    }
                    Step::Took(tag.len)
                }
                None if last => Step::Took(s.len()),
                None => Step::Wait,
            },
            _ => text_lt(sink),
        }
    }
}

fn text_lt(sink: &mut impl TokenSink) -> Step {
    sink.text(b"<", false, true);
    Step::Took(1)
}

fn raw_text(raw: &[u8], rcdata: bool, valid: bool, sink: &mut impl TokenSink) {
    sink.text(raw, rcdata, valid);
}

/// `<!…>` and `<?…>`: up to the next `>`, ignored as a comment.
fn declaration(s: &[u8], from: usize, last: bool, sink: &mut impl TokenSink) -> Step {
    match memchr::memchr(b'>', &s[from..]) {
        Some(i) => {
            sink.comment(&s[..from + i + 1]);
            Step::Took(from + i + 1)
        }
        None if last => Step::Took(s.len()),
        None => Step::Wait,
    }
}

/// Where a comment's body ends (`-->` or `--!>`), past its end.
fn comment_end(body: &[u8]) -> Option<usize> {
    let mut at = 0;
    while let Some(i) = memchr::memmem::find(&body[at..], b"--") {
        let k = at + i;
        match body.get(k + 2) {
            Some(b'>') => return Some(k + 3),
            Some(b'!') if body.get(k + 3) == Some(&b'>') => return Some(k + 4),
            Some(_) => at = k + 1,
            None => return None,
        }
    }
    None
}

/// Where `</name` (any case) followed by whitespace, `/` or `>` starts.
fn find_end_tag(s: &[u8], name: &[u8]) -> Option<usize> {
    let mut at = 0;
    while let Some(i) = memchr::memchr(b'<', &s[at..]) {
        let k = at + i;
        let tail = &s[k..];
        if tail.len() < name.len() + 3 {
            return None;
        }
        if tail[1] == b'/' && tail[2..2 + name.len()].eq_ignore_ascii_case(name) && ends_name(tail[2 + name.len()]) {
            return Some(k);
        }
        at = k + 1;
    }
    None
}

struct StartTag {
    len: usize,
    el: Element,
    self_closing: bool,
}

enum InPlace {
    Tag(StartTag),
    Incomplete,
    /// It needs copying: a name in upper case, a reference in a kept value, unchecked bytes.
    Owned,
}

/// A whole start tag at `s` read in place: its element points into `s` (see [`Element::borrowed`]).
fn start_tag_in_place(s: &[u8], valid: bool, filter: &AttrFilter) -> InPlace {
    if !valid {
        return InPlace::Owned;
    }
    let n = s.len();
    let mut i = 1;
    while i < n && !ends_name(s[i]) {
        if s[i].is_ascii_uppercase() {
            return InPlace::Owned;
        }
        i += 1;
    }
    if i >= n {
        return InPlace::Incomplete;
    }
    let name = (1, i as u32);
    let mut attrs: SmallVec<[(u32, u32, u32, u32); 2]> = SmallVec::new();
    loop {
        while i < n && is_ws(s[i]) {
            i += 1;
        }
        let Some(&c) = s.get(i) else { return InPlace::Incomplete };
        if c == b'>' {
            let len = i + 1;
            return InPlace::Tag(StartTag {
                len,
                el: Element::borrowed(&s[..len], name, attrs),
                self_closing: false,
            });
        }
        if c == b'/' {
            i += 1;
            while i < n && is_ws(s[i]) {
                i += 1;
            }
            match s.get(i) {
                None => return InPlace::Incomplete,
                Some(b'>') => {
                    let len = i + 1;
                    return InPlace::Tag(StartTag {
                        len,
                        el: Element::borrowed(&s[..len], name, attrs),
                        self_closing: true,
                    });
                }
                Some(_) => continue,
            }
        }
        let name_start = i;
        i += 1;
        while i < n && !ends_attr(s[i]) {
            i += 1;
        }
        if i >= n {
            return InPlace::Incomplete;
        }
        let name_end = i;
        let keep = filter.wants(&s[name_start..name_end]);
        if keep && s[name_start..name_end].iter().any(u8::is_ascii_uppercase) {
            return InPlace::Owned;
        }
        while i < n && is_ws(s[i]) {
            i += 1;
        }
        let (mut vs, mut ve) = (name_end, name_end);
        let Some(&eq) = s.get(i) else {
            return InPlace::Incomplete;
        };
        if eq == b'=' {
            i += 1;
            while i < n && is_ws(s[i]) {
                i += 1;
            }
            let Some(&q) = s.get(i) else { return InPlace::Incomplete };
            if q == b'"' || q == b'\'' {
                let Some(close) = memchr::memchr(q, &s[i + 1..]) else {
                    return InPlace::Incomplete;
                };
                vs = i + 1;
                ve = i + 1 + close;
                i = ve + 1;
            } else {
                vs = i;
                while i < n && !ends_value(s[i]) {
                    i += 1;
                }
                if i >= n {
                    return InPlace::Incomplete;
                }
                ve = i;
            }
        }
        if !keep {
            continue;
        }
        if memchr::memchr(b'&', &s[vs..ve]).is_some() {
            return InPlace::Owned;
        }
        let this = &s[name_start..name_end];
        if !attrs.iter().any(|&(a, b, _, _)| &s[a as usize..b as usize] == this) {
            attrs.push((name_start as u32, name_end as u32, vs as u32, ve as u32));
        }
    }
}

/// An attribute value onto `out`, references decoded.
fn push_value(raw: &[u8], out: &mut String, valid: bool) {
    if memchr::memchr(b'&', raw).is_some() {
        decode_into_valid(raw, true, out, valid);
    } else {
        out.push_str(&utf8_valid(raw, valid));
    }
}

/// A whole start tag at `s`, or None while it is not whole.
fn start_tag(s: &[u8], valid: bool, filter: &AttrFilter, mut buf: String) -> Option<StartTag> {
    let n = s.len();
    let mut i = 1;
    while i < n && !ends_name(s[i]) {
        i += 1;
    }
    if i >= n {
        return None;
    }
    buf.reserve(i + 32);
    push_lower(&s[1..i], &mut buf, valid);
    let mut el = Element::begin(buf);
    loop {
        while i < n && is_ws(s[i]) {
            i += 1;
        }
        let &c = s.get(i)?;
        if c == b'>' {
            return Some(StartTag {
                len: i + 1,
                el,
                self_closing: false,
            });
        }
        if c == b'/' {
            i += 1;
            while i < n && is_ws(s[i]) {
                i += 1;
            }
            if *s.get(i)? == b'>' {
                return Some(StartTag {
                    len: i + 1,
                    el,
                    self_closing: true,
                });
            }
            continue;
        }
        // An attribute's name: its first character whatever it is, then up to `=`, whitespace, `/` or `>`.
        let name_start = i;
        i += 1;
        while i < n && !ends_attr(s[i]) {
            i += 1;
        }
        if i >= n {
            return None;
        }
        let keep = filter.wants(&s[name_start..i]);
        let at = el.buf_mut().len();
        if keep {
            push_lower(&s[name_start..i], el.buf_mut(), valid);
        }
        let name_end = el.buf_mut().len();
        while i < n && is_ws(s[i]) {
            i += 1;
        }
        if *s.get(i)? == b'=' {
            i += 1;
            while i < n && is_ws(s[i]) {
                i += 1;
            }
            let &q = s.get(i)?;
            if q == b'"' || q == b'\'' {
                let close = memchr::memchr(q, &s[i + 1..])? + i + 1;
                if keep {
                    push_value(&s[i + 1..close], el.buf_mut(), valid);
                }
                i = close + 1;
            } else {
                let start = i;
                while i < n && !ends_value(s[i]) {
                    i += 1;
                }
                if i >= n {
                    return None;
                }
                if keep {
                    push_value(&s[start..i], el.buf_mut(), valid);
                }
            }
        }
        if !keep {
            continue;
        }
        el.end_attr(at, name_end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl TokenSink for Log {
        fn start_tag(&mut self, el: Element, self_closing: bool, _: &[u8]) {
            let attrs: String = el.attrs().map(|(k, v)| format!(" {k}={v}")).collect();
            self.0
                .push(format!("<{}{attrs}{}>", el.name(), if self_closing { "/" } else { "" }));
        }
        fn end_tag(&mut self, name: &str, _: &[u8]) {
            self.0.push(format!("</{name}>"));
        }
        fn text(&mut self, raw: &[u8], references: bool, valid: bool) {
            let decoded = crate::decode::text_of(raw, references, valid);
            let text: &str = &decoded;
            // Pieces of one run of text are one text.
            match self.0.last_mut() {
                Some(last) if last.starts_with('"') => {
                    last.pop();
                    last.push_str(text);
                    last.push('"');
                }
                _ => self.0.push(format!("\"{text}\"")),
            }
        }
        fn comment(&mut self, _: &[u8]) {
            self.0.push("#".into());
        }
        fn in_foreign(&self) -> bool {
            false
        }
    }

    fn tokens(html: &str) -> Vec<String> {
        let mut log = Log::default();
        let mut t = Tokenizer::new(AttrFilter::all());
        t.write(html.as_bytes(), &mut log);
        t.end(&mut log);
        log.0
    }

    /// The same tokens whatever the chunks: fed a byte at a time.
    fn bytewise(html: &str) -> Vec<String> {
        let mut log = Log::default();
        let mut t = Tokenizer::new(AttrFilter::all());
        for b in html.as_bytes() {
            t.write(std::slice::from_ref(b), &mut log);
        }
        t.end(&mut log);
        log.0
    }

    #[test]
    fn tags_attributes_text() {
        let html = r#"<!DOCTYPE html><A HREF="/x?a=1&amp;b=2" data-x='y' checked hidden=no>T&eacute; &lt;</a><br/><p class=a/b>x"#;
        let want = [
            "#",
            "<a href=/x?a=1&b=2 data-x=y checked= hidden=no>",
            "\"Té <\"",
            "</a>",
            "<br/>",
            "<p class=a/b>",
            "\"x\"",
        ];
        assert_eq!(tokens(html), want);
        assert_eq!(bytewise(html), want);
    }

    #[test]
    fn raw_text_elements() {
        let html = "<script type=\"application/ld+json\">{\"a\":\"</b>&amp;\"}</SCRIPT ><title>A &amp; B</title><style>p{}</style>";
        let want = [
            "<script type=application/ld+json>",
            "\"{\"a\":\"</b>&amp;\"}\"",
            "</script>",
            "<title>",
            "\"A & B\"",
            "</title>",
            "<style>",
            "\"p{}\"",
            "</style>",
        ];
        assert_eq!(tokens(html), want);
        assert_eq!(bytewise(html), want);
    }

    #[test]
    fn comments_and_odd_markup() {
        let html = "a<!-- x -- y --!>b<!-- -->c< d</ e>f</>g<?php x ?>h<!-- open";
        let want = ["\"a\"", "#", "\"b\"", "#", "\"c< d\"", "#", "\"fg\"", "#", "\"h\"", "#"];
        let got = tokens(html);
        assert_eq!(got, want);
        assert_eq!(bytewise(html), want);
    }

    #[test]
    fn a_cut_tag_waits_and_is_dropped_at_the_end() {
        assert_eq!(tokens("x<div class=\"a"), ["\"x\""]);
        assert_eq!(
            bytewise("é&amp;ü<b>&eacute</b>&euro x"),
            ["\"é&ü\"", "<b>", "\"é\"", "</b>", "\"&euro x\""]
        );
    }
}
