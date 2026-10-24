/// A number as people write it: `25`, `25,50`, `€ 1.250,00`, `1,250.00`,
/// `25,-`, `-3`. The last `.` or `,` followed by one or two digits (or by
/// `-`) is the decimal point; other separators group thousands. Anything
/// around the first run of digits is ignored. None when there is no digit.
pub fn lenient_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let first = bytes.iter().position(u8::is_ascii_digit)?;
    let negative = text[..first].trim_end().ends_with('-');
    let mut end = first;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || matches!(bytes[end], b'.' | b',' | b' ' | b'\'')) {
        // A space or apostrophe groups digits only between digits.
        if matches!(bytes[end], b' ' | b'\'') && !bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
            break;
        }
        end += 1;
    }
    let run = text[first..end].trim_end_matches(['.', ',', ' ', '\'']);
    let decimal_at = decimal_point(run);
    let mut digits = String::with_capacity(run.len());
    for (i, c) in run.char_indices() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if Some(i) == decimal_at {
            digits.push('.');
        }
    }
    let n: f64 = digits.parse().ok()?;
    Some(if negative { -n } else { n })
}

/// Where the decimal point of a run of digits and separators is, if it has one.
fn decimal_point(run: &str) -> Option<usize> {
    let last = run.rfind(['.', ','])?;
    let sep = run.as_bytes()[last] as char;
    let decimals = run.len() - last - 1;
    let other = if sep == '.' { ',' } else { '.' };
    if run.contains(other) {
        // `1.250,00` / `1,250.00`: the last kind is the decimal point.
        return Some(last);
    }
    // One kind only: twice is grouping (`1.250.000`), and so is once with three digits after (`1.250`).
    if run.matches(sep).count() > 1 || decimals == 3 {
        None
    } else {
        Some(last)
    }
}

#[cfg(test)]
mod tests {
    use super::lenient_number as n;

    #[test]
    fn numbers_as_written() {
        assert_eq!(n("25"), Some(25.0));
        assert_eq!(n("€ 25,50"), Some(25.5));
        assert_eq!(n("€25,-"), Some(25.0));
        assert_eq!(n("1.250,00"), Some(1250.0));
        assert_eq!(n("1,250.00"), Some(1250.0));
        assert_eq!(n("1.250"), Some(1250.0));
        assert_eq!(n("12.5"), Some(12.5));
        assert_eq!(n("2 500 visitors"), Some(2500.0));
        assert_eq!(n("-3 °C"), Some(-3.0));
        assert_eq!(n("from 19.95 EUR"), Some(19.95));
        assert_eq!(n("sold out"), None);
    }
}
