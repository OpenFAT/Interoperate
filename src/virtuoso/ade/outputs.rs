//! Output expressions both ways (spec `interop.md` §9.1, §9.2): Schemify calculator
//! expressions → ADE expressions ([`out_expr`]) and back (`out_from_ade`).

use super::*;

/// One-argument calculator functions with an ADE spelling, both ways: (Schemify, ADE). The
/// ADE ones are the waveform calculator's (*ADE Calculator* / *OCEAN Reference*): `real`
/// `imag` for `re` `im`, `phaseDeg` (degrees, as `ph`), `dB20` for `db`, `peakToPeak` for
/// `pp`, `average` for `avg`.
const OUT_FNS: [(&str, &str); 19] = [
    ("ymax", "ymax"),
    ("ymin", "ymin"),
    ("avg", "average"),
    ("rms", "rms"),
    ("integ", "integ"),
    ("deriv", "deriv"),
    ("abs", "abs"),
    ("sqrt", "sqrt"),
    ("exp", "exp"),
    ("ln", "ln"),
    ("log10", "log10"),
    ("re", "real"),
    ("im", "imag"),
    ("mag", "mag"),
    ("ph", "phaseDeg"),
    ("db", "dB20"),
    ("pp", "peakToPeak"),
    ("phase_margin", "phaseMargin"),
    ("gain_margin", "gainMargin"),
];

/// What the output translator knows about one instance path of the bench.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstInfo {
    /// The instance's Virtuoso path, segments joined by `/` (`DUT/M1` for SPICE's
    /// `XDUT.M1`).
    pub path: String,
    /// The Virtuoso terminal `i(dev)` reads: the exported master's first terminal (`p` of a
    /// `schemify_core` source or ammeter, `PLUS` on analogLib).
    pub terminal: String,
}

/// The bench facts an output translation reads.
pub struct OutScope<'a, F: Fn(&str) -> Option<InstInfo>> {
    /// The bench's vars (written `VAR("x")`).
    pub vars: &'a [String],
    /// The signal letter of `v()`/`i()`: `T` (tran), `F` (ac), `S` (dc sweep) or `DC` (§9.1).
    pub letter: &'a str,
    /// Instance path (segments joined by `/`) → its facts; `None` when the path names no
    /// instance (then `i()` reads terminal `PLUS` and `op()` the path as written).
    pub inst: F,
    /// The maestro test the output belongs to: `result(o, <it>)` is its dependent output
    /// `o`, read by name.
    pub test: &'a str,
}

/// The signal letter of an output's `analysis <a>` attribute: `tran` `T`, `ac` `F`, `dc` (a
/// sweep) `S`, `op` (the operating point) `DC`; `None` for another name or one the test
/// does not run (`op` needs a dc analysis with `saveOppoint`, `dc` one with a sweep).
pub(super) fn analysis_letter(a: &str, analyses: &[Analysis]) -> Option<&'static str> {
    let dc = analyses.iter().find(|x| x.0 == "dc");
    let has = |k: &str| dc.is_some_and(|d| d.2.iter().any(|o| o.0 == k));
    match a {
        "tran" | "ac" if analyses.iter().any(|x| x.0 == a) => {
            Some(if a == "tran" { "T" } else { "F" })
        }
        "dc" if has("sweep") => Some("S"),
        "op" if has("saveOppoint") => Some("DC"),
        _ => None,
    }
}

/// The ADE signal letter for `v()`/`i()` (§9.1): the first of tran (`T`), ac (`F`), a dc
/// sweep (`S`), an operating point (`DC`) the test runs; `T` without analyses.
pub(super) fn signal_letter(analyses: &[Analysis]) -> &'static str {
    let has = |n: &str| analyses.iter().find(|a| a.0 == n);
    if has("tran").is_some() {
        "T"
    } else if has("ac").is_some() {
        "F"
    } else if let Some(dc) = has("dc") {
        if dc.2.iter().any(|(k, _)| k == "sweep") {
            "S"
        } else {
            "DC"
        }
    } else {
        "T"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tk {
    Ws,
    Num,
    Ident,
    Str,
    Op,
}

/// Tokens `(kind, text)` of an expression in either syntax; `None` on a stray character or
/// an unterminated string.
fn out_tokens(s: &str) -> Option<Vec<(Tk, &str)>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut k = 0;
    while k < b.len() {
        let st = k;
        let c = b[k];
        let kind = if c.is_ascii_whitespace() {
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            Tk::Ws
        } else if c.is_ascii_digit() || (c == b'.' && b.get(k + 1).is_some_and(u8::is_ascii_digit))
        {
            while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'.') {
                k += 1;
            }
            if k < b.len() && (b[k] == b'e' || b[k] == b'E') {
                let mut j = k + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    k = j;
                    while k < b.len() && b[k].is_ascii_digit() {
                        k += 1;
                    }
                }
            }
            while k < b.len() && b[k].is_ascii_alphabetic() {
                k += 1;
            }
            Tk::Num
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while k < b.len()
                && (b[k].is_ascii_alphanumeric()
                    || b[k] == b'_'
                    || (b[k] == b'.'
                        && b.get(k + 1)
                            .is_some_and(|n| n.is_ascii_alphabetic() || *n == b'_')))
            {
                k += 1;
            }
            Tk::Ident
        } else if c == b'"' {
            k += 1;
            loop {
                match b.get(k)? {
                    b'"' => break,
                    b'\\' => k += 2,
                    _ => k += 1,
                }
            }
            k += 1;
            Tk::Str
        } else if s[k..].starts_with("**") {
            k += 2;
            Tk::Op
        } else if b"+-*/()^,%<>=!?:&|".contains(&c) {
            k += 1;
            Tk::Op
        } else {
            return None;
        };
        out.push((kind, &s[st..k]));
    }
    Some(out)
}

/// Index of the next non-space token at or after `k`.
fn next_tok(t: &[(Tk, &str)], mut k: usize) -> usize {
    while k < t.len() && t[k].0 == Tk::Ws {
        k += 1;
    }
    k
}

/// The single argument of the call whose `(` is at `k`: (argument token, index after `)`).
fn one_arg<'a>(t: &[(Tk, &'a str)], k: usize) -> Option<((Tk, &'a str), usize)> {
    if t.get(k)?.1 != "(" {
        return None;
    }
    let a = next_tok(t, k + 1);
    let close = next_tok(t, a + 1);
    (t.get(close)?.1 == ")").then(|| (t[a], close + 1))
}

/// Most nested calls an output expression may hold (bounds the recursion of [`emit`]).
const OUT_DEPTH_MAX: u32 = 64;

/// A calculator output expression as a maestro output (§9.1, the old three-argument form
/// kept for the tests): `i()` on terminal `PLUS`, no instance facts.
#[cfg(test)]
pub(super) fn out_to_ade(e: &str, vars: &[String], letter: &str) -> Option<String> {
    out_expr(
        e,
        &OutScope {
            vars,
            letter,
            inst: |_: &str| None,
            test: "",
        },
    )
}

/// A calculator output expression as an ADE (maestro) or OCEAN expression (§9.1 outputs
/// table): signals `v(n)`/`i(dev)` → `V<L>("/n")`/`I<L>("/dev/<terminal>")`, bench vars →
/// `VAR("x")`, `re im mag ph db` → `real imag mag phaseDeg dB20`, `at(y,x)` → `value(y x)`,
/// `x()` → `xval(<the expression's first signal>)`, `max`/`min` of two or more values →
/// the exact `(((a)+(b)±abs((a)-(b)))/2)` (the calculator's `max` is not element-wise),
/// `pow(a,b)` and `^` → `**`, `pi` → its value, `cross(y,l[,edge[,n]])` → `cross(y l n
/// "rising|falling|either")`, `settle(y,tol)` → `settlingTime` over the whole data,
/// `delay(y1,l1,y2,l2)` → the difference of the two crossings, then no group around a
/// single operand ([`ungroup`]),
/// `phase_margin`/`gain_margin` → `phaseMargin`/`gainMargin`, `op("M1","gm")` →
/// `OP("/M1" "gm")`. The same text serves maestro and OCEAN. The source's spacing is kept
/// outside rewritten calls, so a value needing no rewrite comes back byte for byte.
///
/// `None` when the expression uses anything else (strings outside `v`/`i`/`op`, other
/// functions, comparisons, `?:`, `x()` without a signal, nesting deeper than 64 calls):
/// the caller reports it and never writes a partial output.
pub fn out_expr<F: Fn(&str) -> Option<InstInfo>>(e: &str, s: &OutScope<'_, F>) -> Option<String> {
    let t = out_tokens(e)?;
    // `x()` is the x axis of the expression's own data: the first signal's.
    let first = (0..t.len()).find_map(|k| {
        let is_sig = t[k].0 == Tk::Ident && matches!(t[k].1, "v" | "V" | "i" | "I");
        let open = next_tok(&t, k + 1);
        (is_sig && t.get(open).is_some_and(|n| n.1 == "("))
            .then(|| signal(&t, k, open, s))
            .flatten()
    });
    let out = ungroup(&emit(
        &t,
        0,
        t.len(),
        s,
        first.as_ref().map(|x| x.0.as_str()),
        0,
    )?)?;
    debug_assert!(
        !out.contains(['^', ',']),
        "AEL has no `^`, SKILL lists no commas"
    );
    Some(out)
}

/// `e` (ADE text) without the grouping parentheses around a single operand: SKILL reads
/// `(x)` and `(f(y))` as calls of `x` and of the list `f(y)` (live OCEAN: `100*(0.01)` is
/// `eval: not a function 0.01`), so a group stays only around an operator (`(a+b)`, `(-1)`).
fn ungroup(e: &str) -> Option<String> {
    let t = out_tokens(e)?;
    let mut drop = vec![false; t.len()];
    for open in (0..t.len()).filter(|&k| t[k].1 == "(") {
        let prev = (0..open).rev().find(|&j| t[j].0 != Tk::Ws);
        if prev.is_some_and(|j| t[j].0 == Tk::Ident) {
            continue; // a call's argument list
        }
        let close = close_of(&t, open)?;
        let mut depth = 0u32;
        let lone = t[open + 1..close].iter().all(|tok| {
            match tok.1 {
                "(" => depth += 1,
                ")" => depth -= 1,
                _ => {}
            }
            depth > 0 || tok.1 == ")" || tok.0 != Tk::Op
        });
        if lone {
            drop[open] = true;
            drop[close] = true;
        }
    }
    Some(
        t.iter()
            .zip(&drop)
            .filter(|(_, d)| !**d)
            .map(|(tok, _)| tok.1)
            .collect(),
    )
}

/// The signal call `v(…)`/`i(…)` whose name is at `k` and `(` at `open`: (ADE text, index
/// after its `)`).
fn signal<F: Fn(&str) -> Option<InstInfo>>(
    t: &[(Tk, &str)],
    k: usize,
    open: usize,
    s: &OutScope<'_, F>,
) -> Option<(String, usize)> {
    let ((ak, a), after) = one_arg(t, open)?;
    let name = match ak {
        Tk::Ident => a.to_owned(),
        Tk::Str if !a.contains('\\') => a[1..a.len() - 1].to_owned(),
        _ => return None,
    };
    let path = name.replace('.', "/");
    let letter = s.letter;
    let bus = |p: &str| p.replace('[', "<").replace(']', ">");
    let text = if t[k].1.eq_ignore_ascii_case("v") {
        // A net inside a subcircuit: its instance prefix through the bench's names.
        let p = match path.rsplit_once('/') {
            Some((insts, net)) => match (s.inst)(insts) {
                Some(i) => format!("{}/{net}", i.path),
                None => path.clone(),
            },
            None => path.clone(),
        };
        format!("V{letter}(\"/{}\")", bus(&p))
    } else {
        let (p, term) = (s.inst)(&path).map_or_else(
            || (path.clone(), "PLUS".to_owned()),
            |i| (i.path, i.terminal),
        );
        format!("I{letter}(\"/{}/{term}\")", bus(&p))
    };
    Some((text, after))
}

/// Index of the `)` matching the `(` at `open`.
fn close_of(t: &[(Tk, &str)], open: usize) -> Option<usize> {
    let mut depth = 0u32;
    for (k, tok) in t.iter().enumerate().skip(open) {
        match tok.1 {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// The argument token ranges of the call whose `(` is at `open` and `)` at `close`, each
/// trimmed of white space; `[]` for `()`. `None` for an empty argument.
fn call_args(t: &[(Tk, &str)], open: usize, close: usize) -> Option<Vec<(usize, usize)>> {
    let trim = |mut a: usize, mut b: usize| {
        while a < b && t[a].0 == Tk::Ws {
            a += 1;
        }
        while b > a && t[b - 1].0 == Tk::Ws {
            b -= 1;
        }
        (a < b).then_some((a, b))
    };
    let mut out = Vec::new();
    let (mut depth, mut start) = (0u32, open + 1);
    for (k, tok) in t.iter().enumerate().take(close).skip(open + 1) {
        match tok.1 {
            "(" => depth += 1,
            ")" => depth -= 1,
            "," if depth == 0 => {
                out.push(trim(start, k)?);
                start = k + 1;
            }
            _ => {}
        }
    }
    match trim(start, close) {
        Some(r) => out.push(r),
        None if out.is_empty() => {}
        None => return None,
    }
    Some(out)
}

/// The text of a string or bare-identifier argument (`"M1"`, `M1`).
fn word_arg<'a>(t: &[(Tk, &'a str)], r: (usize, usize)) -> Option<&'a str> {
    if r.1 != r.0 + 1 {
        return None;
    }
    match t[r.0] {
        (Tk::Ident, x) => Some(x),
        (Tk::Str, x) if !x.contains('\\') => Some(&x[1..x.len() - 1]),
        _ => None,
    }
}

/// Emits tokens `lo..hi` (see [`out_expr`]); `first` is the expression's first signal.
fn emit<F: Fn(&str) -> Option<InstInfo>>(
    t: &[(Tk, &str)],
    lo: usize,
    hi: usize,
    s: &OutScope<'_, F>,
    first: Option<&str>,
    depth: u32,
) -> Option<String> {
    if depth >= OUT_DEPTH_MAX {
        return None;
    }
    assert!(lo <= hi && hi <= t.len());
    let mut out = String::new();
    let mut k = lo;
    while k < hi {
        let (kind, x) = t[k];
        match kind {
            Tk::Ws => out.push_str(x),
            Tk::Num => out.push_str(&ael::number_to_ael(x)?),
            Tk::Str => return None,
            Tk::Op => match x {
                "+" | "-" | "*" | "/" | "(" | ")" => out.push_str(x),
                "^" => out.push_str("**"),
                _ => return None,
            },
            Tk::Ident => {}
        }
        if kind != Tk::Ident {
            k += 1;
            continue;
        }
        let open = next_tok(t, k + 1);
        if !(open < hi && t[open].1 == "(") {
            if s.vars.iter().any(|v| v == x) {
                out.push_str(&format!("VAR(\"{x}\")"));
            } else if x == "pi" {
                write_num(&mut out, std::f64::consts::PI);
            } else {
                return None;
            }
            k += 1;
            continue;
        }
        if matches!(x, "v" | "V" | "i" | "I") {
            let (text, after) = signal(t, k, open, s)?;
            out.push_str(&text);
            k = after;
            continue;
        }
        let close = close_of(t, open).filter(|c| *c < hi)?;
        let args = call_args(t, open, close)?;
        let arg = |r: (usize, usize)| emit(t, r.0, r.1, s, first, depth + 1);
        match (x, args.as_slice()) {
            ("x", []) => out.push_str(&format!("xval({})", first?)),
            ("at", [y, x0]) => {
                let (y, x0) = (arg(*y)?, arg(*x0)?);
                out.push_str(&format!("value({y} {x0})"));
            }
            ("pow", [a, b]) => {
                let (a, b) = (arg(*a)?, arg(*b)?);
                out.push_str(&format!("({a})**({b})"));
            }
            ("max" | "min", [a, rest @ ..]) if !rest.is_empty() => {
                let sign = if x == "max" { '+' } else { '-' };
                let mut acc = arg(*a)?;
                for b in rest {
                    let b = arg(*b)?;
                    // Grouped whole: `1/max(a,b)` divides by the maximum, not by its sum.
                    acc = format!("((({acc})+({b}){sign}abs(({acc})-({b})))/2)");
                }
                out.push_str(&acc);
            }
            // Another output: by name in its own test (a dependent output), else calcVal.
            ("result", [o, test]) => {
                let (o, test) = (word_arg(t, *o)?, word_arg(t, *test)?);
                if !is_ident(o) || test.is_empty() {
                    return None;
                }
                if test == s.test {
                    out.push_str(o);
                } else {
                    out.push_str(&format!("calcVal(\"{o}\" \"{test}\")"));
                }
            }
            ("op", [inst, param]) => {
                let (inst, param) = (word_arg(t, *inst)?, word_arg(t, *param)?);
                let segs: Vec<&str> = inst.strip_prefix('/').unwrap_or(inst).split('/').collect();
                if param.is_empty() || segs.iter().any(|g| g.is_empty()) {
                    return None;
                }
                // The longest prefix naming instances; the rest names devices inside
                // the placed PDK subcircuit, as written.
                let (info, rest) = (1..=segs.len())
                    .rev()
                    .find_map(|n| (s.inst)(&segs[..n].join("/")).map(|i| (i, &segs[n..])))
                    .unwrap_or_else(|| {
                        let i = InstInfo {
                            path: segs.join("/"),
                            ..InstInfo::default()
                        };
                        (i, &segs[segs.len()..])
                    });
                let mut path: Vec<&str> = info.path.split('/').collect();
                path.extend(rest.iter().copied());
                out.push_str(&format!("OP(\"/{}\" \"{param}\")", path.join("/")));
            }
            ("cross", [y, level, more @ ..]) if more.len() <= 2 => {
                let edge = match more
                    .first()
                    .map(|r| arg(*r))
                    .map(|e| e.ok_or(()))
                    .transpose()
                    .ok()?
                    .as_deref()
                {
                    None | Some("0") => "either",
                    Some("1") => "rising",
                    Some("-1") => "falling",
                    Some(_) => return None,
                };
                let n = match more.get(1) {
                    None => "1".to_owned(),
                    Some(r) => arg(*r).filter(|n| n.parse::<u32>().is_ok_and(|n| n > 0))?,
                };
                let (y, level) = (arg(*y)?, arg(*level)?);
                out.push_str(&format!("cross({y} {level} {n} \"{edge}\")"));
            }
            // The second signal's first crossing of its level minus the first's (expr.md),
            // as two calculator crossings: exact where ADE's `delay` adds periods and
            // thresholds of its own.
            ("delay", [y1, l1, y2, l2]) => {
                let (y1, l1, y2, l2) = (arg(*y1)?, arg(*l1)?, arg(*y2)?, arg(*l2)?);
                out.push_str(&format!(
                    "(cross({y2} {l2} 1 \"either\")-cross({y1} {l1} 1 \"either\"))"
                ));
            }
            ("settle", [y, tol]) => {
                // Absolute tolerance → settlingTime's percent of the whole step,
                // from the first to the last point of the data.
                let (y, tol) = (arg(*y)?, arg(*tol)?);
                let (x0, x1) = (format!("ymin(xval({y}))"), format!("ymax(xval({y}))"));
                out.push_str(&format!(
                    "settlingTime({y} {x0} t {x1} t \
                             100*({tol})/abs(value({y} {x1})-value({y} {x0})))"
                ));
            }
            (_, [a]) => {
                let f = OUT_FNS.iter().find(|f| f.0 == x)?.1;
                out.push_str(&format!("{f}({})", arg(*a)?));
            }
            _ => return None,
        }
        k = close + 1;
    }
    Some(out)
}

/// An ADE output expression as a calculator expression (the inverse of [`out_to_ade`]);
/// `None` when it uses anything else or does not compile.
/// A number as ADE stores it, 7 significant digits, in engineering text (`632.1206m`, `1m`);
/// any other text unchanged.
fn ade_number(t: &str) -> String {
    match schemify_expr::parse_num(t.trim()) {
        Ok(x) if x.is_finite() => {
            let r: f64 = format!("{x:.6e}").parse().unwrap_or(x);
            let mut s = String::new();
            schemify_expr::write_num(&mut s, r);
            crate::virtuoso::read::si_text(&s)
        }
        _ => t.to_owned(),
    }
}

/// `value(y x)`, `cross(y l n "edge")`, `xval(y)`, `OP("/path" "param")` or `calcVal("o"
/// "T")`, the call of ADE function `f` between tokens `open` and `close` of `e`, as the
/// calculator expression export turned into it (`own`: see [`out_from_ade_in`]).
fn ade_call_back(
    e: &str,
    t: &[(Tk, &str)],
    f: &str,
    open: usize,
    close: usize,
    own: (&str, &[&str]),
) -> Option<String> {
    let back = |x: &str| out_from_ade_in(x, own.0, own.1);
    // Arguments are separated by white space at depth 0 between two operands (`a - b`
    // stays one argument).
    let offset = |s: &str| s.as_ptr() as usize - e.as_ptr() as usize;
    let mut args: Vec<&str> = Vec::new();
    let mut depth = 0u32;
    let mut start: Option<usize> = None;
    let mut last: Option<(Tk, &str)> = None;
    for k in open + 1..close {
        let (kind, s) = t[k];
        match kind {
            Tk::Ws if depth == 0 => {
                let next = t[next_tok(t, k)..close].first().copied();
                let op = |x: Option<(Tk, &str)>| {
                    x.is_some_and(|(k, s)| k == Tk::Op && !matches!(s, "(" | ")" | "?"))
                };
                if let Some(a) = start.filter(|_| !op(last) && !op(next)) {
                    args.push(&e[a..offset(s)]);
                    start = None;
                }
                continue;
            }
            Tk::Op if s == "(" => depth += 1,
            Tk::Op if s == ")" => depth = depth.checked_sub(1)?,
            _ => {}
        }
        start.get_or_insert(offset(s));
        if kind != Tk::Ws {
            last = Some((kind, s));
        }
    }
    if let Some(a) = start {
        args.push(e[a..offset(t[close].1)].trim_end());
    }
    let unquote = |s: &str| {
        s.strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .filter(|s| !s.contains(['"', '\\']))
            .map(str::to_owned)
    };
    Some(match (f, args.as_slice()) {
        ("value", [y, x0]) => format!("at({}, {})", back(y)?, back(x0)?),
        ("xval", [_]) => "x()".to_owned(),
        // `settle`'s own form (export): the step from the first to the last x, the
        // tolerance as a percent of it.
        // ICADVM20.1 answers it parsed and printed, `((100 * tol) / abs((value(…) -
        // value(…))))` (live): compared without white space and outer groups.
        ("settlingTime", [y, x0, a, x1, b, pct]) => {
            let (lo, hi) = (format!("ymin(xval({y}))"), format!("ymax(xval({y}))"));
            if (*x0, *a, *x1, *b) != (lo.as_str(), "t", hi.as_str(), "t") {
                return None;
            }
            // ponytail: white space goes from the tolerance too; one with a
            // space-separated call (`value(x 1)`) does not read back.
            let bare = |s: &str| {
                let mut s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                while s.starts_with('(')
                    && close_of(&out_tokens(&s)?, 0) == Some(out_tokens(&s)?.len() - 1)
                {
                    s = s[1..s.len() - 1].to_owned();
                }
                Some(s)
            };
            let p = bare(pct)?;
            let tail = format!(
                "/abs({})",
                bare(&format!("value({y} {hi})-value({y} {lo})"))?
            );
            let tail2 = format!(
                "/abs(({}))",
                bare(&format!("value({y} {hi})-value({y} {lo})"))?
            );
            let head = p
                .strip_suffix(tail.as_str())
                .or_else(|| p.strip_suffix(tail2.as_str()))?;
            let tol = bare(head)?;
            let tol = tol.strip_prefix("100*")?;
            format!("settle({}, {})", back(y)?, back(tol)?)
        }
        // `delay` with every edge given: the difference of the two crossings (live OCEAN:
        // equal to cross(…)-cross(…) on an RC step).
        ("delay", kw) if kw.len() == 16 => {
            let arg = |k: &str| {
                kw.chunks(2)
                    .find(|p| p[0] == k)
                    .map(|p| p[1])
                    .filter(|_| kw.chunks(2).all(|p| p[0].starts_with('?')))
            };
            let edge = |k: &str| match unquote(arg(k)?)?.as_str() {
                "either" => Some("0"),
                "rising" => Some("1"),
                "falling" => Some("-1"),
                _ => None,
            };
            let nth = |k: &str| arg(k).filter(|n| n.parse::<u32>().is_ok_and(|n| n > 0));
            format!(
                "cross({}, {}, {}, {})-cross({}, {}, {}, {})",
                back(arg("?wf2")?)?,
                back(arg("?value2")?)?,
                edge("?edge2")?,
                nth("?nth2")?,
                back(arg("?wf1")?)?,
                back(arg("?value1")?)?,
                edge("?edge1")?,
                nth("?nth1")?
            )
        }
        ("cross", [y, l, n, edge]) => {
            let e = match unquote(edge)?.as_str() {
                "either" => "0",
                "rising" => "1",
                "falling" => "-1",
                _ => return None,
            };
            let (y, l) = (back(y)?, back(l)?);
            match (*n, e) {
                ("1", "0") => format!("cross({y}, {l})"),
                ("1", _) => format!("cross({y}, {l}, {e})"),
                (n, _) if n.parse::<u32>().is_ok_and(|n| n > 0) => {
                    format!("cross({y}, {l}, {e}, {n})")
                }
                _ => return None,
            }
        }
        ("OP", [path, param]) => {
            let (path, param) = (unquote(path)?, unquote(param)?);
            format!("op(\"{}\", \"{param}\")", path.strip_prefix('/')?)
        }
        // Keywords (`?historyName`, `?cornerName`) have no Schemify form.
        ("calcVal", [o, test]) => {
            let (o, test) = (unquote(o)?, unquote(test)?);
            (is_ident(&o) && !test.is_empty() && !test.contains(['[', ']']))
                .then(|| format!("result(\"{o}\", \"{test}\")"))?
        }
        _ => return None,
    })
}

#[cfg(test)]
pub(super) fn out_from_ade(e: &str) -> Option<String> {
    out_from_ade_in(e, "", &[])
}

/// [`out_from_ade`] for an output of maestro test `test`, whose outputs are `outs`: a bare
/// name of one of them (a dependent output) reads as `result("<name>", "<test>")`
/// (expr.md §5).
pub(super) fn out_from_ade_in(e: &str, test: &str, outs: &[&str]) -> Option<String> {
    let t = out_tokens(e)?;
    let mut out = String::with_capacity(e.len());
    let mut k = 0;
    while k < t.len() {
        let (kind, x) = t[k];
        match kind {
            Tk::Ws => out.push_str(x),
            // ADE keeps 7 significant digits (live run: 0.6321205588285577 came back
            // 0.6321206, 1m as 0.001); both sides of the stash comparison read numbers so.
            Tk::Num => out.push_str(&ade_number(
                &ael::from_ael(x, |_| AelName::Unknown, |_| None)?.text,
            )),
            Tk::Str => return None,
            Tk::Op => match x {
                "+" | "-" | "*" | "/" | "(" | ")" => out.push_str(x),
                "**" => out.push('^'),
                _ => return None,
            },
            Tk::Ident => {
                let open = next_tok(&t, k + 1);
                let sig = matches!(x, "VT" | "VF" | "VS" | "VDC" | "IT" | "IF" | "IS" | "IDC");
                if sig || x == "VAR" {
                    let ((ak, a), after) = one_arg(&t, open)?;
                    if ak != Tk::Str || a.contains('\\') {
                        return None;
                    }
                    let a = &a[1..a.len() - 1];
                    if x == "VAR" {
                        if !is_ident(a) {
                            return None;
                        }
                        out.push_str(a);
                    } else {
                        let p = a.strip_prefix('/')?;
                        let p = if x.starts_with('I') {
                            p.strip_suffix("/PLUS").or_else(|| p.strip_suffix("/p"))?
                        } else {
                            p
                        };
                        let name = p.replace('/', ".").replace('<', "[").replace('>', "]");
                        let bare = !name.is_empty() && name.split('.').all(is_ident);
                        let f = if x.starts_with('V') { 'v' } else { 'i' };
                        if bare {
                            out.push_str(&format!("{f}({name})"));
                        } else {
                            out.push_str(&format!("{f}(\"{name}\")"));
                        }
                    }
                    k = after;
                    continue;
                }
                // The calls export writes with several space-separated arguments, back to
                // their calculator form (§9.1 outputs table read backwards).
                if matches!(
                    x,
                    "value" | "cross" | "xval" | "OP" | "calcVal" | "settlingTime" | "delay"
                ) && t.get(open).is_some_and(|n| n.1 == "(")
                {
                    let close = close_of(&t, open)?;
                    out.push_str(&ade_call_back(e, &t, x, open, close, (test, outs))?);
                    k = close + 1;
                    continue;
                }
                if t.get(open).is_some_and(|n| n.1 == "(") {
                    out.push_str(OUT_FNS.iter().find(|f| f.1 == x)?.0);
                } else if outs.contains(&x) && !test.is_empty() {
                    out.push_str(&format!("result(\"{x}\", \"{test}\")"));
                } else {
                    return None;
                }
            }
        }
        k += 1;
    }
    Program::compile(&out, Mode::Calc).ok().map(|_| out)
}
