//! UTF-16 (how JavaScript holds a string) to UTF-8, fast: eight ASCII units
//! at a time, lone surrogates as U+FFFD.

/// `src` as UTF-8, appended to `out`.
pub fn utf16_to_utf8(src: &[u16], out: &mut Vec<u8>) {
    out.reserve(src.len() + src.len() / 4);
    let n = src.len();
    let mut i = 0;
    while i < n {
        // ASCII runs, eight units at a time.
        while i + 8 <= n {
            let chunk = &src[i..i + 8];
            if chunk.iter().fold(0u16, |a, &u| a | u) >= 0x80 {
                break;
            }
            out.extend(chunk.iter().map(|&u| u as u8));
            i += 8;
        }
        if i >= n {
            break;
        }
        let u = src[i] as u32;
        i += 1;
        if u < 0x80 {
            out.push(u as u8);
        } else if u < 0x800 {
            out.extend_from_slice(&[0xC0 | (u >> 6) as u8, 0x80 | (u & 0x3F) as u8]);
        } else if (0xD800..0xDC00).contains(&u) && i < n && (0xDC00..0xE000).contains(&(src[i] as u32)) {
            let c = 0x10000 + ((u - 0xD800) << 10) + (src[i] as u32 - 0xDC00);
            i += 1;
            out.extend_from_slice(&[
                0xF0 | (c >> 18) as u8,
                0x80 | ((c >> 12) & 0x3F) as u8,
                0x80 | ((c >> 6) & 0x3F) as u8,
                0x80 | (c & 0x3F) as u8,
            ]);
        } else {
            // A lone surrogate is U+FFFD.
            let c = if (0xD800..0xE000).contains(&u) { 0xFFFD } else { u };
            out.extend_from_slice(&[
                0xE0 | (c >> 12) as u8,
                0x80 | ((c >> 6) & 0x3F) as u8,
                0x80 | (c & 0x3F) as u8,
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utf16_to_utf8;

    #[test]
    fn like_std() {
        for s in [
            "",
            "plain ascii text, longer than eight units",
            "café · 🎸 – 中文 ok",
            "a\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}",
        ] {
            let units: Vec<u16> = s.encode_utf16().collect();
            let mut out = Vec::new();
            utf16_to_utf8(&units, &mut out);
            assert_eq!(out, s.as_bytes());
        }
        let mut out = Vec::new();
        utf16_to_utf8(&[0x61, 0xD800, 0x62, 0xDC00], &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "a\u{FFFD}b\u{FFFD}");
    }
}
