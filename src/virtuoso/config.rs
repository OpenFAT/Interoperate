//! Virtuoso config views (`expand.cfg`) ⇄ bench `config` records (spec `interop.md` §4.2
//! `config`, §9.3; format.md rank 6, netlist.md §2).
//!
//! A config view crosses as its `expand.cfg` text (`("config" view text)`): SKILL has no hdb
//! getters for the lists and rules (G34). [`import`] maps every statement Schemify can hold
//! onto the bench's `config <view>` record — the global view list to `order`, the stop list
//! and cell stops to `stop`, cell bindings to `cell`, instance bindings to `cellinst`,
//! occurrence bindings to `inst` — and keeps the whole text as `vcfg map|keep <piece>`
//! lines (one per statement with the comments and blank lines before it, in file order;
//! `keep` marks a statement with no Schemify form) under `vcfgsum`, the digest of the mapped
//! attributes. [`export`] sends the pieces back verbatim while that digest holds; after a
//! Schemify edit it writes the `keep` statements and regenerates the rest from the record.
//! A maestro test bound to a config view gives the bench header `config` (import) and back
//! (`bind_test`, export).

use schemify_format::{Document, FileKind, RecordKind};

use super::read::{DCell, Dump};
use crate::design::{Cell, Design, copy_header, push_attr};
use crate::map::Plan;
use crate::merge::digest;
use crate::notes::Notes;
use crate::profile;
use crate::sfyx::{Record, Value};

/// A config view of a dump cell (`("config" view text)`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigView {
    /// View name.
    pub view: String,
    /// The view's `expand.cfg`.
    pub text: String,
}

/// One piece of the imported `expand.cfg`: `map` or `keep`, then the text.
const VCFG: &str = "vcfg";
/// Digest of the mapped attributes and the design the import wrote.
const VCFGSUM: &str = "vcfgsum";
/// The attributes a config's statements map onto, in format.md order.
const MAPPED: [&str; 5] = ["order", "cell", "inst", "cellinst", "stop"];

/// The impl a Virtuoso view becomes (§9.3 view↔impl table); `None`: no Schemify impl.
pub fn impl_of_view(view: &str) -> Option<&'static str> {
    match view {
        "schematic" => Some("schematic"),
        "spectre" | "cmos_sch" | "symbol" => Some("spice"),
        "veriloga" => Some("veriloga"),
        "functional" | "verilog" => Some("verilog"),
        _ => None,
    }
}

/// The view an impl (or impl language) goes back to.
pub fn view_of_impl(imp: &str) -> Option<&'static str> {
    match imp {
        "schematic" => Some("schematic"),
        "spice" => Some("spectre"),
        "veriloga" => Some("veriloga"),
        "verilog" => Some("functional"),
        _ => None,
    }
}

/// What an object rule does.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Rule {
    /// `binding [L.C]:V`: the cell, when named, then the view.
    Bind(Option<(String, String)>, String),
    /// `stop`.
    Stop,
    /// A per-object `viewlist` or `liblist`.
    Other,
}

/// One `expand.cfg` statement, its comments stripped.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stmt {
    /// `config`, `endconfig`, or only comments and blanks.
    Header,
    /// `design L.C:V`.
    Design(String, String, String),
    /// The global `viewlist`.
    ViewList(Vec<String>),
    /// The global `stoplist`.
    StopList(Vec<String>),
    /// `cell L.C <rule>`.
    Cell(String, String, Rule),
    /// `inst (L.C:V).I <rule>`: the cell holding the instance, its name.
    Inst(String, String, String, Rule),
    /// `path (L.C:V).(…)I.J… <rule>`: the top cell, the instance names from the top.
    Path(String, String, Vec<String>, Rule),
    /// `liblist` and anything else: Virtuoso's only.
    Other,
}

/// `text` cut after each `;` outside a `//` comment: `(raw piece, statement)` with the
/// statement's comments dropped, whitespace collapsed and the `;` removed. The pieces
/// concatenate to `text`.
fn pieces(text: &str) -> Vec<(&str, String)> {
    let mut out = Vec::new();
    let (mut start, mut comment) = (0, false);
    let b = text.as_bytes();
    for (k, &c) in b.iter().enumerate() {
        match c {
            b'\n' => comment = false,
            b'/' if !comment && b.get(k + 1) == Some(&b'/') => comment = true,
            b';' if !comment => {
                out.push(&text[start..=k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    debug_assert_eq!(out.concat(), text);
    out.into_iter().map(|p| (p, statement(p))).collect()
}

fn statement(raw: &str) -> String {
    let code: Vec<&str> = raw
        .lines()
        .map(|l| l.find("//").map_or(l, |k| &l[..k]))
        .collect();
    let s = code.join(" ");
    let s = s.trim_end().strip_suffix(';').unwrap_or(&s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn list(s: &str) -> Vec<String> {
    s.split([',', ' '])
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A cellview as `expand.cfg` names it, `(lib, cell, view)`.
type Lcv = (String, String, String);

/// `L.C:V`.
fn lcv(s: &str) -> Option<Lcv> {
    let (lc, v) = s.rsplit_once(':')?;
    let (l, c) = lc.split_once('.')?;
    Some((l.to_owned(), c.to_owned(), v.to_owned()))
}

/// `(L.C:V).(L.C:V)I.J`: the cellviews and the instance names, each in order.
// G41 settled (live check l3): hdb prints each level's master before its instance,
// `(top).(m1)I1.(m2)I2.R0`, and the last instance bare; `generate` writes the same.
fn obj_path(s: &str) -> Option<(Vec<Lcv>, Vec<String>)> {
    let (mut cvs, mut insts) = (Vec::new(), Vec::new());
    let mut rest = s;
    loop {
        rest = rest.trim_start_matches('.');
        if rest.is_empty() {
            return Some((cvs, insts));
        }
        if let Some(r) = rest.strip_prefix('(') {
            let (cv, r) = r.split_once(')')?;
            cvs.push(lcv(cv)?);
            rest = r;
        } else {
            let end = rest.find(['.', '(']).unwrap_or(rest.len());
            insts.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
    }
}

fn rule(s: &str) -> Rule {
    match s.split_once(' ') {
        _ if s == "stop" => Rule::Stop,
        Some(("binding", t)) => match t.strip_prefix(':') {
            Some(v) => Rule::Bind(None, v.to_owned()),
            None => match lcv(t) {
                Some((l, c, v)) => Rule::Bind(Some((l, c)), v),
                None => Rule::Other,
            },
        },
        _ => Rule::Other,
    }
}

fn parse(s: &str) -> Stmt {
    let (word, rest) = s.split_once(' ').unwrap_or((s, ""));
    let (obj, r) = rest.split_once(' ').unwrap_or((rest, ""));
    let parsed = match word {
        "" | "config" | "endconfig" => Some(Stmt::Header),
        "design" => lcv(rest).map(|(l, c, v)| Stmt::Design(l, c, v)),
        "viewlist" => Some(Stmt::ViewList(list(rest))),
        "stoplist" => Some(Stmt::StopList(list(rest))),
        "cell" => obj
            .split_once('.')
            .map(|(l, c)| Stmt::Cell(l.into(), c.into(), rule(r))),
        "inst" => match obj_path(obj) {
            Some((cv, i)) if cv.len() == 1 && i.len() == 1 => {
                let (l, c, _) = cv.into_iter().next().expect("one");
                Some(Stmt::Inst(
                    l,
                    c,
                    i.into_iter().next().expect("one"),
                    rule(r),
                ))
            }
            _ => None,
        },
        "path" => match obj_path(obj) {
            Some((cv, i)) if !cv.is_empty() && !i.is_empty() => {
                let (l, c, _) = cv.into_iter().next().expect("top");
                Some(Stmt::Path(l, c, i, rule(r)))
            }
            _ => None,
        },
        _ => None,
    };
    parsed.unwrap_or(Stmt::Other)
}

/// The mapped attributes in canonical order, as `(key, values)`.
type Attrs = Vec<(&'static str, Vec<String>)>;

/// The digest `vcfgsum` holds: the mapped attributes and the design cell.
fn sum(attrs: &Attrs, lib: &str, cell: &str) -> String {
    let mut s = format!("design {lib}.{cell}\n");
    for (k, v) in attrs {
        s.push_str(&format!("{k} {}\n", v.join(" ")));
    }
    digest(&s)
}

fn push_unique(v: &mut Vec<String>, x: &str) {
    if !v.iter().any(|y| y == x) {
        v.push(x.to_owned());
    }
}

/// The master `(lib, cell)` of instance `inst` in the dump's schematic of `lib/cell`.
fn master_of<'a>(dump: &'a Dump, lib: &str, cell: &str, inst: &str) -> Option<(&'a str, &'a str)> {
    let c = dump.cells.iter().find(|c| c.lib == lib && c.name == cell)?;
    let i = c.cv("schematic")?.insts.iter().find(|i| i.name == inst)?;
    Some((&i.lib, &i.cell))
}

/// `doc` (the bench built from dump cell `c`) with one `config` record per config view of
/// `c`, and header `config` when a maestro test of `c` is bound to one of them (G31).
/// `cellref` gives the Schemify cellref of a Virtuoso cell. What has no Schemify form is
/// reported (`needs_review:config`) and kept for Virtuoso as a `keep` piece; a config view
/// of a cell that is no bench is reported and not imported.
pub fn import(
    doc: Document<'static>,
    c: &DCell,
    dump: &Dump,
    cellref: &dyn Fn(&str, &str) -> Option<String>,
    notes: &mut Notes,
) -> Document<'static> {
    let at = c.name.as_str();
    if c.configs.is_empty() {
        return doc;
    }
    if doc.kind() != FileKind::Shbench {
        for v in &c.configs {
            notes.review(
                "config",
                at,
                format!(
                    "config view {}: only a bench holds configs; not imported",
                    v.view
                ),
            );
        }
        return doc;
    }
    // The first test bound to a view other than the schematic names the bench's config.
    let bound = dump
        .maestro
        .iter()
        .map(|t| &t[0])
        .filter(|m| m.str(1).ok() == Some(c.lib.as_str()) && m.str(2).ok() == Some(c.name.as_str()))
        .find_map(|m| Some((m.str(0).ok()?, m.str(3).ok()?)).filter(|x| x.1 != "schematic"));
    let header = match bound {
        Some((_, v)) if c.configs.iter().any(|x| x.view == v) => Some(v),
        Some((t, v)) => {
            notes.review(
                "config",
                at,
                format!("maestro test {t} is bound to view {v}, no config view of this cell"),
            );
            None
        }
        None => None,
    };
    let mut out = Document::new(doc.kind());
    match header {
        Some(v) => {
            copy_header(&mut out, &doc, &["config"]);
            push_attr(&mut out, "config", &[v]);
        }
        None => copy_header(&mut out, &doc, &[]),
    }
    for r in 1..doc.record_count() {
        out.copy_record(&doc, r);
    }
    for v in &c.configs {
        config_in(&mut out, v, c, dump, cellref, notes);
    }
    out
}

fn config_in(
    doc: &mut Document<'static>,
    v: &ConfigView,
    c: &DCell,
    dump: &Dump,
    cellref: &dyn Fn(&str, &str) -> Option<String>,
    notes: &mut Notes,
) {
    let (mut order, mut stop) = (Vec::new(), Vec::new());
    let (mut cells, mut insts, mut cellinsts) = (Vec::new(), Vec::new(), Vec::new());
    let mut out: Vec<(bool, &str)> = Vec::new();
    for (raw, s) in pieces(&v.text) {
        // A binding to another cell (`binding L.C:V` naming a master other than the
        // object's) has no Schemify form: a config selects impls, it does not re-master.
        let imp = |r: &Rule, master: Option<(&str, &str)>| match r {
            Rule::Bind(t, view)
                if t.as_ref()
                    .is_none_or(|(l, c)| master == Some((l.as_str(), c.as_str()))) =>
            {
                impl_of_view(view)
            }
            _ => None,
        };
        let mapped = match parse(&s) {
            Stmt::Header => true,
            Stmt::Design(l, cl, view) => {
                if l != c.lib || cl != c.name || view != "schematic" {
                    notes.review(
                        "config",
                        &c.name,
                        format!(
                            "config {}: design {l}.{cl}:{view}, not this schematic",
                            v.view
                        ),
                    );
                }
                true
            }
            Stmt::ViewList(views) | Stmt::StopList(views) => {
                let into = if s.starts_with("viewlist") {
                    &mut order
                } else {
                    &mut stop
                };
                for x in &views {
                    match impl_of_view(x) {
                        Some(i) => push_unique(into, i),
                        None => notes.review(
                            "config",
                            &c.name,
                            format!(
                                "config {}: view {x} has no Schemify impl; dropped from `{s}`",
                                v.view
                            ),
                        ),
                    }
                }
                true
            }
            Stmt::Cell(l, cl, r) => match (cellref(&l, &cl), &r) {
                (Some(cr), Rule::Stop) => {
                    push_unique(&mut stop, &cr);
                    true
                }
                (Some(cr), _) => imp(&r, Some((l.as_str(), cl.as_str())))
                    .map(|i| cells.push(vec![cr, i.to_owned()]))
                    .is_some(),
                (None, _) => false,
            },
            Stmt::Inst(l, cl, i, r) => match cellref(&l, &cl) {
                Some(cr) => imp(&r, master_of(dump, &l, &cl, &i))
                    .map(|m| {
                        cellinsts.push(vec![cr, profile::inst_from_virtuoso(&i), m.to_owned()]);
                    })
                    .is_some(),
                None => false,
            },
            Stmt::Path(l, cl, names, r) if l == c.lib && cl == c.name => {
                let mut cur = Some((l.as_str(), cl.as_str()));
                for n in &names {
                    cur = cur.and_then(|(l, cl)| master_of(dump, l, cl, n));
                }
                imp(&r, cur)
                    .map(|m| {
                        let p: Vec<String> = names
                            .iter()
                            .map(|n| profile::inst_from_virtuoso(n))
                            .collect();
                        insts.push(vec![format!("/{}", p.join("/")), m.to_owned()]);
                    })
                    .is_some()
            }
            Stmt::Path(..) | Stmt::Other => false,
        };
        if !mapped {
            notes.review(
                "config",
                &c.name,
                format!(
                    "config {}: `{s}` has no Schemify form; kept for Virtuoso",
                    v.view
                ),
            );
        }
        out.push((mapped, raw));
    }
    let mut attrs: Attrs = Vec::new();
    if !order.is_empty() {
        attrs.push(("order", order));
    }
    attrs.extend(cells.into_iter().map(|x| ("cell", x)));
    attrs.extend(insts.into_iter().map(|x| ("inst", x)));
    attrs.extend(cellinsts.into_iter().map(|x| ("cellinst", x)));
    if !stop.is_empty() {
        attrs.push(("stop", stop));
    }
    doc.begin_record(RecordKind::Config);
    doc.push_field(&v.view);
    for (k, vals) in &attrs {
        push_attr(doc, k, vals);
    }
    push_attr(doc, VCFGSUM, &[sum(&attrs, &c.lib, &c.name)]);
    for (mapped, raw) in out {
        push_attr(doc, VCFG, &[if mapped { "map" } else { "keep" }, raw]);
    }
}

/// The config views of bench `ci` (one per `config` record), its Virtuoso cell being the
/// plan's. A record imported from Virtuoso and not edited since (`vcfgsum` holds) gives
/// back its `expand.cfg` byte for byte; otherwise the text is its `keep` statements plus
/// statements regenerated from the record. A rule naming a cell, path or impl with no
/// Virtuoso form is reported (`needs_review:config`) and not sent.
pub fn export(d: &Design, ci: usize, plan: &Plan, notes: &mut Notes) -> Vec<ConfigView> {
    let c = &d.cells[ci];
    let (vlib, vcell) = (plan.lib[ci].as_str(), plan.cell[ci].as_str());
    let mut out = Vec::new();
    for r in c.records(RecordKind::Config) {
        let mut attrs: Attrs = Vec::new();
        let mut pieces: Vec<(&str, &str)> = Vec::new();
        for a in c.doc.attrs(r) {
            let vals: Vec<&str> = c.doc.attr_values(a).collect();
            match (c.doc.attr_key(a), vals.as_slice()) {
                (VCFG, [flag, raw]) => pieces.push((*flag, *raw)),
                (k, _) => {
                    if let Some(m) = MAPPED.iter().find(|m| **m == k) {
                        attrs.push((*m, vals.iter().map(|v| (*v).to_owned()).collect()));
                    }
                }
            }
        }
        let fresh = Some(sum(&attrs, vlib, vcell));
        let text = if !pieces.is_empty() && c.attr(r, VCFGSUM).map(str::to_owned) == fresh {
            pieces.iter().map(|p| p.1).collect::<String>()
        } else {
            generate(d, ci, plan, &attrs, &pieces, c.field(r, 0), notes)
        };
        out.push(ConfigView {
            view: c.field(r, 0).to_owned(),
            text,
        });
    }
    out
}

/// The Design cell `cref` names from bench `ci`.
fn cell_of(d: &Design, ci: usize, cref: &str) -> Option<usize> {
    d.resolve(ci, cref).or_else(|| {
        let (lib, name) = match cref.strip_prefix("./") {
            Some(n) => (d.cells[ci].lib.as_str(), n),
            None => cref.split_once(':')?,
        };
        d.cells.iter().position(|x| x.lib == lib && x.name == name)
    })
}

/// The view impl `imp` of cell `mi` goes back to: by its language when the cell has it.
fn view_of(d: &Design, mi: usize, imp: &str) -> Option<&'static str> {
    let c = &d.cells[mi];
    let lang = c
        .records(RecordKind::Impl)
        .find(|&r| c.field(r, 0) == imp)
        .map(|r| c.field(r, 1));
    view_of_impl(imp).or_else(|| lang.and_then(view_of_impl))
}

/// Instance path `/A/B` of bench `ci`: per segment, its master cell and Virtuoso name.
fn walk(d: &Design, ci: usize, path: &str) -> Option<Vec<(usize, String)>> {
    let mut cur = ci;
    let mut out = Vec::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let c = &d.cells[cur];
        let r = c
            .records(RecordKind::Inst)
            .find(|&r| c.attr(r, "name") == Some(seg))?;
        cur = d.resolve(cur, c.field(r, 1))?;
        out.push((
            cur,
            profile::inst_to_virtuoso(seg).unwrap_or_else(|| seg.to_owned()),
        ));
    }
    (!out.is_empty()).then_some(out)
}

/// Virtuoso libraries under bench `ci`: its own, then every master's (sorted).
fn libs(d: &Design, ci: usize, plan: &Plan) -> Vec<String> {
    let mut seen = vec![ci];
    let mut todo = vec![ci];
    while let Some(k) = todo.pop() {
        let c = &d.cells[k];
        for r in c.records(RecordKind::Inst) {
            if let Some(m) = d.resolve(k, c.field(r, 1))
                && !seen.contains(&m)
            {
                seen.push(m);
                todo.push(m);
            }
        }
    }
    let mut rest: Vec<String> = seen[1..].iter().map(|&k| plan.lib[k].clone()).collect();
    rest.sort();
    rest.dedup();
    rest.retain(|l| *l != plan.lib[ci]);
    std::iter::once(plan.lib[ci].clone()).chain(rest).collect()
}

fn generate(
    d: &Design,
    ci: usize,
    plan: &Plan,
    attrs: &Attrs,
    pieces: &[(&str, &str)],
    name: &str,
    notes: &mut Notes,
) -> String {
    let at = d.cells[ci].path.as_str();
    let (vlib, vcell) = (&plan.lib[ci], &plan.cell[ci]);
    let top = format!("({vlib}.{vcell}:schematic)");
    let mut t = format!("config {vcell};\ndesign {vlib}.{vcell}:schematic;\n");
    let kept: Vec<String> = pieces
        .iter()
        .filter(|p| p.0 == "keep")
        .map(|p| statement(p.1))
        .collect();
    if !kept.iter().any(|s| s.starts_with("liblist ")) {
        t.push_str(&format!("liblist {};\n", libs(d, ci, plan).join(", ")));
    }
    for s in &kept {
        t.push_str(&format!("{s};\n"));
    }
    let mut lost =
        |what: String| notes.review("config", at, format!("config {name}: {what}; not sent"));
    let views = |imps: &[String], lost: &mut dyn FnMut(String)| -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for i in imps
            .iter()
            .filter(|i| !i.contains(':') && !i.starts_with("./"))
        {
            match view_of_impl(i) {
                Some(x) if !v.contains(&x) => v.push(x),
                Some(_) => {}
                None => lost(format!("impl {i} has no Virtuoso view")),
            }
        }
        v
    };
    let vcellname = |k: usize| format!("{}.{}", plan.lib[k], plan.cell[k]);
    let mut rules = String::new();
    for (k, vals) in attrs {
        match (*k, vals.as_slice()) {
            ("order", imps) => {
                let v = views(imps, &mut lost);
                if !v.is_empty() {
                    t.push_str(&format!("viewlist {};\n", v.join(", ")));
                }
            }
            ("stop", words) => {
                let v = views(words, &mut lost);
                if !v.is_empty() {
                    t.push_str(&format!("stoplist {};\n", v.join(", ")));
                }
                for w in words
                    .iter()
                    .filter(|w| w.contains(':') || w.starts_with("./"))
                {
                    match cell_of(d, ci, w) {
                        Some(m) => rules.push_str(&format!("cell {} stop;\n", vcellname(m))),
                        None => lost(format!("stop {w}: no such cell")),
                    }
                }
            }
            ("cell", [cr, imp]) => match cell_of(d, ci, cr)
                .and_then(|m| Some((m, view_of(d, m, imp)?)))
            {
                Some((m, v)) => rules.push_str(&format!("cell {} binding :{v};\n", vcellname(m))),
                None => lost(format!("cell {cr} {imp}: no such cell or view")),
            },
            ("cellinst", [cr, inst, imp]) => {
                // The instance's master gives the view.
                let found = cell_of(d, ci, cr).and_then(|m| {
                    let c = &d.cells[m];
                    let r = c
                        .records(RecordKind::Inst)
                        .find(|&r| c.attr(r, "name") == Some(inst.as_str()))?;
                    let mm = d.resolve(m, c.field(r, 1))?;
                    Some((m, view_of(d, mm, imp)?))
                });
                match found {
                    Some((m, v)) => {
                        let vi = profile::inst_to_virtuoso(inst).unwrap_or_else(|| inst.clone());
                        rules.push_str(&format!(
                            "inst ({}:schematic).{vi} binding :{v};\n",
                            vcellname(m)
                        ));
                    }
                    None => lost(format!(
                        "cellinst {cr} {inst} {imp}: no such instance or view"
                    )),
                }
            }
            ("inst", [path, imp]) => {
                match walk(d, ci, path).and_then(|w| {
                    let last = w.last().expect("non-empty").0;
                    Some((view_of(d, last, imp)?, w))
                }) {
                    Some((v, w)) => {
                        let mut p = top.clone();
                        for (k, (m, n)) in w.iter().enumerate() {
                            if k + 1 < w.len() {
                                p.push_str(&format!(".({}:schematic){n}", vcellname(*m)));
                            } else {
                                p.push_str(&format!(".{n}"));
                            }
                        }
                        let last = w.last().expect("non-empty").0;
                        rules.push_str(&format!("path {p} binding {}:{v};\n", vcellname(last)));
                    }
                    None => lost(format!("inst {path} {imp}: no such path or view")),
                }
            }
            _ => {}
        }
    }
    t.push_str(&rules);
    t.push_str("endconfig\n");
    t
}

/// Points the maestro test of bench `c` (`records`, its `maestro` record first) at the
/// config view the bench header `config` names, when the bench has that config.
pub fn bind_test(c: &Cell, records: &mut [Record]) {
    let Some(name) = c.header("config") else {
        return;
    };
    let has = c.records(RecordKind::Config).any(|r| c.field(r, 0) == name);
    if let Some(m) = records
        .first_mut()
        .filter(|m| m.tag == "maestro" && m.fields.len() > 3 && has)
    {
        m.fields[3] = Value::Str(name.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = "//Revision 1\nconfig hier;\ndesign sfyprobe_lib.hier:schematic;\nliblist sfyprobe_lib, analogLib, basic;\n\nviewlist spectre, cmos_sch, schematic, veriloga;\nstoplist spectre;\n\n\ncell sfyprobe_lib.sub viewlist schematic, symbol;\ninst (sfyprobe_lib.hier:schematic).I1 liblist sfyprobe_lib;\ninst (sfyprobe_lib.hier:schematic).I1 binding :schematic;\ninst (sfyprobe_lib.hier:schematic).I0 stop;\n\ncell analogLib.res binding :spectre;\n\npath (sfyprobe_lib.hier:schematic).(sfyprobe_lib.sub:schematic)I1.RS0\n    binding analogLib.res:spectre;\n\nendconfig\n";

    #[test]
    fn every_statement_form_of_probe_config_parses() {
        let p = pieces(PROBE);
        assert_eq!(p.iter().map(|x| x.0).collect::<String>(), PROBE);
        let s: Vec<Stmt> = p.iter().map(|x| parse(&x.1)).collect();
        let st = |a: &str| a.to_owned();
        assert_eq!(s[0], Stmt::Header);
        assert_eq!(
            s[1],
            Stmt::Design(st("sfyprobe_lib"), st("hier"), st("schematic"))
        );
        assert_eq!(s[2], Stmt::Other);
        assert_eq!(
            s[3],
            Stmt::ViewList(vec![
                st("spectre"),
                st("cmos_sch"),
                st("schematic"),
                st("veriloga")
            ])
        );
        assert_eq!(s[4], Stmt::StopList(vec![st("spectre")]));
        assert_eq!(s[5], Stmt::Cell(st("sfyprobe_lib"), st("sub"), Rule::Other));
        assert_eq!(
            s[6],
            Stmt::Inst(st("sfyprobe_lib"), st("hier"), st("I1"), Rule::Other)
        );
        assert_eq!(
            s[7],
            Stmt::Inst(
                st("sfyprobe_lib"),
                st("hier"),
                st("I1"),
                Rule::Bind(None, st("schematic"))
            )
        );
        assert_eq!(
            s[8],
            Stmt::Inst(st("sfyprobe_lib"), st("hier"), st("I0"), Rule::Stop)
        );
        assert_eq!(
            s[9],
            Stmt::Cell(st("analogLib"), st("res"), Rule::Bind(None, st("spectre")))
        );
        assert_eq!(
            s[10],
            Stmt::Path(
                st("sfyprobe_lib"),
                st("hier"),
                vec![st("I1"), st("RS0")],
                Rule::Bind(Some((st("analogLib"), st("res"))), st("spectre"))
            )
        );
        assert_eq!(s[11], Stmt::Header);
        assert_eq!(s.len(), 12);
    }

    #[test]
    fn a_semicolon_in_a_comment_does_not_end_a_statement() {
        let t = "// a; b\nconfig x; // c; d\nendconfig\n";
        let p = pieces(t);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].1, "config x");
        assert_eq!(p[1].1, "endconfig");
    }
}
