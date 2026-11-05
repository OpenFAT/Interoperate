//! `.sfyx` records → typed dump → Schemify files (spec `interop.md` §6, §7.3).
//!
//! Spine of [`import`]: `sfyx::read` → [`dump`] (records into per-kind tables, every
//! record checked, with its line) → interfaces of every dump cell (pins, params, simInfo
//! formats) → one fresh document per target file (`build`) → `ade::import` for benches
//! → `merge::merge` with the existing file or the export's stash → manifest rows → files.
//!
//! The dump model ([`Dump`], [`Cv`] and its rows) is shared with `write`, which builds the
//! same tables from a Design and serializes them.
//!
//! Submodules, one concern each: `dump` (records → tables), `cdf` (CDF, simInfo, §6.3),
//! `ctx` (the import's lookup tables and units), `symbol` (`build`: one cell's document and
//! its symbol), `schematic` (`schematic_in`), `stash` (cellview properties, §7.3) and
//! `labels` (label values and placement). This file keeps the entry points.

use std::collections::{BTreeMap, BTreeSet};

use schemify_format::{Document, FileKind, IdRng, Orient, RecordKind, fresh_id, is_id, parse};

use super::ade;
use super::ael::{self, AelName};
use super::devices;
use super::{Import, ImportOptions};
use crate::design::{
    Files, cell_path, copy_header, push_anchored, push_attr, push_lib, push_record,
};
use crate::map::{CORE_LIB, Mapping, ROOT_LIB};
use crate::merge;
use crate::notes::Notes;
use crate::profile::{self, coord_text, net_from_virtuoso, point_from_virtuoso};
use crate::sfyx::{self, Error, Record, Value};

// Data design (six questions): in = one `.sfyx` text (≤ 2^30 bytes, ≤ 2^24 records); out
// = one Schemify document per design/ref cell. Rows per kind live in `Vec`s per
// cellview (existence-based: a kind with no records has an empty table); refs are the
// dump's integers, unique per cellview, looked up through a sorted index. Lifetime: one
// import call. Access: each kind is walked once in dump order by the builder.

mod cdf;
mod ctx;
mod dump;
mod labels;
mod schematic;
mod stash;
mod symbol;
#[cfg(test)]
mod tests;

pub use cdf::*;
use ctx::*;
pub use dump::*;
pub(crate) use labels::*;
use schematic::*;
use stash::*;
pub(crate) use symbol::*;

/// The fresh documents of every written dump cell (after `ade::import`), their stash
/// texts and whether a maestro setup came with them.
struct Fresh {
    docs: Vec<Option<(Document<'static>, Option<String>, bool)>>,
}

fn fresh(ctx: &Ctx<'_>, notes: &mut Notes) -> Fresh {
    let mut docs = Vec::new();
    let mut pending: Vec<Vec<Pending>> = Vec::new();
    for k in 0..ctx.dump.cells.len() {
        let mut p = Vec::new();
        if !ctx.targets[k].write {
            docs.push(None);
            pending.push(p);
            continue;
        }
        let (mut doc, st) = build(ctx, k, &mut p, notes);
        pending.push(p);
        let c = &ctx.dump.cells[k];
        let tests: Vec<&[Record]> = ctx
            .dump
            .maestro
            .iter()
            .filter(|t| {
                t[0].str(1).ok() == Some(c.lib.as_str())
                    && t[0].str(2).ok() == Some(c.name.as_str())
            })
            .map(Vec::as_slice)
            .collect();
        let maestro = !tests.is_empty();
        if maestro {
            ade::import(&tests, &mut doc, &c.name, notes);
        }
        let dir = ctx.targets[k].dir.as_str();
        let cellref = |l: &str, cl: &str| ctx.master(l, cl).cellref(dir);
        let doc = super::config::import(doc, c, ctx.dump, &cellref, notes);
        docs.push(Some((doc, st, maestro)));
    }
    if pending.iter().any(|p| !p.is_empty()) {
        remaster(ctx, &mut docs, &pending, notes);
    }
    check_connectivity(ctx, &docs, notes);
    Fresh { docs }
}

/// The project as an import leaves it: the existing files, every fresh document and a
/// manifest naming every library they land in (what `load_closed` resolves against).
fn fresh_files(ctx: &Ctx<'_>, docs: &[Option<(Document<'static>, Option<String>, bool)>]) -> Files {
    let mut files: Files = ctx.existing.clone();
    for (k, d) in docs.iter().enumerate() {
        if let Some((doc, _, _)) = d {
            files.insert(
                ctx.targets[k].path(),
                schemify_format::write(doc).into_bytes(),
            );
        }
    }
    let mut manifest = Document::new(FileKind::Project);
    for (l, d) in &ctx.libs {
        push_lib(&mut manifest, l, d);
    }
    for t in ctx
        .targets
        .iter()
        .filter(|t| t.write && !t.slib.is_empty() && !ctx.libs.contains_key(&t.slib))
    {
        if (1..manifest.record_count()).all(|r| manifest.field(r, 0) != Some(t.slib.as_str())) {
            push_lib(&mut manifest, &t.slib, &t.dir);
        }
    }
    files.insert(
        "schemify.project".into(),
        schemify_format::write(&manifest).into_bytes(),
    );
    files
}

/// The connectivity oracle (S1): every schematic whose dump carries Virtuoso's own
/// instance terminal → net table (`tnet`) is elaborated here (`nets::extract`) and the two
/// partitions of instance pins compared; each difference is a `needs_review:connectivity`
/// row, so a geometric rule the import models differently from Virtuoso is never silent.
// ponytail: bus and iterated terminals (`<`, `,` in a name) and pins renamed by the
// analogLib re-mastering are not compared; a per-bit table when a dump needs them.
fn check_connectivity(
    ctx: &Ctx<'_>,
    docs: &[Option<(Document<'static>, Option<String>, bool)>],
    notes: &mut Notes,
) {
    use crate::design::{geom, load_closed, sym_pins};
    use std::collections::BTreeMap as Map;
    let todo: Vec<usize> = (0..docs.len())
        .filter(|&k| docs[k].is_some())
        .filter(|&k| {
            ctx.dump.cells[k]
                .cv("schematic")
                .is_some_and(|s| !s.tnets.is_empty())
        })
        .collect();
    if todo.is_empty() {
        return;
    }
    let files = fresh_files(ctx, docs);
    let plain = |s: &str| !s.contains(['<', ',']);
    for k in todo {
        let sv = ctx.dump.cells[k].cv("schematic").expect("filtered");
        let path = ctx.targets[k].path();
        let d = match load_closed(&files, &[path.as_str()]) {
            Ok(d) => d,
            Err(p) => {
                notes.review(
                    "connectivity",
                    &path,
                    format!("not compared with Virtuoso's: {}", p.join("; ")),
                );
                continue;
            }
        };
        let ci = d.find(&path).expect("loaded");
        let g = geom(&d, ci);
        let n = crate::nets::extract(&d, ci, &g);
        // (instance.pin, Schemify net, Virtuoso net or None: unconnected there). A Schemify
        // net is its name: labels of one name are one net (elab §9.2).
        let mut rows: Vec<(String, &str, Option<&str>)> = Vec::new();
        for inst in sv.insts.iter().filter(|i| plain(&i.name)) {
            let name = profile::inst_from_virtuoso(&inst.name);
            let Some(gi) = g.inst_name.iter().position(|x| *x == name) else {
                continue; // a global label or a noconn now
            };
            let pins = sym_pins(&d.cells[g.inst_master[gi] as usize]);
            let tn: Vec<&Conn> = sv.tnets.iter().filter(|t| t.r == inst.r).collect();
            if tn
                .iter()
                .any(|t| !pins.name.contains(&net_from_virtuoso(&t.term).text))
            {
                continue; // re-mastered onto other pin names
            }
            for (p, pn) in pins.name.iter().enumerate() {
                let node = n.inst_pin[gi][p];
                if node == crate::nets::NO_NODE || pins.alias[p].is_some() {
                    continue;
                }
                let v = tn.iter().find(|t| net_from_virtuoso(&t.term).text == *pn);
                if v.is_some_and(|t| !plain(&t.term) || !plain(&t.net)) {
                    continue;
                }
                rows.push((
                    format!("{}.{pn}", inst.name),
                    n.net_name[n.node_net[node as usize] as usize].as_str(),
                    v.map(|t| t.net.as_str()),
                ));
            }
        }
        let mut by_v: Map<&str, Vec<usize>> = Map::new();
        let mut by_s: Map<&str, Vec<usize>> = Map::new();
        for (i, r) in rows.iter().enumerate() {
            if let Some(v) = r.2 {
                by_v.entry(v).or_default().push(i);
            }
            by_s.entry(r.1).or_default().push(i);
        }
        let pins_of = |ix: &[usize]| ix.iter().map(|&i| rows[i].0.as_str()).collect::<Vec<_>>();
        for (v, ix) in &by_v {
            let mut nets: Vec<&str> = ix.iter().map(|&i| rows[i].1).collect();
            nets.sort_unstable();
            nets.dedup();
            if nets.len() > 1 {
                notes.review(
                    "connectivity",
                    &path,
                    format!(
                        "Virtuoso's net {v} ({}) is {} nets here",
                        pins_of(ix).join(" "),
                        nets.len()
                    ),
                );
            }
        }
        for (s, ix) in &by_s {
            let mut vs: Vec<Option<&str>> = ix.iter().map(|&i| rows[i].2).collect();
            vs.sort_unstable();
            vs.dedup();
            if ix.len() > 1 && (vs.len() > 1 || vs[0].is_none()) {
                notes.review(
                    "connectivity",
                    &path,
                    format!(
                        "net {} joins pins Virtuoso keeps apart: {}",
                        s,
                        ix.iter()
                            .map(|&i| format!("{} ({})", rows[i].0, rows[i].2.unwrap_or("open")))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                );
            }
        }
    }
}

/// The Schemify-side re-mastering pass (§6.4): loads the fresh documents (with the
/// project's files and the core library), pulls wires back from the analogLib mirror
/// pins, labels them, and moves each pending instance onto its core cell with stubs and
/// labels at the core pins.
fn remaster(
    ctx: &Ctx<'_>,
    docs: &mut [Option<(Document<'static>, Option<String>, bool)>],
    pending: &[Vec<Pending>],
    notes: &mut Notes,
) {
    use crate::design::{geom, load_closed, sym_pins};
    let files = fresh_files(ctx, docs);
    for (k, todo) in pending.iter().enumerate() {
        if todo.is_empty() {
            continue;
        }
        let path = ctx.targets[k].path();
        let d = match load_closed(&files, &[path.as_str()]) {
            Ok(d) => d,
            Err(p) => {
                let why = format!("re-mastering skipped: {}", p.join("; "));
                notes.review("name", &path, why);
                continue;
            }
        };
        let ci = d.find(&path).expect("loaded");
        let g = geom(&d, ci);
        let n = crate::nets::extract(&d, ci, &g);
        let mut rem = Vec::new();
        let mut masters = Vec::new();
        for p in todo {
            let Some(inst) = g.inst_id.iter().position(|x| *x == p.id) else {
                continue;
            };
            let core = format!("{}.shprim", devices::DEVICES[p.device].core);
            let new_pins = schemify_doc::core_library()
                .iter()
                .find(|(nm, _)| *nm == core)
                .and_then(|(_, b)| Ctx::iface_of_text(b))
                .map(|i| {
                    i.pins
                        .into_iter()
                        .map(|q| (q.name, q.at))
                        .collect::<Vec<_>>()
                });
            let old_pins = sym_pins(&d.cells[g.inst_master[inst] as usize]);
            let new_names: Vec<String> = new_pins
                .as_ref()
                .map_or(Vec::new(), |v| v.iter().map(|x| x.0.clone()).collect());
            let pairs: Vec<(usize, String)> = (0..old_pins.name.len())
                .filter_map(|q| new_names.get(q).map(|nm| (q, nm.clone())))
                .collect();
            // Pin-pattern alignment (§6.4): the core cell goes where its pins land on the
            // Virtuoso master's, in the instance's orientation.
            let at = new_pins.as_ref().and_then(|np| {
                let (o, at) = (g.inst_orient[inst], g.inst_at[inst]);
                crate::align::fit_instance(&old_pins.at, &pairs, np, o, at)
            });
            rem.push(crate::remap::Remaster {
                inst,
                pins: pairs,
                new_pins,
                at,
            });
            masters.push((
                inst,
                p.cellref.clone(),
                Some(
                    p.attrs
                        .iter()
                        .map(|(a, v)| (a.clone(), vec![v.clone()]))
                        .collect(),
                ),
            ));
        }
        let e = crate::remap::apply(&g, &n, &rem);
        let (doc, st, m) = docs[k].take().expect("pending cells are built");
        let mut taken: BTreeSet<String> = (1..doc.record_count())
            .filter(|&r| doc.record_kind(r).key() == schemify_format::KeyRule::Id)
            .filter_map(|r| doc.field(r, 0).map(str::to_owned))
            .collect();
        let seed = format!("{}/{}/remap", ctx.dump.cells[k].lib, ctx.dump.cells[k].name);
        let mut ord = 0u32;
        let new = crate::remap::apply_to_doc(&d.cells[ci].doc, &g, &e, &masters, || {
            ord += 1;
            let mut rng = IdRng::for_import(&seed, ord);
            let id = fresh_id(&mut rng, |c| taken.contains(c))
                .as_str()
                .to_owned();
            taken.insert(id.clone());
            id
        });
        docs[k] = Some((new, st, m));
    }
}

/// Rows for what a dump names but does not carry: views other than the cellviews and text
/// views it holds (gaps2 S4), a library's CDF and its libInit.il (S10).
fn report_left_out(d: &Dump, notes: &mut Notes) {
    for c in &d.cells {
        for (v, file) in &c.skipviews {
            notes.review(
                "format",
                &format!("{}/{}", c.lib, c.name),
                format!("view {v} ({file}) is not in the dump: not imported"),
            );
        }
    }
    for (lib, n, init) in &d.libcdfs {
        if *n > 0 {
            notes.review(
                "callback",
                lib,
                format!("the library CDF ({n} params) is not carried"),
            );
        }
        if *init {
            notes.review(
                "callback",
                lib,
                "libInit.il is not carried: a library without it lacks its procedures",
            );
        }
    }
}

/// See [`super::import_targets`].
pub fn targets(sfyx: &str) -> Result<Vec<String>, Vec<String>> {
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    let d = dump(&recs).map_err(|e| vec![e.to_string()])?;
    let opts = ImportOptions::default();
    let mut out: BTreeSet<String> =
        ["schemify.project".to_owned(), "interop.map".to_owned()].into();
    for c in &d.cells {
        let slib = slib_of(&c.lib, &opts);
        if slib == "core" || consumed(&c.lib, &c.name) {
            continue;
        }
        for ext in ["shcomp", "shprim", "shbench"] {
            out.insert(cell_path(&slib, &c.name, ext));
        }
    }
    Ok(out.into_iter().collect())
}

/// See [`super::import`].
pub fn import(
    sfyx: &str,
    existing: &Files,
    map: &Mapping,
    opts: &ImportOptions,
) -> Result<Import, Vec<String>> {
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    let d = dump(&recs).map_err(|e| vec![e.to_string()])?;
    let mut notes = Notes::default();
    for r in &d.unknown {
        notes.dropped(
            "unknown-record",
            "-",
            format!("`{}` on line {}", r.tag, r.line),
        );
    }
    report_left_out(&d, &mut notes);
    let ctx = Ctx::new(&d, existing, map, opts, &mut notes);
    let fr = fresh(&ctx, &mut notes);
    group_kit_callbacks(&mut notes);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut new_libs: BTreeMap<String, String> = BTreeMap::new();
    let manifest = existing
        .get("schemify.project")
        .and_then(|m| crate::design::manifest_libs(m).ok())
        .unwrap_or_default();
    for (k, f) in fr.docs.into_iter().enumerate() {
        let Some((doc, st, maestro)) = f else {
            continue;
        };
        let t = &ctx.targets[k];
        let path = t.path();
        let cell = path.clone();
        let local = existing
            .get(&path)
            .and_then(|b| merge::parse_stash(std::str::from_utf8(b).ok()?, doc.kind()).ok());
        let stash = st.and_then(|s| match merge::parse_stash(&s, doc.kind()) {
            Ok(d) => Some(d),
            Err(e) => {
                notes.review("format", &cell, format!("stash ignored: {e}"));
                None
            }
        });
        let out = merge::merge(
            &doc,
            local.as_ref(),
            stash.as_ref(),
            maestro,
            &cell,
            &mut notes,
        );
        let out = with_fingerprints(&out, &doc, ctx.dump.cells[k].cdf.as_ref());
        let text = schemify_format::write(&out);
        if let Some(old) = existing.get(&path)
            && old.as_slice() != text.as_bytes()
        {
            notes.add("updated", &cell, "the import changed this file");
        }
        if !t.slib.is_empty() && !manifest.contains_key(&t.slib) {
            new_libs.insert(t.slib.clone(), t.dir.clone());
        }
        files.push((path, text.into_bytes()));
    }
    if !new_libs.is_empty() {
        files.push((
            "schemify.project".into(),
            manifest_with(existing.get("schemify.project"), &new_libs).into_bytes(),
        ));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        files.windows(2).all(|w| w[0].0 < w[1].0),
        "one file per path"
    );
    Ok(Import { files, notes })
}

/// `doc` (the merged file) with header attributes `vsym`/`vsch` set to the fingerprints of
/// the parts its kind has, taken from `fresh` (what the dump holds, before local data was
/// merged, §7.5): an export can then tell a cellview nobody changed on the Schemify side.
fn with_fingerprints(
    doc: &Document<'static>,
    fresh: &Document<'static>,
    cdf: Option<&Cdf>,
) -> Document<'static> {
    use super::{Part, fingerprint, fingerprint_attr};
    let parts: &[Part] = match doc.kind() {
        FileKind::Shprim => &[Part::Symbol],
        FileKind::Shcomp => &[Part::Symbol, Part::Schematic, Part::Interface],
        _ => &[Part::Schematic],
    };
    let mut out = Document::new(doc.kind());
    copy_header(&mut out, doc, &["vsym", "vsch", "vcdf", "vcdfrec"]);
    for p in parts {
        out.push_attr(fingerprint_attr(*p));
        out.push_value(&fingerprint(fresh, *p));
    }
    // The reverse stash of the CDF (§7.5): a component's CDF records as Virtuoso had them,
    // for export to send back while `vcdf` still matches (none: the cell had no CDF).
    if doc.kind() == FileKind::Shcomp {
        for r in cdf.map(super::write::cdf_records).unwrap_or_default() {
            let mut s = String::new();
            sfyx::write_record(&r, &mut s);
            out.push_attr("vcdfrec");
            out.push_value(s.trim_end());
        }
    }
    for r in 1..doc.record_count() {
        out.copy_record(doc, r);
    }
    assert_eq!(out.record_count(), doc.record_count());
    out
}

/// The manifest with `lib <name> path <dir>` rows added for `new`.
fn manifest_with(old: Option<&Vec<u8>>, new: &BTreeMap<String, String>) -> String {
    let mut doc = match old.and_then(|b| parse(b).ok()) {
        Some(src) => crate::design::owned_doc(&src),
        None => Document::new(FileKind::Project),
    };
    for (l, d) in new {
        push_lib(&mut doc, l, d);
    }
    schemify_format::write(&doc)
}

/// The fresh documents an import of `records` produces (no merge), keyed by (Virtuoso
/// library, cell): the export's stash oracle. Libraries map back through the inverse of
/// the export's renames; masters outside the dump resolve through `d`'s files.
pub(crate) fn image(
    records: &[Record],
    plan: &crate::map::Plan,
    d: &crate::design::Design,
    map: &Mapping,
) -> BTreeMap<(String, String), Document<'static>> {
    let dump = dump(records).expect("the exporter writes valid records");
    let mut iopts = ImportOptions::default();
    for (ci, c) in d.cells.iter().enumerate() {
        if plan.how[ci] == crate::map::How::Design {
            iopts.lib_map.insert(plan.lib[ci].clone(), c.lib.clone());
        }
    }
    iopts.lib_dir = d.libs.clone();
    let mut files: Files = Files::new();
    let mut manifest = Document::new(FileKind::Project);
    for (l, dir) in &d.libs {
        push_lib(&mut manifest, l, dir);
    }
    files.insert(
        "schemify.project".into(),
        schemify_format::write(&manifest).into_bytes(),
    );
    for c in &d.cells {
        if c.lib != "core" {
            files.insert(c.path.clone(), c.text().into_bytes());
        }
    }
    let mut notes = Notes::default();
    let ctx = Ctx::new(&dump, &files, map, &iopts, &mut notes);
    let fr = fresh(&ctx, &mut notes);
    let mut out = BTreeMap::new();
    for (k, f) in fr.docs.into_iter().enumerate() {
        if let Some((doc, _, _)) = f {
            out.insert((dump.cells[k].lib.clone(), dump.cells[k].name.clone()), doc);
        }
    }
    out
}
