//! Values, expressions and templates in xschem spelling (spec `interop.md` §3.4 xschem paragraph, §3.5; `import.md` §4 inverted).

use super::*;

// ---------------------------------------------------------------------------------------
// Values and expressions (§3.4, xschem paragraph)
// ---------------------------------------------------------------------------------------

/// One token of a Schemify expression, as far as translation needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tok<'a> {
    Num(&'a str),
    /// An identifier, possibly namespaced (`ns.name`).
    Ident(&'a str),
    /// One of `+ - * / ^ ( ) ,`.
    Op(&'a str),
}

/// Splits `s` into tokens; `None` for anything outside numbers, identifiers and the
/// arithmetic operators (strings, comparisons, `?:`, `&&`, …): no xschem spelling.
pub(super) fn lex(s: &str) -> Option<Vec<Tok<'_>>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let run = |mut i: usize, f: fn(u8) -> bool| {
        while i < b.len() && f(b[i]) {
            i += 1;
        }
        i
    };
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i = run(i, |c| c.is_ascii_digit());
            if b.get(i) == Some(&b'.') {
                i = run(i + 1, |c| c.is_ascii_digit());
            }
            if matches!(b.get(i), Some(b'e' | b'E')) {
                let j = i + 1 + usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
                if b.get(j).is_some_and(u8::is_ascii_digit) {
                    i = run(j, |c| c.is_ascii_digit());
                }
            }
            i = run(i, |c| c.is_ascii_alphabetic());
            out.push(Tok::Num(&s[start..i]));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            i = run(i, |c| c.is_ascii_alphanumeric() || c == b'_');
            if b.get(i) == Some(&b'.')
                && b.get(i + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            {
                i = run(i + 1, |c| c.is_ascii_alphanumeric() || c == b'_');
            }
            out.push(Tok::Ident(&s[start..i]));
        } else if b"+-*/^(),".contains(&c) {
            i += 1;
            out.push(Tok::Op(&s[start..i]));
        } else {
            return None;
        }
    }
    Some(out)
}

/// How names resolve inside one cell when its expressions are exported.
pub(super) struct Scope {
    /// Names the netlist resolves (cell parameters, bench vars): written as they are.
    pub(super) keep: Vec<String>,
    /// Names xschem has no place for (a component's vars, computed params): their
    /// definition is inlined in parentheses; `None` when it does not translate.
    pub(super) inline: Vec<(String, Option<String>)>,
}

impl Scope {
    /// The scope of `c`: a bench keeps its vars (they become `.param`s), a component or
    /// device inlines them, using a plugin-call var's cached value (noted).
    pub(super) fn of(c: &Cell, notes: &mut Notes) -> Scope {
        let bench = c.kind() == FileKind::Shbench;
        let mut sc = Scope {
            keep: Vec::new(),
            inline: Vec::new(),
        };
        for p in params(c) {
            match p.computed {
                Some(e) => sc.inline.push((p.name.to_owned(), Some(e.to_owned()))),
                None => sc.keep.push(p.name.to_owned()),
            }
        }
        for r in c.records(RecordKind::Var) {
            let (name, expr) = (c.field(r, 0), c.field(r, 1));
            if bench {
                sc.keep.push(name.to_owned());
                continue;
            }
            let plugin = lex(expr).is_none_or(|t| {
                t.iter()
                    .any(|t| matches!(t, Tok::Ident(x) if x.contains('.')))
            });
            let def = if plugin {
                let cached = c
                    .doc
                    .find_attr(r, "cached")
                    .and_then(|a| c.doc.attr_value(a, 0));
                if let Some(v) = cached {
                    notes.review(
                        "expression",
                        &c.path,
                        format!("var {name}: inlined as its cached value {v}"),
                    );
                }
                cached.map(str::to_owned)
            } else {
                Some(expr.to_owned())
            };
            sc.inline.push((name.to_owned(), def));
        }
        sc
    }
}

/// `src` with every inlined name of `sc` replaced by its definition in parentheses.
pub(super) fn inline(src: &str, sc: &Scope, depth: u32) -> Result<String, &'static str> {
    if depth > INLINE_DEPTH_MAX {
        return Err("inlined-var");
    }
    let Some(toks) = lex(src) else {
        return Ok(src.to_owned()); // fails translation later, with the right reason
    };
    if !toks
        .iter()
        .any(|t| matches!(t, Tok::Ident(x) if sc.inline.iter().any(|(n, _)| n == x)))
    {
        return Ok(src.to_owned());
    }
    let mut out = String::new();
    for (k, t) in toks.iter().enumerate() {
        match *t {
            Tok::Ident(x)
                if toks.get(k + 1) != Some(&Tok::Op("(")) && !sc.keep.iter().any(|n| n == x) =>
            {
                match sc.inline.iter().find(|(n, _)| n == x) {
                    Some((_, Some(def))) => {
                        let def = inline(def, sc, depth + 1)?;
                        if matches!(lex(&def).as_deref(), Some([Tok::Num(_)])) {
                            out.push_str(def.trim());
                        } else {
                            out.push('(');
                            out.push_str(&def);
                            out.push(')');
                        }
                    }
                    Some((_, None)) => return Err("expression"),
                    None => out.push_str(x),
                }
            }
            Tok::Num(s) | Tok::Ident(s) | Tok::Op(s) => out.push_str(s),
        }
    }
    Ok(out)
}

/// A number expression as a SPICE value: a number literal as written (Schemify's suffixes
/// are SPICE's), anything else as a SPICE expression with `^` → `**`, quoted `'…'` unless
/// `bare` (the format quotes it: a raw parameter, see [`FmtCtx::raw`]).
///
/// # Errors
///
/// The `needs_review` reason: `expression` for strings, conditionals, comparisons,
/// namespaced names and functions SPICE lacks.
pub(super) fn spice_value(src: &str, sc: &Scope, bare: bool) -> Result<String, &'static str> {
    let e = inline(src.trim(), sc, 0)?;
    let toks = lex(&e).ok_or("expression")?;
    match toks.as_slice() {
        [] => return Err("expression"),
        [Tok::Num(_)] | [Tok::Op("-"), Tok::Num(_)] => return Ok(e.trim().to_owned()),
        _ => {}
    }
    let q = if bare { "" } else { "'" };
    let mut out = String::from(q);
    for (k, t) in toks.iter().enumerate() {
        match *t {
            Tok::Ident(x) => {
                let call = toks.get(k + 1) == Some(&Tok::Op("("));
                if x.contains('.')
                    || matches!(x, "true" | "false")
                    || (call && !SPICE_FUNCS.contains(&x))
                {
                    return Err("expression");
                }
                out.push_str(x);
            }
            Tok::Op("^") => out.push_str("**"),
            Tok::Num(s) | Tok::Op(s) => out.push_str(s),
        }
    }
    out.push_str(q);
    Ok(out)
}

/// A string-kind value (a template): literal text kept, a hole naming one identifier
/// written as that bare name (the inverse of the importer's parent-parameter holes).
pub(super) fn string_value(src: &str) -> Result<String, &'static str> {
    let mut out = String::new();
    for piece in split_template(src).ok_or("template")? {
        match piece {
            Piece::Lit(s) => out.push_str(&s),
            Piece::Hole(h) if is_ident(h.trim()) => out.push_str(h.trim()),
            Piece::Hole(_) => return Err("template"),
        }
    }
    Ok(out)
}

/// A value of a parameter of kind `kind` in xschem spelling; `raw` for a parameter its
/// symbol's format quotes itself ([`FmtCtx::raw`]).
pub(super) fn xschem_value(
    kind: &str,
    src: &str,
    sc: &Scope,
    raw: bool,
) -> Result<String, &'static str> {
    match kind {
        "string" | "choice" => string_value(src),
        "bool" => match src.trim() {
            v @ ("true" | "false") => Ok(v.to_owned()),
            _ => Err("expression"),
        },
        _ => spice_value(src, sc, raw),
    }
}

pub(super) fn is_ident(s: &str) -> bool {
    let mut b = s.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

// ---------------------------------------------------------------------------------------
// Templates (§3.5, import.md §4 inverted)
// ---------------------------------------------------------------------------------------

/// A piece of a Schemify template.
pub(super) enum Piece {
    /// Literal text, `{{`/`}}` resolved.
    Lit(String),
    /// A hole's expression text.
    Hole(String),
}

/// Splits a template (expr.md §6); `None` for an unclosed `{` or a lone `}`.
pub(super) fn split_template(s: &str) -> Option<Vec<Piece>> {
    let b = s.as_bytes();
    let (mut out, mut lit) = (Vec::new(), String::new());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'{', Some(b'{')) | (b'}', Some(b'}')) => {
                lit.push(b[i] as char);
                i += 2;
            }
            (b'{', _) => {
                let mut j = i + 1;
                let mut in_str = false;
                while j < b.len() && (in_str || b[j] != b'}') {
                    match b[j] {
                        b'\\' if in_str => j += 1,
                        b'"' => in_str = !in_str,
                        _ => {}
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return None;
                }
                if !lit.is_empty() {
                    out.push(Piece::Lit(std::mem::take(&mut lit)));
                }
                out.push(Piece::Hole(s[i + 1..j].to_owned()));
                i = j + 1;
            }
            (b'}', _) => return None,
            _ => {
                let ch = s[i..].chars().next().expect("in bounds");
                lit.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    Some(out)
}

/// What a format template's identifiers may name.
pub(super) struct FmtCtx<'a> {
    /// Ordinary pins: Schemify name → xschem name.
    pub(super) pins: Vec<(&'a str, &'a str)>,
    /// Inherited pins (xschem `extra` nets).
    pub(super) inherited: Vec<&'a str>,
    /// Overridable parameters.
    pub(super) params: Vec<&'a str>,
    /// Computed parameters and their expressions.
    pub(super) computed: Vec<(&'a str, &'a str)>,
    /// The impl's `model` attribute.
    pub(super) model: Option<&'a str>,
    /// Parameters a format uses as numbers (a hole with a glued scale suffix, `{W}u`, or
    /// inside an arithmetic hole): every hole of one is written as the SPICE expression
    /// `'(@W)'` and its values unquoted, so a parametric value (`wn*2`) still netlists
    /// (xschem pastes values as text: `'wn'u` would not).
    pub(super) raw: Vec<&'a str>,
}

/// The format context of cell `c` with pins `pins` (its [`sym_pins`]).
pub(super) fn fmt_ctx<'a>(c: &'a Cell, pins: &'a SymPins) -> FmtCtx<'a> {
    let ps = params(c);
    let imp = default_impl(c);
    // Pins: xschem shorts same-named pins, so an alias pin takes its target's name.
    let mut ctx = FmtCtx {
        pins: (0..pins.name.len())
            .filter(|&p| pins.inherit[p].is_none())
            .map(|p| {
                (
                    pins.name[p].as_str(),
                    pins.alias[p].as_deref().unwrap_or(&pins.name[p]),
                )
            })
            .collect(),
        inherited: (0..pins.name.len())
            .filter(|&p| pins.inherit[p].is_some())
            .map(|p| pins.name[p].as_str())
            .collect(),
        params: ps
            .iter()
            .filter(|p| p.computed.is_none())
            .map(|p| p.name)
            .collect(),
        computed: ps
            .iter()
            .filter_map(|p| Some((p.name, p.computed?)))
            .collect(),
        model: imp.and_then(|r| c.attr(r, "model")),
        raw: Vec::new(),
    };
    let mut raw = Vec::new();
    for (_, tpl) in imp.map(|r| formats(c, r)).unwrap_or_default() {
        let pieces = split_template(tpl).unwrap_or_default();
        for (k, piece) in pieces.iter().enumerate() {
            let Piece::Hole(h) = piece else { continue };
            let h = h.trim();
            let glued = matches!(pieces.get(k + 1), Some(Piece::Lit(l)) if glued_suffix(l) > 0);
            if let Some(p) = ctx.params.iter().find(|p| **p == h) {
                if glued {
                    raw.push(*p);
                }
            } else if !is_special(h, &ctx) {
                mark_raw(h, &ctx, &mut raw, 0);
            }
        }
    }
    raw.sort_unstable();
    raw.dedup();
    assert!(
        raw.iter().all(|r| ctx.params.contains(r)),
        "raw params are params"
    );
    ctx.raw = raw;
    ctx
}

/// True for holes `convert_format` spells as a token of their own (builtins, pins, the
/// `savecurrent` idiom), never as a number.
pub(super) fn is_special(h: &str, ctx: &FmtCtx) -> bool {
    matches!(h, "name" | "cell" | "pins" | "params" | "file")
        || (h == "model" && ctx.model.is_some())
        || ctx.pins.iter().any(|(n, _)| *n == h)
        || ctx.inherited.contains(&h)
        || (h.starts_with("savecurrent") && h.contains('?'))
}

/// Adds every parameter arithmetic hole `h` reads (through computed params) to `raw`.
pub(super) fn mark_raw<'a>(h: &str, ctx: &FmtCtx<'a>, raw: &mut Vec<&'a str>, depth: u32) {
    let Some(toks) = lex(h) else { return };
    for t in toks {
        let Tok::Ident(x) = t else { continue };
        if let Some(p) = ctx.params.iter().find(|p| **p == x) {
            raw.push(p);
        } else if let Some((_, e)) = ctx.computed.iter().find(|(n, _)| *n == x)
            && depth < INLINE_DEPTH_MAX
        {
            mark_raw(e, ctx, raw, depth + 1);
        }
    }
}

/// Length of the SPICE scale suffix `lit` starts with (`u` after `{W}` in `{W}u`), else 0.
pub(super) fn glued_suffix(lit: &str) -> usize {
    let n = lit.bytes().take_while(u8::is_ascii_alphabetic).count();
    let word = lit[..n].to_ascii_lowercase();
    if matches!(
        word.as_str(),
        "f" | "p" | "n" | "u" | "m" | "k" | "meg" | "g" | "t" | "mil"
    ) {
        n
    } else {
        0
    }
}

/// Output of a template conversion: an xschem `@token` or literal text.
pub(super) enum Out {
    Tok(String),
    Lit(String),
}

/// Writes token-layer text: `@tok`, and literal text with `@ % \` escaped; a literal that
/// directly follows a token is escaped too, since only white space or the next `@`/`%`
/// ends a token (`@W\u` is `@W` then `u`).
pub(super) fn render(pieces: &[Out]) -> String {
    let mut out = String::new();
    let mut after_tok = false;
    for p in pieces {
        match p {
            Out::Tok(t) => {
                out.push('@');
                out.push_str(t);
                after_tok = true;
            }
            Out::Lit(s) => {
                for (k, ch) in s.chars().enumerate() {
                    let ends_tok = k == 0 && after_tok && !ch.is_whitespace();
                    if ends_tok || matches!(ch, '@' | '%' | '\\') {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                after_tok &= s.is_empty();
            }
        }
    }
    out
}

/// A netlist template as an xschem format: `{name}` → `@name`, `{cell}` → `@symname`,
/// `{pins}` → `@pinlist` (then the inherited pins), a pin → `@@pin`, an inherited pin or a
/// parameter → `@p`, the `savecurrent` idiom → `@savecurrent`; a raw parameter
/// ([`FmtCtx::raw`]) and any other arithmetic hole → a SPICE expression `'(@W)*1u'`
/// (computed params inlined, a glued scale suffix folded in).
///
/// # Errors
///
/// Why the template has no xschem spelling.
pub(super) fn convert_format(tpl: &str, ctx: &FmtCtx) -> Result<String, String> {
    let pieces = split_template(tpl).ok_or("not a template")?;
    let mut out = Vec::new();
    let mut skip = 0; // suffix bytes of the next literal folded into the previous hole
    for (k, piece) in pieces.iter().enumerate() {
        let h = match piece {
            Piece::Lit(s) => {
                out.push(Out::Lit(s[std::mem::take(&mut skip)..].to_owned()));
                continue;
            }
            Piece::Hole(h) => h.trim(),
        };
        match h {
            "name" => out.push(Out::Tok("name".into())),
            "cell" => out.push(Out::Tok("symname".into())),
            "pins" => {
                out.push(Out::Tok("pinlist".into()));
                for p in &ctx.inherited {
                    out.push(Out::Lit(" ".into()));
                    out.push(Out::Tok((*p).into()));
                }
            }
            "model" if ctx.model.is_some() => out.push(Out::Lit(ctx.model.unwrap_or("").into())),
            "params" | "file" => return Err(format!("{{{h}}} has no xschem token")),
            _ if let Some((_, x)) = ctx.pins.iter().find(|(n, _)| *n == h) => {
                out.push(Out::Tok(format!("@{x}")));
            }
            _ if ctx.inherited.contains(&h) => out.push(Out::Tok(h.into())),
            _ if ctx.params.contains(&h) && !ctx.raw.contains(&h) => out.push(Out::Tok(h.into())),
            _ if h.starts_with("savecurrent")
                && h.contains('?')
                && ctx.params.contains(&"savecurrent") =>
            {
                out.push(Out::Tok("savecurrent".into()));
            }
            _ => {
                out.push(Out::Lit("'".into()));
                spice_pieces(h, ctx, &mut out, 0)?;
                if let Some(Piece::Lit(l)) = pieces.get(k + 1) {
                    skip = glued_suffix(l);
                    if skip > 0 {
                        out.push(Out::Lit(format!("*1{}", &l[..skip])));
                    }
                }
                out.push(Out::Lit("'".into()));
            }
        }
    }
    Ok(render(&out))
}

/// An arithmetic hole as SPICE expression pieces, every parameter as `(@p)`.
pub(super) fn spice_pieces(
    h: &str,
    ctx: &FmtCtx,
    out: &mut Vec<Out>,
    depth: u32,
) -> Result<(), String> {
    if depth > INLINE_DEPTH_MAX {
        return Err("computed parameters nest too deep".into());
    }
    let toks = lex(h).ok_or_else(|| format!("{{{h}}}: not arithmetic"))?;
    for (k, t) in toks.iter().enumerate() {
        match *t {
            Tok::Ident(x) if toks.get(k + 1) == Some(&Tok::Op("(")) => {
                if !SPICE_FUNCS.contains(&x) {
                    return Err(format!("{x}(): no SPICE function"));
                }
                out.push(Out::Lit(x.into()));
            }
            Tok::Ident(x) if ctx.params.contains(&x) => {
                out.push(Out::Lit("(".into()));
                out.push(Out::Tok(x.into()));
                out.push(Out::Lit(")".into()));
            }
            Tok::Ident(x) => match ctx.computed.iter().find(|(n, _)| *n == x) {
                Some((_, e)) => {
                    out.push(Out::Lit("(".into()));
                    spice_pieces(e, ctx, out, depth + 1)?;
                    out.push(Out::Lit(")".into()));
                }
                None => return Err(format!("{{{h}}}: {x} is no parameter")),
            },
            Tok::Op("^") => out.push(Out::Lit("**".into())),
            Tok::Num(s) | Tok::Op(s) => out.push(Out::Lit(s.into())),
        }
    }
    Ok(())
}

/// A symbol text template as an xschem `T` text: `{name}` → `@name`, `{cell}` →
/// `@symname`, `{p}` → `@p`; other holes are dropped (the flag says so).
pub(super) fn convert_stext(tpl: &str, ctx: &FmtCtx) -> (String, bool) {
    let Some(pieces) = split_template(tpl) else {
        return (render(&[Out::Lit(tpl.into())]), true);
    };
    let (mut out, mut dropped) = (Vec::new(), false);
    for piece in pieces {
        match piece {
            Piece::Lit(s) => out.push(Out::Lit(s)),
            Piece::Hole(h) => match h.trim() {
                "name" => out.push(Out::Tok("name".into())),
                "cell" => out.push(Out::Tok("symname".into())),
                p if ctx.params.contains(&p) => out.push(Out::Tok(p.into())),
                _ => dropped = true,
            },
        }
    }
    (render(&out), dropped)
}
