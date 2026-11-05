//! The words both directions read and write (spec `interop.md` §9.1, §9.2): `key=value`
//! options, identifiers, model paths in ADE's form, sweep grids and top-level splits.

use super::*;

/// Spectre's transient accuracy presets (`tran` parameter `errpreset`).
pub(super) const ERRPRESETS: [&str; 3] = ["liberal", "moderate", "conservative"];

/// Spectre simulator options with an ngspice `.options` of the same meaning: (Spectre,
/// ngspice). Their ICADVM20.1 defaults (forms.tsv) are ngspice's.
pub(super) const NGSPICE_OPTIONS: [(&str, &str); 6] = [
    ("reltol", "reltol"),
    ("iabstol", "abstol"),
    ("vabstol", "vntol"),
    ("gmin", "gmin"),
    ("tnom", "tnom"),
    ("pivrel", "pivrel"),
];

pub(super) fn kv(k: &str, v: &str) -> (String, String) {
    (k.to_owned(), v.to_owned())
}

pub(super) fn is_ident(x: &str) -> bool {
    let mut ch = x.chars();
    ch.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `${NAME}` → `$NAME` (§9.1).
pub(super) fn path_to_ade(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut rest = p;
    while let Some(k) = rest.find("${") {
        out.push_str(&rest[..k]);
        match rest[k + 2..].find('}') {
            Some(e) => {
                out.push('$');
                out.push_str(&rest[k + 2..k + 2 + e]);
                rest = &rest[k + 3 + e..];
            }
            None => {
                out.push_str(&rest[k..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `$NAME` → `${NAME}` (the inverse of [`path_to_ade`]).
pub(super) fn path_from_ade(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 4);
    let b = p.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if b[k] == b'$' && k + 1 < b.len() && (b[k + 1].is_ascii_alphabetic() || b[k + 1] == b'_') {
            let mut e = k + 1;
            while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                e += 1;
            }
            out.push_str("${");
            out.push_str(&p[k + 1..e]);
            out.push('}');
            k = e;
        } else {
            let ch = p[k..].chars().next().expect("in bounds");
            out.push(ch);
            k += ch.len_utf8();
        }
    }
    out
}

/// `n` points from `a` to `b` (run.md §2): linear, or geometric through the expression
/// language (bit-identical on every target), the last exactly `b`; each point rounded to
/// 15 significant digits, then canonical number text (`0.01`, never the binary noise
/// `0.010000000000000002` of `0.8 · 1/80`). The rounding moves a point by at most 5e-16
/// relative, far below any simulator's resolution, and makes the list the decimal grid the
/// bench means.
pub(super) fn grid(a: f64, b: f64, n: u32, log: bool) -> Option<Vec<String>> {
    assert!((1..=SWEEP_POINTS_MAX).contains(&n));
    if log && (a == 0.0 || b == 0.0 || (a < 0.0) != (b < 0.0)) {
        return None;
    }
    let geo = Program::compile("a * (b / a) ^ t", Mode::Scalar).expect("constant source");
    let mut scratch = Scratch::default();
    let mut out = Vec::with_capacity(n as usize);
    for k in 0..n {
        let t = if n == 1 {
            0.0
        } else {
            f64::from(k) / f64::from(n - 1)
        };
        let x = match (k + 1 == n && n > 1, log) {
            (true, _) => b,
            (false, false) => a + (b - a) * t,
            (false, true) => {
                let ins = [XValue::Num(a), XValue::Num(b), XValue::Num(t)];
                match geo.eval(&ins, &mut scratch) {
                    Ok(XValue::Num(x)) => x,
                    _ => return None,
                }
            }
        };
        let x: f64 = format!("{x:.14e}").parse().expect("Rust float text parses");
        let mut s = String::new();
        write_num(&mut s, x);
        out.push(s);
    }
    assert_eq!(out.len(), n as usize);
    Some(out)
}

/// `s` split at commas outside parentheses and strings, each piece trimmed.
pub(super) fn top_level_split(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut q, mut start) = (0i32, false, 0);
    let mut esc = false;
    for (k, ch) in s.char_indices() {
        match ch {
            _ if esc => esc = false,
            '\\' if q => esc = true,
            '"' => q = !q,
            '(' if !q => depth += 1,
            ')' if !q => depth -= 1,
            ',' if !q && depth == 0 => {
                out.push(s[start..k].trim());
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}
