//! The dump tables → `.sfyx` records in §4.2 order ([`records`], spec `interop.md` §4.2):
//! sorted per kind, refs assigned after sorting, so the same tables always give the same
//! records.

use super::*;

/// The `cdf`, `param` and `siminfo` records of `cdf` (§4.2): params in CDF order, siminfo
/// as stored (the dump writes it sorted by simulator).
pub fn cdf_records(cdf: &Cdf) -> Vec<Record> {
    let mut out = vec![Record::new("cdf", vec![sfyx::text(&cdf.label_set)])];
    for (k, v) in &cdf.attrs {
        out.push(Record::new("cdfattr", vec![s(k), s(v)]));
    }
    for p in &cdf.params {
        out.push(Record::new(
            "param",
            vec![
                s(&p.name),
                s(&p.ty),
                s(&p.def),
                s(&p.prompt),
                s(&p.units),
                Value::List(p.flags.iter().map(|f| s(f)).collect()),
                Value::List(p.choices.iter().map(|f| s(f)).collect()),
                sfyx::text(&p.callback),
            ],
        ));
        for (k, v) in &p.attrs {
            out.push(Record::new("paramattr", vec![s(&p.name), s(k), s(v)]));
        }
    }
    for si in &cdf.siminfo {
        let fields = si
            .fields
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    SimVal::Str(x) => s(x),
                    SimVal::List(l) => Value::List(l.iter().map(|x| s(x)).collect()),
                    SimVal::Pairs(p) => pairs(p.iter().map(|(a, b)| (a.as_str(), b.as_str()))),
                };
                Value::List(vec![s(k), v])
            })
            .collect();
        out.push(Record::new(
            "siminfo",
            vec![s(&si.sim), Value::List(fields)],
        ));
    }
    debug_assert_eq!(
        out.len(),
        1 + cdf.attrs.len()
            + cdf.params.iter().map(|p| 1 + p.attrs.len()).sum::<usize>()
            + cdf.siminfo.len()
    );
    out
}

/// The CDF a cell from Virtuoso had, when its interface (pins, params, impls) is unchanged
/// since the import (`vcdf` fingerprint, §7.5): `Some(None)` for a cell that had no CDF,
/// `Some(Some(cdf))` for the stored records (`vcdfrec`), `None` when the cell is not from
/// Virtuoso or was edited (export then derives the CDF, §6.3).
pub(super) fn cdf_as_imported(c: &Cell) -> Option<Option<Cdf>> {
    let fp = c.header(crate::virtuoso::fingerprint_attr(
        crate::virtuoso::Part::Interface,
    ))?;
    if fp != crate::virtuoso::fingerprint(&c.doc, crate::virtuoso::Part::Interface) {
        return None;
    }
    Some(stashed_cdf(c))
}

/// The CDF records the import stored (`vcdfrec`), whatever the cell's edits since; `None`
/// when there are none.
pub(super) fn stashed_cdf(c: &Cell) -> Option<Cdf> {
    let recs: Vec<&str> = (0..c.doc.attrs(0).len() as u32)
        .map(|k| c.doc.attrs(0).start + k)
        .filter(|&a| c.doc.attr_key(a) == "vcdfrec")
        .filter_map(|a| c.doc.attr_value(a, 0))
        .collect();
    if recs.is_empty() {
        return None;
    }
    let text = format!(
        "(\"sfyx\" 1 \"vcdf\")\n(\"lib\" \"l\")\n(\"cell\" \"c\" \"design\")\n{}\n(\"end\")\n",
        recs.join("\n")
    );
    let cdf = sfyx::read(&text)
        .ok()
        .and_then(|r| read::dump(&r).ok())
        .and_then(|d| d.cells.into_iter().next())
        .and_then(|c| c.cdf)?;
    Some(cdf)
}

/// The records of `d` in §4.2 order: header; per library (sorted), its `ref` cells then
/// its `design` cells (each sorted by name) with CDF, cellviews (sorted by view, objects
/// in kind order, each kind sorted by name, else coordinates, else text), text views and
/// config views;
/// maestro records; `end`. Refs are renumbered 1… in emission order, so the same tables
/// always give the same records.
pub fn records(d: &Dump) -> Vec<Record> {
    let mut out = vec![Record::new("sfyx", vec![i(1), s(&d.producer)])];
    let mut order: Vec<&DCell> = d.cells.iter().collect();
    order.sort_by(|a, b| (&a.lib, a.design, &a.name).cmp(&(&b.lib, b.design, &b.name)));
    let mut lib: Option<&str> = None;
    for c in order {
        if lib != Some(c.lib.as_str()) {
            out.push(Record::new("lib", vec![s(&c.lib)]));
            lib = Some(&c.lib);
        }
        out.push(Record::new(
            "cell",
            vec![s(&c.name), s(if c.design { "design" } else { "ref" })],
        ));
        if let Some(cdf) = &c.cdf {
            out.extend(cdf_records(cdf));
        }
        let mut cvs: Vec<&Cv> = c.cvs.iter().collect();
        cvs.sort_by(|a, b| a.view.cmp(&b.view));
        for cv in cvs {
            cv_records(cv, &mut out);
        }
        let mut tvs: Vec<&TextView> = c.textviews.iter().collect();
        tvs.sort_by(|a, b| a.view.cmp(&b.view));
        for t in tvs {
            out.push(Record::new(
                "textview",
                vec![s(&t.view), s(&t.file), sfyx::text(&t.text)],
            ));
        }
        let mut cfgs: Vec<_> = c.configs.iter().collect();
        cfgs.sort_by(|a, b| a.view.cmp(&b.view));
        for x in cfgs {
            out.push(Record::new("config", vec![s(&x.view), sfyx::text(&x.text)]));
        }
    }
    // Maestro tests by (cell, test name), as the SKILL dumper writes them (§4.2).
    let mut tests: Vec<&Vec<Record>> = d.maestro.iter().collect();
    let key = |t: &Vec<Record>| -> (String, String) {
        let r = &t[0];
        (
            r.str(2).unwrap_or("").to_owned(),
            r.str(0).unwrap_or("").to_owned(),
        )
    };
    tests.sort_by_key(|t| key(t));
    for test in tests {
        out.extend(test.iter().cloned());
    }
    out.push(Record::new("end", vec![]));
    out
}

/// Marks `cv` with `schemifyKeep` when `part` of `c` is unchanged since it was imported
/// (its fingerprint equals the header attribute import wrote, §7.5).
pub(super) fn keep(c: &Cell, part: crate::virtuoso::Part, mut cv: Cv) -> Cv {
    if c.header(crate::virtuoso::fingerprint_attr(part))
        == Some(crate::virtuoso::fingerprint(&c.doc, part).as_str())
    {
        cv.props.push(Prop::string(0, "schemifyKeep", "1"));
    }
    cv
}

/// A wire's points as written: a two-point wire from its smaller end (x, then y), as the
/// SKILL dumper writes it, so drawing direction never shows as a difference (§4.2).
fn canon(p: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut v = p.to_vec();
    if v.len() == 2 && v[1] < v[0] {
        v.swap(0, 1);
    }
    v
}

pub(super) fn pts(v: &[[i32; 2]]) -> Value {
    Value::List(v.iter().flat_map(|p| [i(p[0]), i(p[1])]).collect())
}

fn cv_records(cv: &Cv, out: &mut Vec<Record>) {
    out.push(Record::new(
        "cv",
        vec![s(&cv.view), s(&cv.vtype), i(cv.upi)],
    ));
    let mut terms: Vec<&Term> = cv.terms.iter().collect();
    terms.sort_by(|a, b| a.name.cmp(&b.name));
    for t in terms {
        out.push(Record::new(
            "term",
            vec![s(&t.name), s(&t.dir), s(&t.netexpr)],
        ));
    }
    // Old ref → new ref.
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    let mut next = 1u32;
    let mut renum = |old: u32, map: &mut BTreeMap<u32, u32>| -> u32 {
        let n = next;
        next += 1;
        let prev = map.insert(old, n);
        assert!(prev.is_none(), "provisional ref {old} used twice");
        n
    };
    let mut pf: Vec<&PinFig> = cv.pinfigs.iter().collect();
    pf.sort_by(|a, b| (&a.term, a.b).cmp(&(&b.term, b.b)));
    let pf: Vec<(u32, &PinFig)> = pf.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut ins: Vec<&Inst> = cv.insts.iter().collect();
    ins.sort_by(|a, b| (&a.name, a.xy, &a.lib, &a.cell).cmp(&(&b.name, b.xy, &b.lib, &b.cell)));
    let ins: Vec<(u32, &Inst)> = ins.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut sp: Vec<&Spin> = cv.spins.iter().collect();
    sp.sort_by(|a, b| (&a.term, a.xy).cmp(&(&b.term, b.xy)));
    let sp: Vec<(u32, &Spin)> = sp.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut wi: Vec<&Wire> = cv.wires.iter().collect();
    wi.sort_by_key(|w| (canon(&w.pts), w.width));
    let wi: Vec<(u32, &Wire)> = wi.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut la: Vec<&Label> = cv.labels.iter().collect();
    la.sort_by(|a, b| {
        (a.xy, &a.text, &a.layer, &a.purpose, &a.kind, &a.orient)
            .cmp(&(b.xy, &b.text, &b.layer, &b.purpose, &b.kind, &b.orient))
    });
    let la: Vec<(u32, &Label)> = la.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut sh: Vec<&Shape> = cv.shapes.iter().collect();
    sh.sort_by(|a, b| {
        (&a.coords, &a.kind, &a.layer, &a.purpose).cmp(&(&b.coords, &b.kind, &b.layer, &b.purpose))
    });
    let sh: Vec<(u32, &Shape)> = sh.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let rr = |old: u32| -> u32 {
        if old == 0 {
            0
        } else {
            *map.get(&old)
                .unwrap_or_else(|| panic!("unknown provisional ref {old}"))
        }
    };
    for (r, p) in pf {
        let mut f = vec![
            i(r as i32),
            s(&p.term),
            i(p.b[0]),
            i(p.b[1]),
            i(p.b[2]),
            i(p.b[3]),
        ];
        if !p.dir.is_empty() {
            f.push(s(&p.dir));
        }
        out.push(Record::new("pinfig", f));
    }
    for (r, x) in ins {
        out.push(Record::new(
            "inst",
            vec![
                i(r as i32),
                s(&x.name),
                s(&x.lib),
                s(&x.cell),
                s(&x.view),
                i(x.xy[0]),
                i(x.xy[1]),
                s(&x.orient),
            ],
        ));
    }
    for (r, x) in sp {
        out.push(Record::new(
            "spin",
            vec![
                i(r as i32),
                s(&x.term),
                s(&x.dir),
                s(&x.lib),
                s(&x.cell),
                s(&x.view),
                i(x.xy[0]),
                i(x.xy[1]),
                s(&x.orient),
            ],
        ));
    }
    for (r, w) in wi {
        out.push(Record::new(
            "wire",
            vec![i(r as i32), pts(&canon(&w.pts)), i(w.width)],
        ));
    }
    for (r, l) in la {
        let mut f = vec![
            i(r as i32),
            i(rr(l.parent) as i32),
            s(&l.text),
            i(l.xy[0]),
            i(l.xy[1]),
            s(&l.orient),
            s(&l.justify),
            s(&l.font),
            i(l.height),
            s(&l.kind),
            s(&l.layer),
            s(&l.purpose),
        ];
        if l.overbar || l.undrafted || l.hidden {
            f.extend([l.overbar, !l.undrafted, !l.hidden].map(|b| i(i32::from(b))));
        }
        out.push(Record::new("label", f));
    }
    for (r, x) in sh {
        out.push(Record::new(
            "shape",
            vec![
                i(r as i32),
                s(&x.kind),
                s(&x.layer),
                s(&x.purpose),
                Value::List(x.coords.iter().map(|v| i(*v)).collect()),
                i(x.width),
            ],
        ));
    }
    let mut tds: Vec<(u32, &TextDisplay)> = cv.tds.iter().map(|t| (rr(t.owner), t)).collect();
    tds.sort_by(|a, b| {
        (a.0, &a.1.assoc, &a.1.name, a.1.xy).cmp(&(b.0, &b.1.assoc, &b.1.name, b.1.xy))
    });
    for (r, t) in tds {
        let mut f = vec![
            i(r as i32),
            s(&t.assoc),
            s(&t.name),
            s(&t.layer),
            s(&t.purpose),
            i(t.xy[0]),
            i(t.xy[1]),
            s(&t.orient),
            s(&t.justify),
            s(&t.font),
            i(t.height),
        ];
        f.extend(t.flags.map(|b| i(i32::from(b))));
        out.push(Record::new("td", f));
    }
    let mut props: Vec<(u32, &Prop)> = cv.props.iter().map(|p| (rr(p.r), p)).collect();
    props.sort_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    for (r, p) in props {
        out.push(Record::new(
            "prop",
            vec![i(r as i32), s(&p.name), s(&p.ty), s(&p.value)],
        ));
    }
    let mut conns: Vec<(u32, &Conn)> = cv.conns.iter().map(|c| (rr(c.r), c)).collect();
    conns.sort_by(|a, b| (a.0, &a.1.term).cmp(&(b.0, &b.1.term)));
    for (r, c) in conns {
        out.push(Record::new(
            "conn",
            vec![i(r as i32), s(&c.term), s(&c.net)],
        ));
    }
}
