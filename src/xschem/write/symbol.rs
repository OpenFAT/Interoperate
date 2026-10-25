//! Schemify symbols → xschem `.sym` files (spec `interop.md` §11: pins, params, formats, graphics, `stext`).

use super::*;

// ---------------------------------------------------------------------------------------
// Symbols
// ---------------------------------------------------------------------------------------

/// One `param` record.
pub(super) struct Param<'a> {
    pub(super) name: &'a str,
    pub(super) default: &'a str,
    pub(super) kind: &'a str,
    pub(super) computed: Option<&'a str>,
}

pub(super) fn params(c: &Cell) -> Vec<Param<'_>> {
    c.records(RecordKind::Param)
        .map(|r| Param {
            name: c.field(r, 0),
            default: c.field(r, 1),
            kind: c.attr(r, "kind").unwrap_or("number"),
            computed: c.attr(r, "computed"),
        })
        .collect()
}

/// The default impl record of `c` (header `default`, else the first impl).
pub(super) fn default_impl(c: &Cell) -> Option<u32> {
    let want = c.header("default");
    let mut impls = c.records(RecordKind::Impl);
    match want {
        Some(w) => impls.find(|&r| c.field(r, 0) == w),
        None => impls.next(),
    }
}

/// True when instances of `m` netlist through Schemify's default template
/// `X{name} {pins} {cell}` (netlist.md §7.1): a schematic or SPICE-text default impl
/// without a `format spice`.
pub(super) fn default_call(m: &Cell) -> bool {
    default_impl(m).is_some_and(|i| {
        matches!(m.field(i, 1), "schematic" | "spice")
            && !formats(m, i).iter().any(|f| f.0 == "spice")
    })
}

/// Formats of impl `r`: `(dialect, template)` in file order.
pub(super) fn formats(c: &Cell, r: u32) -> Vec<(&str, &str)> {
    c.doc
        .attrs(r)
        .filter(|&a| c.doc.attr_key(a) == "format")
        .map(|a| {
            (
                c.doc.attr_value(a, 0).unwrap_or(""),
                c.doc.attr_value(a, 1).unwrap_or(""),
            )
        })
        .collect()
}

/// Writes the `.sym` of a `.shprim` or `.shcomp`.
pub(super) fn write_symbol(c: &Cell, sc: &Scope, notes: &mut Notes) -> String {
    assert!(matches!(c.kind(), FileKind::Shprim | FileKind::Shcomp));
    let ps = params(c);
    let pins = sym_pins(c);
    let imp = default_impl(c);
    let lang = imp.map_or("", |r| c.field(r, 1));
    let subckt = lang == "schematic";
    let ctx = fmt_ctx(c, &pins);
    let xname = |p: usize| pins.alias[p].as_deref().unwrap_or(&pins.name[p]);
    let mut k: Vec<String> = vec![kv("type", if subckt { "subcircuit" } else { "primitive" })];
    // Netlist formats of the default impl.
    let mut spice = None;
    if let Some(r) = imp {
        for (dialect, tpl) in formats(c, r) {
            let key = match dialect {
                "spice" => "format",
                "cdl" => "lvs_format",
                "spectre" => "spectre_format",
                "verilog" => "verilog_format",
                other => {
                    notes.review("format", &c.path, format!("format {other}: no xschem key"));
                    continue;
                }
            };
            match convert_format(tpl, &ctx) {
                Ok(f) => {
                    if key == "format" {
                        spice = Some(f);
                    } else {
                        k.push(kv(key, &f));
                    }
                }
                Err(why) => notes.review("template", &c.path, format!("format {dialect}: {why}")),
            }
        }
    }
    if spice.is_none() && default_call(c) {
        // Schemify's default template `X{name} {pins} {cell}`; the `X` is the instance
        // name's (`instance` renames calls that lack it).
        let mut f = "@name @pinlist".to_owned();
        for p in &ctx.inherited {
            write!(f, " @{p}").unwrap();
        }
        f.push_str(" @symname");
        spice = Some(f);
    }
    if let (true, Some(f)) = (subckt, spice.as_mut()) {
        // Every parameter passed by name (the importer strips the `k=@k` pairs again).
        for p in &ctx.params {
            write!(f, " {p}=@{p}").unwrap();
        }
    } else if spice.is_none() && imp.is_some() {
        notes.review(
            "format",
            &c.path,
            format!("impl {lang}: no spice format, xschem will not netlist it"),
        );
    }
    if let Some(f) = spice {
        k.insert(1, kv("format", &f));
    }
    // Template: instance-name prefix, parameter defaults, inherited-pin default nets.
    let prefix = c.header("prefix").unwrap_or(if subckt { "x" } else { "U" });
    let mut tpl = vec![kv("name", &format!("{prefix}1"))];
    for p in ps.iter().filter(|p| p.computed.is_none()) {
        match xschem_value(p.kind, p.default, sc, ctx.raw.contains(&p.name)) {
            Ok(v) => tpl.push(kv(p.name, &v)),
            Err(why) => notes.review(
                why,
                &c.path,
                format!("param {} default {:?}: left empty", p.name, p.default),
            ),
        }
    }
    for p in 0..pins.name.len() {
        if let Some(net) = &pins.inherit[p] {
            tpl.push(kv(&pins.name[p], net));
        }
    }
    k.push(kv("template", &tpl.join(" ")));
    if !ctx.inherited.is_empty() {
        k.push(kv("extra", &ctx.inherited.join(" ")));
    }
    if let Some(r) = imp {
        if let Some(ig) = c.attr(r, "ignore") {
            k.push(kv("spice_ignore", ig));
        }
        if c.attr(r, "file").is_some() {
            notes.review(
                "format",
                &c.path,
                "impl file: not carried (xschem has no source file)",
            );
        }
        let name = c.field(r, 0);
        if let Some(b) = c.records(RecordKind::Body).find(|&b| c.field(b, 0) == name) {
            if lang == "spice" || lang == "schematic" {
                let text: Vec<&str> = c.doc.verbatim(b).collect();
                k.push(kv("device_model", &text.join("\n")));
            } else {
                notes.review(
                    "format",
                    &c.path,
                    format!("body {name} ({lang}): not carried"),
                );
            }
        }
    }
    let other_impls = c.records(RecordKind::Impl).count().saturating_sub(1);
    if other_impls > 0 {
        notes.review(
            "format",
            &c.path,
            format!("{other_impls} non-default impls: only the default one is exported"),
        );
    }
    let mut out = String::from(VERSION);
    out.push_str("G {}\n");
    writeln!(out, "K {}", brace(&k.join("\n"))).unwrap();
    out.push_str("V {}\nS {}\nE {}\n");
    let mut drew = false;
    for r in 1..c.doc.record_count() {
        let kind = c.doc.record_kind(r);
        if matches!(
            kind,
            RecordKind::Line
                | RecordKind::Rect
                | RecordKind::Arc
                | RecordKind::Circle
                | RecordKind::Poly
        ) {
            drew = true;
            let vals: Vec<i32> = (0..).map_while(|i| c.coord(r, i)).collect();
            shape(&mut out, c, r, kind, &vals, true, notes);
        } else if kind == RecordKind::Stext {
            let (text, dropped) = convert_stext(c.field(r, 3), &ctx);
            if dropped {
                notes.review(
                    "template",
                    &c.path,
                    format!(
                        "stext {:?}: holes other than name, cell, params dropped",
                        c.field(r, 3)
                    ),
                );
            }
            let o = c.orient(r, 2);
            let mut props = Vec::new();
            if let Some(l) = c.attr(r, "layer") {
                props.push(kv("layer", l));
            }
            for flag in ["hcenter", "vcenter"] {
                if c.doc.find_attr(r, flag).is_some() {
                    props.push(kv(flag, "true"));
                }
            }
            let size = c.attr(r, "size").unwrap_or("0.2");
            text_record(
                &mut out,
                &text,
                [c.coord(r, 0), c.coord(r, 1)],
                o,
                size,
                &props,
            );
        }
    }
    if !drew {
        // A symbol without graphics would be invisible in xschem: a box around the pins.
        let (mut lo, mut hi) = ([-20_000, -20_000], [20_000, 20_000]);
        for p in pins
            .at
            .iter()
            .zip(&pins.inherit)
            .filter(|(_, i)| i.is_none())
            .map(|(a, _)| a)
        {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i] + 10_000);
                hi[i] = hi[i].max(p[i] - 10_000);
            }
        }
        let mut line = String::from("B 4");
        for v in [lo[0], lo[1], hi[0], hi[1]] {
            line.push(' ');
            format_coord(v, &mut line);
        }
        writeln!(out, "{line} {{fill=false}}").unwrap();
        text_record(
            &mut out,
            "@symname",
            [Some(lo[0]), Some(lo[1] - 15_000)],
            Orient::R0,
            "0.2",
            &[],
        );
    }
    for p in 0..pins.name.len() {
        if pins.inherit[p].is_some() {
            continue;
        }
        let [x, y] = pins.at[p];
        let mut line = String::from("B 5");
        for v in [x - 2500, y - 2500, x + 2500, y + 2500] {
            line.push(' ');
            format_coord(v, &mut line);
        }
        let props = format!("{} {}", kv("name", xname(p)), kv("dir", &pins.dir[p]));
        writeln!(out, "{line} {}", brace(&props)).unwrap();
    }
    let drawn = pins.inherit.iter().filter(|i| i.is_none()).count();
    assert_eq!(ctx.pins.len() + ctx.inherited.len(), pins.name.len());
    assert_eq!(ctx.pins.len(), drawn, "a pin box per drawn pin");
    out
}

/// Notes the header, parameter and pin data xschem has no field for.
pub(super) fn lost_attrs(c: &Cell, notes: &mut Notes) {
    for a in c.doc.attrs(0) {
        let reason = match c.doc.attr_key(a) {
            "default" | "prefix" | "corner" | "provenance" => continue,
            "doc" | "category" => "docs",
            "config" => "config",
            _ => "header",
        };
        notes.review(reason, &c.path, format!("header {}", c.doc.attr_key(a)));
    }
    let mut lost = BTreeSet::new();
    for r in c.records(RecordKind::Param) {
        for a in c.doc.attrs(r) {
            let key = c.doc.attr_key(a);
            if key != "kind" && key != "computed" {
                lost.insert(format!("param {} {key}", c.field(r, 0)));
            }
        }
    }
    for r in c.records(RecordKind::Pin) {
        for a in c.doc.attrs(r) {
            let key = c.doc.attr_key(a);
            if key != "inherit" && key != "alias" {
                lost.insert(format!("pin {} {key}", c.field(r, 0)));
            }
        }
    }
    if !lost.is_empty() {
        let list: Vec<String> = lost.into_iter().collect();
        notes.review("param-attr", &c.path, list.join(", "));
    }
}
