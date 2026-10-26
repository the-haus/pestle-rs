//! The tags the tree rules name, as small numbers: an element's kind is
//! looked up once, when its start tag is read, and the rules (implied
//! closes, void elements, foreign content, raw text) compare numbers.

use std::sync::OnceLock;

macro_rules! tags {
    ($($id:ident = $name:literal),* $(,)?) => {
        /// A tag no rule names.
        pub const OTHER: u8 = 0;
        tags!(@consts 1u8, $($id),*);
        /// Every named tag, by kind (index 0: none).
        pub static NAMES: &[&str] = &["", $($name),*];
    };
    (@consts $n:expr, $id:ident $(, $rest:ident)*) => {
        pub const $id: u8 = $n;
        tags!(@consts $n + 1u8 $(, $rest)*);
    };
    (@consts $n:expr) => {};
}

tags! {
    A = "a", ABBR = "abbr", B = "b", BDI = "bdi", BDO = "bdo", CANVAS = "canvas", CITE = "cite", CODE = "code",
    DATA = "data", EM = "em", I = "i", KBD = "kbd", MARK = "mark", NOSCRIPT = "noscript", OBJECT = "object",
    Q = "q", S = "s", SMALL = "small", SPAN = "span", STRONG = "strong", SUB = "sub", SUP = "sup", TIME = "time",
    U = "u", VAR = "var", ADDRESS = "address", ANNOTATION_XML = "annotation-xml", AREA = "area", ARTICLE = "article",
    ASIDE = "aside", BASE = "base", BASEFONT = "basefont", BLOCKQUOTE = "blockquote", BODY = "body", BR = "br",
    BUTTON = "button", COL = "col", COMMAND = "command", DATALIST = "datalist", DD = "dd", DESC = "desc",
    DETAILS = "details", DIV = "div", DL = "dl", DT = "dt", EMBED = "embed", FIELDSET = "fieldset",
    FIGCAPTION = "figcaption", FIGURE = "figure", FOOTER = "footer", FOREIGNOBJECT = "foreignobject", FORM = "form",
    FRAME = "frame", H1 = "h1", H2 = "h2", H3 = "h3", H4 = "h4", H5 = "h5", H6 = "h6", HEAD = "head",
    HEADER = "header", HR = "hr", HTML = "html", IFRAME = "iframe", IMAGE = "image", IMG = "img", INPUT = "input",
    ISINDEX = "isindex", KEYGEN = "keygen", LI = "li", LINK = "link", MAIN = "main", MATH = "math", META = "meta",
    MI = "mi", MN = "mn", MO = "mo", MS = "ms", MTEXT = "mtext", NAV = "nav", NOEMBED = "noembed",
    NOFRAMES = "noframes", OL = "ol", OPTGROUP = "optgroup", OPTION = "option", OUTPUT = "output", P = "p",
    PARAM = "param", PLAINTEXT = "plaintext", PRE = "pre", RP = "rp", RT = "rt", SCRIPT = "script",
    SECTION = "section", SELECT = "select", SOURCE = "source", STYLE = "style", SVG = "svg", TABLE = "table",
    TBODY = "tbody", TD = "td", TEMPLATE = "template", TEXTAREA = "textarea", TFOOT = "tfoot", TH = "th",
    THEAD = "thead", TITLE = "title", TR = "tr", TRACK = "track", UL = "ul", WBR = "wbr", XMP = "xmp",
    CAPTION = "caption", DIALOG = "dialog", HGROUP = "hgroup", LEGEND = "legend", MENU = "menu", SEARCH = "search",
    SUMMARY = "summary",
}

const SLOTS: usize = 512;

fn table() -> &'static [(u32, u8); SLOTS] {
    static TABLE: OnceLock<[(u32, u8); SLOTS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [(0u32, OTHER); SLOTS];
        for (kind, name) in NAMES.iter().enumerate().skip(1) {
            let h = crate::tree::tag_hash(name);
            let mut i = h as usize % SLOTS;
            while t[i].1 != OTHER {
                i = (i + 1) % SLOTS;
            }
            t[i] = (h, kind as u8);
        }
        t
    })
}

/// Elements that do not break a run of text, by kind (see [`crate::text::is_inline`]).
pub fn is_inline(kind: u8) -> bool {
    matches!(
        kind,
        A | ABBR
            | B
            | BDI
            | BDO
            | CITE
            | CODE
            | DATA
            | EM
            | I
            | KBD
            | MARK
            | Q
            | S
            | SMALL
            | SPAN
            | STRONG
            | SUB
            | SUP
            | TIME
            | U
            | VAR
            | WBR
    )
}

/// Elements that end a line of text read as lines (see [`crate::text::breaks_line`]), by kind.
pub fn breaks_line(kind: u8) -> bool {
    matches!(
        kind,
        ADDRESS
            | ARTICLE
            | ASIDE
            | BLOCKQUOTE
            | BODY
            | BR
            | CAPTION
            | DD
            | DETAILS
            | DIALOG
            | DIV
            | DL
            | DT
            | FIELDSET
            | FIGCAPTION
            | FIGURE
            | FOOTER
            | FORM
            | H1
            | H2
            | H3
            | H4
            | H5
            | H6
            | HEADER
            | HGROUP
            | HR
            | HTML
            | LEGEND
            | LI
            | MAIN
            | MENU
            | NAV
            | OL
            | OPTION
            | P
            | PRE
            | SEARCH
            | SECTION
            | SUMMARY
            | TABLE
            | TBODY
            | TFOOT
            | THEAD
            | TR
            | UL
    )
}

/// What an element puts in a run of text where it opens or closes: a line break (`lines`, a
/// block), a space (any other element but an inline one), or nothing.
pub fn text_break(kind: u8, lines: bool) -> Option<char> {
    if lines && breaks_line(kind) {
        Some('\n')
    } else if is_inline(kind) {
        None
    } else {
        Some(' ')
    }
}

/// Elements whose content is never shown, by kind (see [`crate::text::is_never_text`]).
pub fn is_never_text(kind: u8) -> bool {
    matches!(
        kind,
        SCRIPT | STYLE | NOSCRIPT | TEMPLATE | SVG | IFRAME | OBJECT | CANVAS
    )
}

/// The kind of a tag named `name` (lower case), whose [`crate::tree::tag_hash`] is `hash`.
#[inline]
pub fn kind(name: &str, hash: u32) -> u8 {
    let t = table();
    let mut i = hash as usize % SLOTS;
    loop {
        let (h, k) = t[i];
        if k == OTHER {
            return OTHER;
        }
        if h == hash && NAMES[k as usize] == name {
            return k;
        }
        i = (i + 1) % SLOTS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_finds_its_kind() {
        for (k, name) in NAMES.iter().enumerate().skip(1) {
            assert_eq!(kind(name, crate::tree::tag_hash(name)) as usize, k, "{name}");
        }
        assert_eq!(kind("festival", crate::tree::tag_hash("festival")), OTHER);
        assert_eq!(NAMES[XMP as usize], "xmp");
    }
}
