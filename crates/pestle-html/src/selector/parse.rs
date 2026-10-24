//! CSS selectors parsed: what css-what parses, less what a stream cannot
//! answer (see [`super`]).

use super::{AttrOp, Combinator, Complex, Compound, Cond, Nth};

pub(crate) fn parse_list(src: &str) -> Result<Vec<Complex>, String> {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
    };
    let list = p.list(false)?;
    p.ws();
    if p.i < p.s.len() {
        return Err(p.error("unexpected character"));
    }
    Ok(list)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at {} in {:?}", self.i, self.src)
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) -> bool {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
        {
            self.i += 1;
        }
        self.i > start
    }

    /// A selector list, up to the end or (`nested`) a `)`.
    fn list(&mut self, nested: bool) -> Result<Vec<Complex>, String> {
        let mut out = Vec::new();
        loop {
            self.ws();
            out.push(self.complex()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') if nested => return Ok(out),
                None if !nested => return Ok(out),
                None => return Err(self.error("missing )")),
                Some(_) => return Err(self.error("unexpected character")),
            }
        }
    }

    fn combinator(&mut self) -> Option<Combinator> {
        let c = match self.peek()? {
            b'>' => Combinator::Child,
            b'+' => Combinator::Adjacent,
            b'~' => Combinator::Sibling,
            _ => return None,
        };
        self.i += 1;
        Some(c)
    }

    fn complex(&mut self) -> Result<Complex, String> {
        let mut parts = Vec::new();
        // A leading combinator: relative to the scope.
        let mut comb = self.combinator();
        let leading = comb;
        self.ws();
        loop {
            let compound = self.compound()?;
            parts.push((comb.unwrap_or(Combinator::Descendant), compound));
            let spaced = self.ws();
            comb = self.combinator();
            if comb.is_some() {
                self.ws();
            } else if spaced && self.peek().is_some_and(|b| b != b',' && b != b')') {
                comb = Some(Combinator::Descendant);
            } else {
                return Ok(Complex { parts, leading });
            }
        }
    }

    fn compound(&mut self) -> Result<Compound, String> {
        let mut c = Compound::default();
        let start = self.i;
        match self.peek() {
            Some(b'*') => self.i += 1,
            Some(b) if is_ident_start(b) || b == b'\\' => {
                let tag = self.ident()?.to_ascii_lowercase();
                c.hash = crate::tree::tag_hash(&tag);
                c.tag = Some(tag);
            }
            _ => {}
        }
        loop {
            match self.peek() {
                Some(b'#') => {
                    self.i += 1;
                    c.conds.push(Cond::Id(self.ident()?));
                }
                Some(b'.') => {
                    self.i += 1;
                    c.conds.push(Cond::Class(self.ident()?));
                }
                Some(b'[') => {
                    self.i += 1;
                    c.conds.push(self.attribute()?);
                }
                Some(b':') => {
                    self.i += 1;
                    if self.peek() == Some(b':') {
                        return Err(self.error("pseudo-elements select no element"));
                    }
                    c.conds.push(self.pseudo()?);
                }
                _ => break,
            }
        }
        if self.i == start {
            return Err(self.error("expected a selector"));
        }
        Ok(c)
    }

    fn attribute(&mut self) -> Result<Cond, String> {
        self.ws();
        let name = self.ident()?.to_ascii_lowercase();
        self.ws();
        let op = match self.peek() {
            Some(b']') => {
                self.i += 1;
                return Ok(Cond::Attr {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    ignore_case: None,
                });
            }
            Some(b'=') => {
                self.i += 1;
                AttrOp::Equals
            }
            Some(b) => {
                let op = match b {
                    b'~' => AttrOp::Includes,
                    b'|' => AttrOp::Dash,
                    b'^' => AttrOp::Prefix,
                    b'$' => AttrOp::Suffix,
                    b'*' => AttrOp::Substring,
                    b'!' => AttrOp::NotEquals,
                    _ => return Err(self.error("expected an attribute operator")),
                };
                if self.s.get(self.i + 1) != Some(&b'=') {
                    return Err(self.error("expected ="));
                }
                self.i += 2;
                op
            }
            None => return Err(self.error("missing ]")),
        };
        self.ws();
        let value = match self.peek() {
            Some(b'"' | b'\'') => self.string()?,
            _ => self.ident()?,
        };
        self.ws();
        let ignore_case = match self.peek() {
            Some(b'i' | b'I') => {
                self.i += 1;
                Some(true)
            }
            Some(b's' | b'S') => {
                self.i += 1;
                Some(false)
            }
            _ => None,
        };
        self.ws();
        if self.peek() != Some(b']') {
            return Err(self.error("missing ]"));
        }
        self.i += 1;
        Ok(Cond::Attr {
            name,
            op,
            value,
            ignore_case,
        })
    }

    fn pseudo(&mut self) -> Result<Cond, String> {
        let name = self.ident()?.to_ascii_lowercase();
        let has_args = self.peek() == Some(b'(');
        if has_args {
            self.i += 1;
        }
        let cond = match (name.as_str(), has_args) {
            ("first-child", false) => Cond::Nth {
                nth: Nth { a: 0, b: 1 },
                of_type: false,
                last: false,
            },
            ("last-child", false) => Cond::Nth {
                nth: Nth { a: 0, b: 1 },
                of_type: false,
                last: true,
            },
            ("first-of-type", false) => Cond::Nth {
                nth: Nth { a: 0, b: 1 },
                of_type: true,
                last: false,
            },
            ("last-of-type", false) => Cond::Nth {
                nth: Nth { a: 0, b: 1 },
                of_type: true,
                last: true,
            },
            ("only-child", false) => Cond::Only { of_type: false },
            ("only-of-type", false) => Cond::Only { of_type: true },
            ("root", false) => Cond::Root,
            ("scope", false) => Cond::Scope,
            ("empty", false) => Cond::Empty,
            ("nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type", true) => {
                let nth = self.nth()?;
                Cond::Nth {
                    nth,
                    of_type: name.ends_with("of-type"),
                    last: name.contains("last"),
                }
            }
            ("not" | "is" | "matches" | "where" | "has", true) => {
                let list = self.list(true)?;
                match name.as_str() {
                    "not" => Cond::Not(list),
                    "has" => Cond::Has(list),
                    _ => Cond::Is(list),
                }
            }
            ("contains" | "icontains" | "-soup-contains", true) => {
                self.ws();
                let text = match self.peek() {
                    Some(b'"' | b'\'') => self.string()?,
                    _ => {
                        let start = self.i;
                        while self.peek().is_some_and(|b| b != b')') {
                            self.i += 1;
                        }
                        self.src[start..self.i].trim().to_owned()
                    }
                };
                Cond::Contains {
                    text,
                    ignore_case: name == "icontains",
                }
            }
            _ => return Err(self.error(&format!("unsupported pseudo-class :{name}"))),
        };
        if has_args {
            self.ws();
            if self.peek() != Some(b')') {
                return Err(self.error("missing )"));
            }
            self.i += 1;
        }
        Ok(cond)
    }

    /// `an+b`, `odd`, `even`.
    fn nth(&mut self) -> Result<Nth, String> {
        let start = self.i;
        while self.peek().is_some_and(|b| b != b')') {
            self.i += 1;
        }
        let text: String = self.src[start..self.i]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let bad = || format!("bad nth expression {text:?} in {:?}", self.src);
        match text.as_str() {
            "odd" => return Ok(Nth { a: 2, b: 1 }),
            "even" => return Ok(Nth { a: 2, b: 0 }),
            _ => {}
        }
        let Some(n) = text.find('n') else {
            return text.parse().map(|b| Nth { a: 0, b }).map_err(|_| bad());
        };
        let a = match &text[..n] {
            "" | "+" => 1,
            "-" => -1,
            s => s.parse().map_err(|_| bad())?,
        };
        let b = match &text[n + 1..] {
            "" => 0,
            s if s.starts_with('+') || s.starts_with('-') => s.trim_start_matches('+').parse().map_err(|_| bad())?,
            _ => return Err(bad()),
        };
        Ok(Nth { a, b })
    }

    fn string(&mut self) -> Result<String, String> {
        let q = self.peek().ok_or_else(|| self.error("expected a string"))?;
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b) if b == q => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    self.escape(&mut out);
                }
                Some(_) => self.char(&mut out),
            }
        }
    }

    fn char(&mut self, out: &mut String) {
        let c = self.src[self.i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(c);
        self.i += c.len_utf8().max(1);
    }

    /// After a `\`: up to six hex digits (and one space), or the next character itself.
    fn escape(&mut self, out: &mut String) {
        let start = self.i;
        while self.i - start < 6 && self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
            self.i += 1;
        }
        if self.i > start {
            let code = u32::from_str_radix(&self.src[start..self.i], 16).unwrap_or(0xFFFD);
            out.push(char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
            if self.peek().is_some_and(|b| b == b' ') {
                self.i += 1;
            }
        } else if self.peek().is_some() {
            self.char(out);
        }
    }

    fn ident(&mut self) -> Result<String, String> {
        let mut out = String::new();
        while let Some(b) = self.peek() {
            if b == b'\\' {
                self.i += 1;
                self.escape(&mut out);
            } else if is_ident_char(b) {
                self.char(&mut out);
            } else {
                break;
            }
        }
        if out.is_empty() {
            return Err(self.error("expected a name"));
        }
        Ok(out)
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'-' || b >= 0x80
}

fn is_ident_char(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}
