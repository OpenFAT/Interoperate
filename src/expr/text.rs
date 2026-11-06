//! Number and string literals (spec §2), `eng` and `spectre_source` (§5).

use super::{Code, Error, Tok, err};

/// Longest string value (spec §1).
pub(super) const MAX_STR: usize = 1 << 20;

/// Appends `i` as canonical text (spec §7: a plain integer).
pub fn write_int(out: &mut String, i: i64) {
    out.push_str(&i.to_string());
}

/// Parses exactly one number literal with an optional sign (`num(s)`, spec §5).
///
/// # Errors
///
/// `X001` for anything else.
pub fn parse_num(s: &str) -> Result<f64, Error> {
    let (neg, body) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let starts = body
        .bytes()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == b'.');
    match starts.then(|| lex_number(body)) {
        Some(Ok((t, n))) if n == body.len() => {
            let x = match t {
                Tok::Int(i) => i as f64,
                Tok::Num(x) => x,
                _ => unreachable!("lex_number yields numbers"),
            };
            Ok(if neg { -x } else { x })
        }
        Some(Err(e)) => Err(e),
        _ => err(Code::X001, format!("`{s}` is not a number")),
    }
}

/// Lexes the number at the start of `s` (which starts with a digit or `.digit`): the token
/// and its byte length.
pub(super) fn lex_number(s: &str) -> Result<(Tok, usize), Error> {
    let b = s.as_bytes();
    let bad = || err(Code::X001, format!("bad number `{s}`"));
    let digits = |k: usize| b[k..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut k = digits(0);
    let int_end = k;
    if b.get(k) == Some(&b'.') {
        let n = digits(k + 1);
        if n == 0 {
            return bad();
        }
        k += 1 + n;
    }
    let mantissa_end = k;
    let mut exp: i64 = 0;
    if matches!(b.get(k), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(k + 1), Some(b'+' | b'-')));
        let n = digits(k + 1 + sign);
        if n == 0 {
            return bad();
        }
        exp = s[k + 1..k + 1 + sign + n].parse().unwrap_or(i64::MAX / 2);
        k += 1 + sign + n;
    }
    let lower = s[k..].to_ascii_lowercase();
    let (scale, mil, n) = [
        ("meg", 6, false),
        ("mil", 0, true),
        ("f", -15, false),
        ("p", -12, false),
        ("n", -9, false),
        ("u", -6, false),
        ("\u{b5}", -6, false),
        ("\u{3bc}", -6, false),
        ("m", -3, false),
        ("k", 3, false),
        ("g", 9, false),
        ("t", 12, false),
    ]
    .iter()
    .find(|(p, ..)| lower.starts_with(p))
    .map_or((0, false, 0), |(p, e, m)| (*e, *m, p.len()));
    let end = k + n;
    if s[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return bad();
    }
    if end == int_end
        && n == 0
        && let Ok(i) = s[..int_end].parse::<i64>()
    {
        return Ok((Tok::Int(i), end));
    }
    // The suffix folds into the decimal exponent: one correctly rounded conversion.
    let text = format!("{}e{}", &s[..mantissa_end], exp.saturating_add(scale));
    let mut x: f64 = text.parse().map_err(|_| bad().unwrap_err())?;
    if mil {
        x *= 25.4e-6;
    }
    if !x.is_finite() {
        return bad();
    }
    Ok((Tok::Num(x), end))
}

/// Lexes the string literal at the start of `s` (which starts with `"`).
pub(super) fn lex_string(s: &str) -> Result<(String, usize), Error> {
    let bad = |why: &str| err(Code::X009, format!("{why} in {s}"));
    let mut out = String::new();
    let mut it = s.char_indices().skip(1);
    while let Some((k, c)) = it.next() {
        match c {
            '"' => {
                if out.len() > MAX_STR {
                    return err(Code::X002, "string over 1 MiB");
                }
                return Ok((out, k + 1));
            }
            '\\' => match it.next().map(|(_, c)| c) {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    if it.next().map(|(_, c)| c) != Some('{') {
                        return bad("bad escape");
                    }
                    let mut hex = String::new();
                    for (_, c) in it.by_ref() {
                        if c == '}' {
                            break;
                        }
                        hex.push(c);
                    }
                    let ch = (1..=6)
                        .contains(&hex.len())
                        .then(|| u32::from_str_radix(&hex, 16).ok())
                        .flatten()
                        .and_then(char::from_u32);
                    match ch {
                        Some(ch) => out.push(ch),
                        None => return bad("bad escape"),
                    }
                }
                _ => return bad("bad escape"),
            },
            c => out.push(c),
        }
    }
    bad("unterminated string")
}

/// `eng(x, digits)`: engineering notation with an SI suffix (spec §5).
pub(super) fn eng(x: f64, digits: usize) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // Rounded to `digits` significant digits; truncated when rounding would overflow.
    let mut e = format!("{:.*e}", digits - 1, x.abs());
    if e.parse::<f64>().map_or(true, |y| !y.is_finite()) {
        let full = format!("{:.17e}", x.abs());
        let (m, ex) = full.split_once('e').expect("exponent form");
        let keep = if digits == 1 { 1 } else { digits + 1 };
        e = format!("{}e{ex}", &m[..keep]);
    }
    let (m, ex) = e.split_once('e').expect("exponent form");
    let ex: i32 = ex.parse().expect("exponent");
    let ds: String = m.chars().filter(char::is_ascii_digit).collect();
    if !(-18..=17).contains(&ex) {
        let mut t = String::from(&ds[..1]);
        let frac = ds[1..].trim_end_matches('0');
        if !frac.is_empty() {
            t.push('.');
            t.push_str(frac);
        }
        return format!("{sign}{t}e{ex}");
    }
    let s = (ex.div_euclid(3) * 3).clamp(-15, 12);
    let suffix = ["f", "p", "n", "u", "m", "", "k", "meg", "g", "t"][((s + 15) / 3) as usize];
    let point = ex - s + 1; // integer digits
    let mut t = if point <= 0 {
        format!("0.{}{ds}", "0".repeat(point.unsigned_abs() as usize))
    } else if point as usize >= ds.len() {
        format!("{ds}{}", "0".repeat(point as usize - ds.len()))
    } else {
        format!("{}.{}", &ds[..point as usize], &ds[point as usize..])
    };
    if t.contains('.') {
        t = t.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    format!("{sign}{t}{suffix}")
}

/// `spectre_source(s)`: a SPICE source specification as Spectre parameters (spec §5).
pub(super) fn spectre_source(s: &str) -> Result<String, Error> {
    let bad = || {
        err(
            Code::X009,
            format!("spectre_source: unsupported source `{s}`"),
        )
    };
    let mut toks: Vec<&str> = Vec::new();
    let mut k = 0;
    let b = s.as_bytes();
    while k < b.len() {
        match b[k] {
            b' ' | b'\t' | b',' => k += 1,
            b'(' | b')' => {
                toks.push(&s[k..k + 1]);
                k += 1;
            }
            _ => {
                let n = b[k..]
                    .iter()
                    .take_while(|c| !matches!(c, b' ' | b'\t' | b',' | b'(' | b')'))
                    .count();
                toks.push(&s[k..k + n]);
                k += n;
            }
        }
    }
    let value = |t: &str| -> Option<String> {
        if let Ok(x) = parse_num(t) {
            let mut o = String::new();
            super::write_num(&mut o, x);
            return Some(o);
        }
        let id = t
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            && t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        id.then(|| t.to_owned())
    };
    let is_value = |t: Option<&&str>| {
        t.is_some_and(|t| {
            !matches!(
                t.to_ascii_lowercase().as_str(),
                "dc" | "ac" | "pulse" | "sin" | "exp" | "pwl"
            ) && value(t).is_some()
        })
    };
    let (mut dc, mut ac, mut func): (Option<String>, Option<Vec<String>>, Option<String>) =
        (None, None, None);
    let mut i = 0;
    while i < toks.len() {
        let w = toks[i].to_ascii_lowercase();
        let call =
            matches!(w.as_str(), "pulse" | "sin" | "exp" | "pwl") && toks.get(i + 1) == Some(&"(");
        i += 1;
        if !call && w != "dc" && w != "ac" {
            // A bare value is the DC value.
            if dc.is_some() {
                return bad();
            }
            dc = Some(value(toks[i - 1]).map_or_else(bad, Ok)?);
            continue;
        }
        match w.as_str() {
            "dc" => {
                if dc.is_some() || !is_value(toks.get(i)) {
                    return bad();
                }
                dc = value(toks[i]);
                i += 1;
            }
            "ac" => {
                if ac.is_some() || !is_value(toks.get(i)) {
                    return bad();
                }
                let mut v = vec![value(toks[i]).expect("checked")];
                i += 1;
                if is_value(toks.get(i)) && toks.get(i + 1) != Some(&"(") {
                    v.push(value(toks[i]).expect("checked"));
                    i += 1;
                }
                ac = Some(v);
            }
            _ => {
                if func.is_some() {
                    return bad();
                }
                i += 1; // `(`
                let mut args = Vec::new();
                while i < toks.len() && toks[i] != ")" {
                    args.push(value(toks[i]).map_or_else(bad, Ok)?);
                    i += 1;
                }
                if i >= toks.len() {
                    return bad();
                }
                i += 1; // `)`
                let names: &[&str] = match w.as_str() {
                    "pulse" => &["val0", "val1", "delay", "rise", "fall", "width", "period"],
                    "sin" => &["sinedc", "ampl", "freq", "delay", "damp", "sinephase"],
                    "exp" => &["val0", "val1", "td1", "tau1", "td2", "tau2"],
                    _ => &[],
                };
                let text = if w == "pwl" {
                    if args.is_empty() || args.len() % 2 != 0 {
                        return bad();
                    }
                    format!("type=pwl wave=[{}]", args.join(" "))
                } else {
                    if args.len() > names.len() || (w == "exp" && args.len() < 2) {
                        return bad();
                    }
                    let kind = if w == "sin" { "sine" } else { w.as_str() };
                    let mut t = format!("type={kind}");
                    for (n, a) in names.iter().zip(&args) {
                        t.push_str(&format!(" {n}={a}"));
                    }
                    t
                };
                func = Some(text);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(d) = dc {
        out.push(format!("dc={d}"));
    }
    if let Some(a) = ac {
        out.push(format!("mag={}", a[0]));
        if let Some(p) = a.get(1) {
            out.push(format!("phase={p}"));
        }
    }
    out.extend(func);
    Ok(out.join(" "))
}
