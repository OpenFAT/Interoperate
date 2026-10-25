//! Design → `.sfyx` records (spec `interop.md` §6, §7.3).
//!
//! Spine of [`export`]: `plan::resolve` → per exported cell, `cell_out` builds the dump
//! tables ([`DCell`]: CDF, symbol and schematic cellviews, text views; `remap::apply` edits
//! for re-mastered instances) → `ade::export` per bench → the image import of those
//! records (`read::image`) → `merge::stash` per cell (Schemify-only data and every record
//! the image does not reproduce) → [`records`] (sorted, refs assigned) → `sfyx::write`.

use std::collections::BTreeMap;

use schemify_format::{FileKind, Orient, RecordKind, parse_coord};

use super::ael::{self, Name};
use super::devices;
use super::dump::*;
// The export runs the import to know what round-trips (the stash image, §7.3).
use super::read;
use super::{Export, ExportOptions, ade};
use crate::design::{Cell, Design, geom, sym_pins};
use crate::notes::Notes;
use crate::virtuoso::align;
use crate::virtuoso::map::Mapping;
use crate::virtuoso::merge;
use crate::virtuoso::nets;
use crate::virtuoso::plan::{self, How, Plan};
use crate::virtuoso::profile::{
    self, MILLI_PER_INCH, coord_text, dir_to_virtuoso, net_from_virtuoso, net_to_virtuoso, norm_box,
};
use crate::virtuoso::remap::{self, Remaster, STEP};
use crate::virtuoso::sfyx::{self};

mod interface;
mod schematic;
mod symbol;
pub(crate) use interface::*;
use schematic::*;
pub(crate) use symbol::*;

/// Units per inch the exporter writes (§3.1): one unit is one Schemify milli-unit.
pub const UPI: i32 = MILLI_PER_INCH as i32;

/// Half the side of an exported symbol pin rectangle (1/80 inch).
pub const PIN_HALF: i32 = 2_000;

/// Width of an exported bus wire (3/80 inch, on Virtuoso's 2-DBU width grid: 1/32 inch came
/// back rounded up to this on a live run); 0 is a narrow wire.
pub const BUS_WIDTH: i32 = 6_000;

/// Height of exported wire and pin-name labels (1/16 inch).
pub const LABEL_HEIGHT: i32 = 10_000;

/// Default `size` of a `stext` and of a `text` when the record has none (the editor's).
pub const STEXT_SIZE_MILLI: i32 = 300;
/// See [`STEXT_SIZE_MILLI`].
pub const TEXT_SIZE_MILLI: i32 = 400;

/// Bytes per `schemifyRecords<k>` property (§7.3).
pub const STASH_CHUNK: usize = 4000;

/// Core params that only shape the ngspice netlist: never sent to an analogLib master and
/// never reported (§6.5).
const NGSPICE_ONLY: [&str; 1] = ["savecurrent"];

/// Virtuoso DBU per user unit the off-grid check assumes (§3.1).
const DBU: i32 = 160;

/// See [`super::export`].
pub fn export(
    d: &Design,
    tops: &[usize],
    m: &Mapping,
    opts: &ExportOptions,
) -> Result<Export, Vec<String>> {
    for &t in tops {
        if t >= d.cells.len() {
            return Err(vec![format!("top {t} is not a cell of the design")]);
        }
    }
    let plan = plan::resolve(d, tops, m, opts.analoglib, &opts.lib_map)?;
    let mut notes = Notes::default();
    let mut dump = Dump {
        producer: super::PRODUCER.to_owned(),
        ..Dump::default()
    };
    let mut ocean: Vec<ade::BenchExport> = Vec::new();
    // (dump cell index, design cell index) of every written cell.
    let mut written = Vec::new();
    for ci in 0..d.cells.len() {
        if !matches!(plan.how[ci], How::Design | How::Core) {
            continue;
        }
        let c = cell_out(d, ci, &plan, m, &mut notes);
        if d.cells[ci].kind() == FileKind::Shbench {
            let mut b = ade::export(
                d,
                ci,
                &plan.lib[ci],
                |p| inst_info(d, &plan, m, ci, p),
                &mut notes,
            );
            if !b.records.is_empty() {
                dump.maestro.push(std::mem::take(&mut b.records));
                ocean.push(b);
            }
        }
        written.push((dump.cells.len(), ci));
        dump.cells.push(c);
    }
    // §7.3: what the Virtuoso side cannot carry rides along as stash properties.
    let first = records(&dump);
    let image = read::image(&first, &plan, d, m);
    for (k, ci) in written {
        let c = &d.cells[ci];
        let img = image.get(&(dump.cells[k].lib.clone(), dump.cells[k].name.clone()));
        if img.is_none() && plan.how[ci] == How::Core {
            continue; // the bundled core: never imported, so nothing to restore
        }
        let Some(text) = merge::stash(&c.doc, img) else {
            continue;
        };
        let view = if c.kind() == FileKind::Shprim {
            "symbol"
        } else {
            "schematic"
        };
        let cv = dump.cells[k]
            .cvs
            .iter_mut()
            .find(|v| v.view == view)
            .expect("every cell kind has its stash view");
        for (n, chunk) in sfyx::chunks(&text, STASH_CHUNK).into_iter().enumerate() {
            cv.props
                .push(Prop::string(0, &format!("schemifyRecords{}", n + 1), chunk));
        }
    }
    let recs = records(&dump);
    assert!(
        recs.first().is_some_and(|r| r.tag == "sfyx")
            && recs.last().is_some_and(|r| r.tag == "end")
    );
    // One script per bench cell; a second cell of the same name (another library) gets `_2`, ….
    let mut scripts: Vec<(String, String)> = Vec::new();
    for ade::BenchExport {
        cell: test,
        ocean: text,
        defns,
        ..
    } in ocean
    {
        let mut name = test.clone();
        let mut k = 1;
        while scripts.iter().any(|s| s.0 == format!("{name}.ocn")) {
            k += 1;
            name = format!("{test}_{k}");
        }
        for (f, t) in defns {
            scripts.push((format!("{name}/{f}"), t));
        }
        let text = if name == test {
            text
        } else {
            text.replace(&format!("\"{test}/"), &format!("\"{name}/"))
        };
        scripts.push((format!("{name}.ocn"), text));
    }
    scripts.sort();
    // §12: one `off-grid` row per cell (the count and the first point), not one per point:
    // a symbol drawn on a half grid has dozens and they all mean the same thing.
    let mut grid: Vec<(String, usize, String)> = Vec::new();
    notes.rows.retain(|r| {
        if r.class != "off-grid" {
            return true;
        }
        match grid.iter_mut().find(|g| g.0 == r.cell) {
            Some(g) => g.1 += 1,
            None => grid.push((r.cell.clone(), 1, r.detail.clone())),
        }
        false
    });
    for (cell, n, first) in grid {
        notes.add(
            "off-grid",
            &cell,
            format!("{n} point(s) off the 1/160\" grid (Virtuoso snaps them), first: {first}"),
        );
    }
    Ok(Export {
        sfyx: sfyx::write(&recs),
        notes,
        ocean: scripts,
    })
}

/// What the output translator needs about instance path `path` (Schemify instance names
/// joined by `/`, a subcircuit's SPICE name `X<name>` accepted for `<name>`) of bench `ci`
/// (§9.1): its Virtuoso path and the terminal `i()` reads (the exported master's first pin:
/// analogLib's, a map row's, else the pin's own name). `None` when a segment names no
/// instance.
fn inst_info(d: &Design, plan: &Plan, m: &Mapping, ci: usize, path: &str) -> Option<ade::InstInfo> {
    let mut cur = ci;
    let mut vpath: Vec<String> = Vec::new();
    let mut last: Option<usize> = None;
    for seg in path.split('/') {
        let c = &d.cells[cur];
        let by_name = |n: &str| {
            c.records(RecordKind::Inst)
                .find(|&r| c.attr(r, "name") == Some(n))
        };
        let r = by_name(seg).or_else(|| {
            seg.strip_prefix(['X', 'x']).and_then(by_name).filter(|&r| {
                d.resolve(cur, c.field(r, 1))
                    .is_some_and(|mi| d.cells[mi].kind() == FileKind::Shcomp)
            })
        })?;
        let name = c.attr(r, "name").expect("found by name");
        vpath.push(profile::inst_to_virtuoso(name).unwrap_or_else(|| name.to_owned()));
        let mi = d.resolve(cur, c.field(r, 1))?;
        last = Some(mi);
        cur = mi;
    }
    let mi = last?;
    let first_pin = sym_pins(&d.cells[mi])
        .name
        .first()
        .cloned()
        .unwrap_or_default();
    let terminal = match plan.how[mi] {
        How::AnalogLib(row) => devices::DEVICES[row as usize]
            .alib_pins
            .first()
            .map_or(first_pin.clone(), |p| (*p).to_owned()),
        How::Map(row) => m.maps[row as usize]
            .pins
            .iter()
            .find(|p| p.0 == first_pin)
            .map_or_else(|| vname(&first_pin), |p| p.1.clone()),
        _ => vname(&first_pin),
    };
    Some(ade::InstInfo {
        path: vpath.join("/"),
        terminal,
    })
}

// ---------------------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------------------

/// Builds the dump tables of Design cell `ci`.
fn cell_out(d: &Design, ci: usize, plan: &Plan, m: &Mapping, notes: &mut Notes) -> DCell {
    let c = &d.cells[ci];
    let mut out = DCell {
        lib: plan.lib[ci].clone(),
        name: plan.cell[ci].clone(),
        design: true,
        ..DCell::default()
    };
    let core = plan.how[ci] == How::Core;
    let mut ids = Ids::default();
    match c.kind() {
        FileKind::Shprim | FileKind::Shcomp => {
            // A cell from Virtuoso whose interface nobody changed gets its CDF back as it
            // was (the reverse stash, §7.5); otherwise the CDF is derived from the cell.
            let as_imported = if core { None } else { cdf_as_imported(c) };
            let cdf = as_imported
                .clone()
                .flatten()
                .unwrap_or_else(|| cdf_out(c, core, notes));
            // A component without parameters whose pins are in Virtuoso's default order
            // (sorted names, §6.3) needs no CDF: none is written, so a cell that had none
            // in Virtuoso gets none back.
            let names: Vec<String> = {
                let p = sym_pins(c);
                p.name
                    .into_iter()
                    .zip(p.alias)
                    .filter(|(_, a)| a.is_none())
                    .map(|(n, _)| vname(&n))
                    .collect()
            };
            let sorted = names.windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes());
            let bare = c.kind() == FileKind::Shcomp && cdf.params.is_empty() && sorted;
            out.cdf = match as_imported {
                Some(stored) => stored,
                None => (!bare).then_some(cdf),
            };
            out.cvs
                .push(keep(c, super::Part::Symbol, symbol_out(c, &mut ids, notes)));
            if c.kind() == FileKind::Shcomp {
                let sch = schematic_out(d, ci, plan, m, &mut ids, notes);
                out.cvs.push(keep(c, super::Part::Schematic, sch));
            }
            out.textviews = textviews_out(c, notes);
        }
        FileKind::Shbench => {
            let sch = schematic_out(d, ci, plan, m, &mut ids, notes);
            out.cvs.push(keep(c, super::Part::Schematic, sch));
        }
        k => unreachable!("design cells are cell files, got {k:?}"),
    }
    assert!(!out.cvs.is_empty());
    out
}

/// Provisional refs of one cell's cellviews.
#[derive(Default)]
struct Ids {
    next: u32,
}

impl Ids {
    fn take(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
}

/// Converts a Schemify point to export units, reporting it when off Virtuoso's grid.
fn conv(p: [i32; 2], what: &str, cell: &Cell, notes: &mut Notes) -> [i32; 2] {
    if !profile::on_dbu_grid(p[0], DBU) || !profile::on_dbu_grid(p[1], DBU) {
        notes.add(
            "off-grid",
            &cell.path,
            format!("{what} at ({}, {})", coord_text(p[0]), coord_text(p[1])),
        );
    }
    profile::point_to_virtuoso(p).unwrap_or([p[0], i32::MAX])
}

/// Coordinates of a Schemify record's fields `from..` as milli-units.
fn coords(c: &Cell, r: u32, from: usize) -> Vec<i32> {
    c.doc
        .fields(r)
        .skip(from)
        .map(|t| parse_coord(t).unwrap_or(0))
        .collect()
}

/// The Virtuoso spelling of a pin or terminal name.
pub fn vname(pin: &str) -> String {
    net_to_virtuoso(pin, false).0
}

/// The CDF a cell from Virtuoso had, when its interface (pins, params, impls) is unchanged
/// since the import (`vcdf` fingerprint, §7.5): `Some(None)` for a cell that had no CDF,
/// `Some(Some(cdf))` for the stored records (`vcdfrec`), `None` when the cell is not from
/// Virtuoso or was edited (export then derives the CDF, §6.3).
fn cdf_as_imported(c: &Cell) -> Option<Option<Cdf>> {
    let fp = c.header(super::fingerprint_attr(super::Part::Interface))?;
    if fp != super::fingerprint(&c.doc, super::Part::Interface) {
        return None;
    }
    let recs: Vec<&str> = (0..c.doc.attrs(0).len() as u32)
        .map(|k| c.doc.attrs(0).start + k)
        .filter(|&a| c.doc.attr_key(a) == "vcdfrec")
        .filter_map(|a| c.doc.attr_value(a, 0))
        .collect();
    if recs.is_empty() {
        return Some(None);
    }
    let text = format!(
        "(\"sfyx\" 1 \"vcdf\")\n(\"lib\" \"l\")\n(\"cell\" \"c\" \"design\")\n{}\n(\"end\")\n",
        recs.join("\n")
    );
    let cdf = sfyx::read(&text)
        .ok()
        .and_then(|r| dump(&r).ok())
        .and_then(|d| d.cells.into_iter().next())
        .and_then(|c| c.cdf)?;
    Some(Some(cdf))
}

/// Marks `cv` with `schemifyKeep` when `part` of `c` is unchanged since it was imported
/// (its fingerprint equals the header attribute import wrote, §7.5).
fn keep(c: &Cell, part: super::Part, mut cv: Cv) -> Cv {
    if c.header(super::fingerprint_attr(part)) == Some(super::fingerprint(&c.doc, part).as_str()) {
        cv.props.push(Prop::string(0, "schemifyKeep", "1"));
    }
    cv
}
