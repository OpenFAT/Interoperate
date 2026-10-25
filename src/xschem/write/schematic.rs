//! Schemify schematics and benches → xschem `.sch` files (spec `interop.md` §11).

use super::*;

// ---------------------------------------------------------------------------------------
// Schematics
// ---------------------------------------------------------------------------------------

/// Instance names already used in one schematic, lowercased (xschem keeps names unique
/// case-insensitively and would rename a clash); hands out fresh ones per prefix.
pub(super) struct Names {
    pub(super) taken: HashSet<String>,
}

impl Names {
    /// `name` itself when free, else `name2`, `name3`, ….
    pub(super) fn unique(&mut self, name: &str) -> String {
        if self.taken.insert(name.to_ascii_lowercase()) {
            return name.to_owned();
        }
        let n = self.fresh(name);
        assert_ne!(n, name);
        n
    }

    /// `prefix1`, `prefix2`, …: the first free one.
    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        // Bounded: among `taken.len() + 1` candidates one is free.
        for k in 1..=self.taken.len() + 1 {
            let n = format!("{prefix}{k}");
            if self.taken.insert(n.to_ascii_lowercase()) {
                return n;
            }
        }
        unreachable!("pigeonhole: a candidate is free")
    }
}

/// Writes the `.sch` of a `.shcomp` or `.shbench`.
pub(super) fn write_schematic(
    d: &Design,
    ci: usize,
    targets: &[Option<Target>],
    sc: &Scope,
    notes: &mut Notes,
) -> String {
    let c = &d.cells[ci];
    assert!(matches!(c.kind(), FileKind::Shcomp | FileKind::Shbench));
    let bench = c.kind() == FileKind::Shbench;
    let mut names = Names {
        taken: c
            .records(RecordKind::Inst)
            .map(|r| c.attr(r, "name").unwrap_or("").to_ascii_lowercase())
            .chain(
                c.records(RecordKind::Code)
                    .map(|r| c.field(r, 0).to_ascii_lowercase()),
            )
            .collect(),
    };
    let iface = sym_pins(c);
    let mut out = String::from(VERSION);
    out.push_str("G {}\nK {}\nV {}\nS {}\nE {}\n");
    let (mut lo, mut hi) = ([i32::MAX, i32::MAX], [i32::MIN, i32::MIN]);
    let mut grow = |p: [i32; 2]| {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    };
    for r in 1..c.doc.record_count() {
        let xy = |i| c.point(r, i);
        let orient = |i| c.orient(r, i);
        match c.doc.record_kind(r) {
            RecordKind::Shape => {
                let form = c.field(r, 1);
                let kind = match form {
                    "line" => RecordKind::Line,
                    "rect" => RecordKind::Rect,
                    "arc" => RecordKind::Arc,
                    "circle" => RecordKind::Circle,
                    _ => RecordKind::Poly,
                };
                let vals: Vec<i32> = (2..).map_while(|i| c.coord(r, i)).collect();
                for p in vals.as_chunks::<2>().0 {
                    grow(*p);
                }
                shape(&mut out, c, r, kind, &vals, false, notes);
            }
            RecordKind::Text => {
                grow(xy(1));
                let size = c.attr(r, "size").unwrap_or("0.4");
                let at = xy(1);
                text_record(
                    &mut out,
                    c.field(r, 4),
                    [Some(at[0]), Some(at[1])],
                    orient(3),
                    size,
                    &[],
                );
            }
            RecordKind::Wire => {
                grow(xy(1));
                grow(xy(3));
                let mut line = String::from("N");
                for i in 1..5 {
                    line.push(' ');
                    format_coord(c.coord(r, i).unwrap_or(0), &mut line);
                }
                let props = if c.attr(r, "bus") == Some("true") {
                    "bus=true"
                } else {
                    ""
                };
                writeln!(out, "{line} {}", brace(props)).unwrap();
            }
            RecordKind::Inst => {
                let a = c.doc.find_attr(r, "at").expect("inst has at (format E029)");
                let v = |i| c.doc.attr_value(a, i).unwrap_or("");
                let at = [
                    parse_coord(v(0)).unwrap_or(0),
                    parse_coord(v(1)).unwrap_or(0),
                ];
                grow(at);
                let m = d
                    .resolve(ci, c.field(r, 1))
                    .expect("loaded Designs resolve every reference");
                let target = targets[m].as_ref().expect("instantiated cells are live");
                let (symref, props) = instance(c, r, &d.cells[m], target, sc, notes);
                component(
                    &mut out,
                    &symref,
                    at,
                    Orient::parse(v(2)).unwrap_or(Orient::R0),
                    &props,
                );
            }
            RecordKind::Label => {
                grow(xy(1));
                let net = c.field(r, 4);
                let mut props = vec![kv("name", &names.fresh("l")), kv("lab", net)];
                if c.attr(r, "global") == Some("true") || net.ends_with('!') {
                    props.push(kv("global", "true"));
                }
                component(&mut out, "lab_pin.sym", xy(1), orient(3), &props);
            }
            RecordKind::Port => {
                grow(xy(1));
                let pin = c.field(r, 4);
                let p = iface.name.iter().position(|n| n == pin);
                let lab = p.and_then(|p| iface.alias[p].as_deref()).unwrap_or(pin);
                let sym = match p.map(|p| iface.dir[p].as_str()) {
                    Some("in") => "ipin.sym",
                    Some("out") => "opin.sym",
                    _ => "iopin.sym",
                };
                let props = [kv("name", &names.fresh("p")), kv("lab", lab)];
                component(&mut out, sym, xy(1), orient(3), &props);
            }
            RecordKind::Noconn => {
                grow(xy(1));
                let props = [kv("name", &names.fresh("nc"))];
                component(&mut out, "noconn.sym", xy(1), Orient::R0, &props);
            }
            RecordKind::Prose => notes.review("docs", &c.path, format!("prose {}", c.field(r, 0))),
            RecordKind::Constraint
            | RecordKind::Waive
            | RecordKind::Sweep
            | RecordKind::Montecarlo
            | RecordKind::Optimize
            | RecordKind::Output
            | RecordKind::Spec
            | RecordKind::Config => {
                let word = c.doc.record_word(r);
                notes.review(word, &c.path, format!("{word} {}", c.field(r, 0)));
            }
            RecordKind::Var if c.attr(r, "dist").is_some() => {
                notes.review("montecarlo", &c.path, format!("var {} dist", c.field(r, 0)));
            }
            _ => {}
        }
    }
    // Netlist text: code_shown instances in a column under the drawing.
    let mut blocks: Vec<(String, String)> = Vec::new();
    if bench {
        let (vars, corner) = bench_text(c, sc, notes);
        if !vars.is_empty() {
            blocks.push((names.unique("params"), vars));
        }
        if !corner.is_empty() {
            blocks.push((names.unique("corner"), corner));
        }
    }
    for r in c.records(RecordKind::Code) {
        let (name, lang) = (c.field(r, 0), c.field(r, 1));
        let text: Vec<&str> = c.doc.verbatim(r).collect();
        let text = text.join("\n");
        if !matches!(lang, "spice" | "ngspice") {
            notes.review(
                "code",
                &c.path,
                format!("code {name} {lang}: only SPICE text carries"),
            );
        } else if text.contains("${") {
            notes.review(
                "code",
                &c.path,
                format!("code {name}: ${{NAME}} expansion has no xschem form"),
            );
        } else {
            blocks.push((name.to_owned(), text.replace("$$", "$")));
        }
    }
    if !bench && let Some(imp) = default_impl(c) {
        let name = c.field(imp, 0);
        if let Some(b) = c.records(RecordKind::Body).find(|&b| c.field(b, 0) == name) {
            let text: Vec<&str> = c.doc.verbatim(b).collect();
            blocks.push((names.unique("body"), text.join("\n")));
        }
    }
    let (x0, mut y) = if lo[0] <= hi[0] {
        (lo[0], hi[1] + 60_000)
    } else {
        (0, 0)
    };
    for (name, text) in &blocks {
        let props = [
            kv("name", name),
            kv("only_toplevel", "false"),
            kv("value", text),
        ];
        component(&mut out, "code_shown.sym", [x0, y], Orient::R0, &props);
        let lines = i32::try_from(text.lines().count()).unwrap_or(i32::MAX / 40_000);
        y = y.saturating_add(60_000 + 20_000 * lines.min(10_000));
    }
    assert!(out.starts_with(VERSION) && out.ends_with('\n'));
    out
}

/// The `.param` lines of a bench's vars and the model lines of its default corner (its
/// `set`s replace var values); other corners are reviewed.
pub(super) fn bench_text(c: &Cell, sc: &Scope, notes: &mut Notes) -> (String, String) {
    let default = c.header("corner");
    let mut sets: Vec<(&str, &str)> = Vec::new();
    let mut corner = String::new();
    for r in c.records(RecordKind::Corner) {
        let name = c.field(r, 0);
        if Some(name) != default {
            notes.review(
                "corner",
                &c.path,
                format!("corner {name}: only the default corner carries"),
            );
            continue;
        }
        for a in c.doc.attrs(r) {
            let v = |i| c.doc.attr_value(a, i).unwrap_or("");
            match c.doc.attr_key(a) {
                "lib" => writeln!(corner, ".lib {} {}", v(0), v(1)).unwrap(),
                "include" => writeln!(corner, ".include {}", v(0)).unwrap(),
                "set" => sets.push((v(0), v(1))),
                "temp" => match spice_value(v(0), sc, false) {
                    Ok(t) => writeln!(corner, ".temp {t}").unwrap(),
                    Err(why) => {
                        notes.review(why, &c.path, format!("corner {name} temp {:?}", v(0)))
                    }
                },
                key => notes.review("corner", &c.path, format!("corner {name} {key}")),
            }
        }
    }
    if corner.contains("${") {
        notes.review(
            "corner",
            &c.path,
            "${NAME} in a model path is written verbatim",
        );
    }
    let mut vars = String::new();
    for r in c.records(RecordKind::Var) {
        let name = c.field(r, 0);
        let expr = sets
            .iter()
            .rev()
            .find(|s| s.0 == name)
            .map_or(c.field(r, 1), |s| s.1);
        let value = spice_value(expr, sc, false).or_else(|why| {
            c.doc
                .find_attr(r, "cached")
                .and_then(|a| c.doc.attr_value(a, 0))
                .map(|v| {
                    notes.review(
                        "expression",
                        &c.path,
                        format!("var {name}: its cached value {v}"),
                    );
                    v.to_owned()
                })
                .ok_or(why)
        });
        match value {
            Ok(v) => writeln!(vars, ".param {name}={v}").unwrap(),
            Err(why) => notes.review(why, &c.path, format!("var {name} {expr:?}: not written")),
        }
    }
    (vars.trim_end().to_owned(), corner.trim_end().to_owned())
}

/// The symbol reference and attributes of instance record `r` of `c`, whose master is `m`.
pub(super) fn instance(
    c: &Cell,
    r: u32,
    m: &Cell,
    target: &Target,
    sc: &Scope,
    notes: &mut Notes,
) -> (String, Vec<String>) {
    assert_eq!(c.doc.record_kind(r), RecordKind::Inst);
    let name = c.attr(r, "name").unwrap_or("");
    // Schemify's default template adds the subcircuit letter to a name without it
    // (netlist.md §7.6.5); xschem writes the name as is, so the name carries it.
    let xname =
        if matches!(target, Target::Own(_)) && default_call(m) && !name.starts_with(['x', 'X']) {
            notes.review(
                "name",
                &c.path,
                format!("{name} written X{name} (the subcircuit call letter)"),
            );
            format!("X{name}")
        } else {
            name.to_owned()
        };
    let mut props = vec![kv("name", &xname)];
    let mparams = params(m);
    let mpins = sym_pins(m);
    let mctx = fmt_ctx(m, &mpins);
    let over = |key: &str| c.attr(r, key);
    let symref = match target {
        Target::Own(dir) => format!("{dir}/{}.sym", m.name),
        Target::Original(s) => s.clone(),
        Target::Device(map) => format!("{}.sym", map.symbol),
        Target::Label(s) => (*s).to_owned(),
    };
    match target {
        Target::Label(_) => {
            let lab = over("lab")
                .or(mparams.first().map(|p| p.default))
                .unwrap_or("");
            props.push(kv("lab", lab));
        }
        // Every core parameter, defaults included: xschem's template defaults differ
        // (`nmos4` w=5u against core 1u), so nothing may fall back to them.
        Target::Device(map) => {
            for p in mparams.iter().filter(|p| p.computed.is_none()) {
                let key = map
                    .params
                    .iter()
                    .find(|(_, core)| *core == p.name)
                    .map_or(p.name, |(x, _)| x);
                let src = over(p.name).unwrap_or(p.default);
                match xschem_value(p.kind, src, sc, false) {
                    Ok(v) => props.push(kv(key, &v)),
                    Err(why) => notes.review(
                        why,
                        &c.path,
                        format!("{name}.{}={src:?}: xschem default kept", p.name),
                    ),
                }
            }
        }
        Target::Own(_) | Target::Original(_) => {
            for a in c.doc.attrs(r) {
                let key = c.doc.attr_key(a);
                if matches!(key, "at" | "name" | "ignore" | "inherit") {
                    continue;
                }
                let src = c.doc.attr_value(a, 0).unwrap_or("");
                let Some(p) = mparams.iter().find(|p| p.name == key) else {
                    notes.review(
                        "expression",
                        &c.path,
                        format!("{name}.{key}: not a parameter of {}", m.name),
                    );
                    continue;
                };
                match xschem_value(p.kind, src, sc, mctx.raw.contains(&key)) {
                    Ok(v) => props.push(kv(key, &v)),
                    Err(why) => notes.review(
                        why,
                        &c.path,
                        format!("{name}.{key}={src:?}: symbol default kept"),
                    ),
                }
            }
        }
    }
    if let Some(ig) = over("ignore") {
        props.push(kv("spice_ignore", ig));
    }
    for a in c.doc.attrs(r).filter(|&a| c.doc.attr_key(a) == "inherit") {
        let (pin, net) = (
            c.doc.attr_value(a, 0).unwrap_or(""),
            c.doc.attr_value(a, 1).unwrap_or(""),
        );
        props.push(kv(pin, net));
    }
    assert!(props[0].starts_with("name=") && !symref.is_empty());
    (symref, props)
}
