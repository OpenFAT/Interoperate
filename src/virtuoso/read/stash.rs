//! A cellview's own properties → the stash text (spec `interop.md` §7.3) and the report
//! of everything else (§12); stashed wire ids put back where Virtuoso swapped them.

use super::*;

/// Per-cellview accounting for count conservation (§12).
#[derive(Default)]
pub(super) struct Tally {
    pub(super) out: usize,
    pub(super) reported: usize,
}

/// Cellview properties: the stash (`schemifyRecords<k>`, joined in numeric order) and
/// everything else reported.
pub(super) fn cv_props(
    sv: &Cv,
    cell: &str,
    notes: &mut Notes,
    tally: &mut Tally,
) -> Option<String> {
    for p in sv.props.iter().filter(|p| p.r == 0) {
        if stash_chunk(&p.name).is_some() {
            tally.out += 1;
        } else {
            notes.dropped(
                "display-only",
                cell,
                format!("cellview property {}", p.name),
            );
            tally.reported += 1;
        }
    }
    stash_of(sv)
}

/// The chunk number of a `schemifyRecords<k>` property name.
fn stash_chunk(name: &str) -> Option<u32> {
    name.strip_prefix("schemifyRecords")?.parse().ok()
}

/// The stash text of a cellview: its `schemifyRecords<k>` properties joined in numeric order.
pub(super) fn stash_of(sv: &Cv) -> Option<String> {
    let mut chunks: Vec<(u32, &str)> = sv
        .props
        .iter()
        .filter(|p| p.r == 0)
        .filter_map(|p| Some((stash_chunk(&p.name)?, p.value.as_str())))
        .collect();
    chunks.sort();
    (!chunks.is_empty()).then(|| chunks.iter().map(|c| c.1).collect())
}

/// Reports conns and unknown records; counts properties of objects (consumed by
/// their owners' builders) as out.
pub(super) fn report_rest(sv: &Cv, cell: &str, notes: &mut Notes, tally: &mut Tally) {
    for c in &sv.conns {
        notes.dropped(
            "display-only",
            cell,
            format!("conn {} {} (loader-only)", c.term, c.net),
        );
    }
    for r in &sv.unknown {
        notes.dropped(
            "unknown-record",
            cell,
            format!("`{}` on line {}", r.tag, r.line),
        );
    }
    // What the dumper left out (S6): one row per kind, with its count.
    for (kind, n) in &sv.skipped {
        notes.dropped(
            "display-only",
            cell,
            format!("{kind} ×{n} ({} view): not in the dump as drawn", sv.view),
        );
    }
    tally.reported += sv.conns.len() + sv.unknown.len() + sv.skipped.len();
    // Virtuoso's connectivity is the oracle `check_connectivity` reads.
    tally.out += sv.props.iter().filter(|p| p.r != 0).count() + sv.tnets.len();
}

/// A segment with its ends in sorted order, the key two drawings of one wire share.
pub(super) fn seg_key(a: [i32; 2], b: [i32; 2]) -> [[i32; 2]; 2] {
    if b < a { [b, a] } else { [a, b] }
}

/// The wires of a stash document: segment → id.
pub(super) fn stash_wires(d: &Document<'_>) -> BTreeMap<[[i32; 2]; 2], String> {
    let mut out = BTreeMap::new();
    for r in 1..d.record_count() {
        if d.record_kind(r) != RecordKind::Wire {
            continue;
        }
        let c = |k| {
            d.field(r, k)
                .and_then(|f| schemify_format::parse_coord(f).ok())
        };
        if let (Some(id), Some(x0), Some(y0), Some(x1), Some(y1)) =
            (d.field(r, 0), c(1), c(2), c(3), c(4))
        {
            out.insert(seg_key([x0, y0], [x1, y1]), id.to_owned());
        }
    }
    out
}

/// Virtuoso merges collinear wires that touch and splits them again where a wire T-joins,
/// handing the pieces' schemifyIds out in its own order (live ICADVM20.1 run: two pieces came
/// back with their ids exchanged). Where wires drawn exactly as stashed carry each other's
/// stashed ids, the ids go back to their stashed segments. `drawn`: (ref, segment, id).
/// Returns ref → id for the wires to re-id.
// ponytail: all-or-nothing, applied only when the re-ids are a pure permutation of the ids
// they replace; a partial match (a wire also moved in Virtuoso) keeps Virtuoso's ids.
pub(super) fn unswap_wire_ids(
    drawn: &[(u32, [[i32; 2]; 2], &str)],
    stashed: &BTreeMap<[[i32; 2]; 2], String>,
) -> BTreeMap<u32, String> {
    let moved: Vec<(u32, &str, &str)> = drawn
        .iter()
        .filter_map(|(r, s, id)| {
            let want = stashed.get(s)?;
            (want != id).then_some((*r, *id, want.as_str()))
        })
        .collect();
    let from: BTreeSet<&str> = moved.iter().map(|m| m.1).collect();
    let to: BTreeSet<&str> = moved.iter().map(|m| m.2).collect();
    if from != to || from.len() != moved.len() {
        return BTreeMap::new();
    }
    let out: BTreeMap<u32, String> = moved.iter().map(|m| (m.0, m.2.to_owned())).collect();
    debug_assert!(out.len() == from.len());
    out
}
