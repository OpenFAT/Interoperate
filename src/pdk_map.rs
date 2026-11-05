//! `schemify-interop pdk-map` (feature `cli`): `interop.map` rows (spec `interop.md` §8.2)
//! that send a kit's scanned cells (`pdk-scan`'s `.shprim`) to the kit's Virtuoso cells, from
//! the names `SfyCdfNames` (`skill/sfy_pdkmap.il`) dumps of its Virtuoso libraries. Kit cell
//! names are the kit's own, so the rows come from the kit, never from a person or a repo.
//!
//! Matching: scanned cell `X` (its pins in netlist order, its param names) maps to a
//! Virtuoso cell whose CDF `model` default is `X` (ASCII case aside: a spice scan lowers its
//! names) and whose spectre termOrder has as many terminals as `X` has pins. Pins pair by
//! position (termOrder is the netlist's port order); each param name both have gets a
//! `param` row (case aside again; the scan's name first). Ties go, in order, to the cell
//! whose symbol's terminals are its termOrder's (a 4-terminal symbol over a 3-terminal one
//! with the body inherited), the library given first, the cell name first. A cell imported
//! from xschem also matches by the device its `format spice` calls (`sky130_fd_pr__{model}`,
//! the model default substituted), and its `W`/`L` rows get `scale 1e-6` when D14 reads
//! their defaults as micrometres (sky130's `L=0.15`; Virtuoso CDFs are in metres).
//!
//! Dump, between the `sfyx` header and `end`, one record per Virtuoso cell:
//!
//! ```text
//! ("cdfnames" lib cell model (termOrder ...) (param ...) (symbol terminal ...))
//! ```
//!
//! [`remote_script`] makes that dump on the kit's host: `ssh <host> sh -s` reads it, starts
//! Virtuoso in batch with the Cadence environment of [`ENV`] and prints only the dump.

use std::collections::BTreeSet;

use schemify_format::RecordKind;

use crate::map::Mapping;
use crate::notes::Notes;
use crate::sfyx;

/// The POSIX `sh` body of [`remote_script`], below its settings.
const REMOTE_BODY: &str = include_str!("pdk_cdf.sh");

/// The settings [`remote_script`] takes from the environment where pdk-map runs:
/// `(variable, the name the script holds it under)`. The licence variables are held aside
/// so that the setup script cannot replace them.
pub const ENV: [(&str, &str); 6] = [
    ("VIRTUOSO", "VIRTUOSO"),
    ("SFY_CADENCE_ENV", "SFY_CADENCE_ENV"),
    ("SFY_CDS_DIR", "SFY_CDS_DIR"),
    ("SFY_VIRTUOSO_TIMEOUT", "SFY_VIRTUOSO_TIMEOUT"),
    ("CDS_LIC_FILE", "SFY_CDS_LIC_FILE"),
    ("CDS_LIC_ONLY", "SFY_CDS_LIC_ONLY"),
];

/// The script `ssh <host> sh -s` runs to dump `libs` (in that order) with `SfyCdfNames`:
/// each [`ENV`] variable `env` gives (`VIRTUOSO` defaults to `virtuoso`), `schemify.il` and
/// the batch session's file, then `src/pdk_cdf.sh`. Its stdout is the dump only.
pub fn remote_script(libs: &[String], env: impl Fn(&str) -> Option<String>) -> String {
    const END: &str = "SFY_FILE_END";
    let sh = |v: &str| format!("'{}'", v.replace('\'', "'\\''"));
    let mut out = String::from("# schemify-interop pdk-map --remote-script\n");
    for (var, held) in ENV {
        let v = env(var)
            .filter(|v| !v.is_empty())
            .or_else(|| (var == "VIRTUOSO").then(|| "virtuoso".into()));
        if let Some(v) = v {
            out.push_str(&format!("{held}={}\nexport {held}\n", sh(&v)));
        }
    }
    let mut run = String::from(
        "(load (strcat (getShellEnvVar \"SFY_PDKMAP_DIR\") \"/schemify.il\"))\n\
         (errset (SfyCdfNames (strcat (getShellEnvVar \"SFY_PDKMAP_DIR\") \"/cdf.sfyx\") (list",
    );
    for l in libs {
        run.push(' ');
        sfyx::write_str(l, &mut run);
    }
    run.push_str(")) t)\n(exit)\n");
    let il = crate::skill::schemify_il();
    assert!(
        il.lines().all(|l| l != END),
        "schemify.il holds the heredoc end"
    );
    out.push_str("sfy_files() {\n");
    for (f, text) in [("schemify.il", il.as_str()), ("run.il", run.as_str())] {
        out.push_str(&format!("cat >\"$sfy_t/{f}\" <<'{END}'\n{text}{END}\n"));
    }
    out.push_str("}\n");
    out.push_str(REMOTE_BODY);
    out
}

/// A scanned cell: what its netlist line needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scanned {
    /// The cell name (the netlist target).
    pub name: String,
    /// Pins, in netlist order.
    pub pins: Vec<String>,
    /// Parameter names.
    pub params: Vec<String>,
    /// `W`/`L` params (any case) whose default D14 reads as micrometres (sky130's `L=0.15`
    /// under `.option scale=1e-6`): their rows get `scale 1e-6` (Virtuoso is in metres).
    pub micro: Vec<String>,
    /// The device the netlist calls when it is not `name`: an xschem import's `format spice`
    /// word holding `{model}`, the model default substituted (`sky130_fd_pr__nfet_01v8`).
    pub netlist_name: Option<String>,
}

/// One Virtuoso cell of the dump.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vcell {
    /// Library.
    pub lib: String,
    /// Cell.
    pub cell: String,
    /// The CDF `model` parameter's default; empty without one.
    pub model: String,
    /// The spectre simInfo termOrder.
    pub order: Vec<String>,
    /// CDF parameter names.
    pub params: Vec<String>,
    /// The symbol's terminal names.
    pub terms: Vec<String>,
}

impl Vcell {
    fn name(&self) -> String {
        format!("{}/{}", self.lib, self.cell)
    }

    /// The symbol has exactly the termOrder's terminals.
    fn symbol_fits(&self) -> bool {
        let t: BTreeSet<&String> = self.terms.iter().collect();
        t == self.order.iter().collect()
    }
}

/// Reads scanned cell `name` from its `.shprim` bytes: its `pin` and `param` records.
///
/// # Errors
///
/// The file does not parse.
pub fn scanned(name: &str, bytes: &[u8]) -> Result<Scanned, String> {
    let doc = schemify_format::parse(bytes).map_err(|e| {
        e.diagnostics.first().map_or("does not parse".into(), |d| {
            format!("line {}: {}", d.line, d.detail)
        })
    })?;
    let mut s = Scanned {
        name: name.to_owned(),
        ..Scanned::default()
    };
    let mut model = None;
    let mut format = None;
    for r in 0..doc.record_count() {
        let field = |i| doc.field(r, i).unwrap_or_default().to_owned();
        match doc.record_kind(r) {
            RecordKind::Pin => s.pins.push(field(0)),
            RecordKind::Param => {
                let (n, d) = (field(0), field(1));
                if matches!(n.as_str(), "w" | "W" | "l" | "L") && crate::limits::is_micro(&d) {
                    s.micro.push(n.clone());
                }
                if n == "model" {
                    model = Some(d);
                }
                s.params.push(n);
            }
            RecordKind::Impl if format.is_none() => {
                format = doc
                    .attrs(r)
                    .find(|&a| doc.attr_key(a) == "format" && doc.attr_value(a, 0) == Some("spice"))
                    .and_then(|a| doc.attr_value(a, 1))
                    .map(str::to_owned);
            }
            _ => {}
        }
    }
    s.netlist_name = format.zip(model).and_then(|(f, m)| {
        let w = f.split_whitespace().find(|w| w.contains("{model}"))?;
        let n = w.replace("{model}", &m);
        (!n.contains(['{', '}']) && n != s.name).then_some(n)
    });
    Ok(s)
}

/// Reads a `SfyCdfNames` dump.
///
/// # Errors
///
/// A malformed or truncated file, or a record that is not `cdfnames` with string fields.
pub fn read_dump(text: &str) -> Result<Vec<Vcell>, String> {
    let at = |e: sfyx::Error| format!("line {}: {}", e.line, e.msg);
    let recs = sfyx::read(text).map_err(at)?;
    sfyx::require_complete(&recs).map_err(at)?;
    let mut out = Vec::new();
    for r in &recs[1..recs.len() - 1] {
        if r.tag != "cdfnames" {
            return Err(format!(
                "line {}: unknown record `{}` (cdfnames)",
                r.line, r.tag
            ));
        }
        r.arity(6).map_err(at)?;
        let names = |i: usize| -> Result<Vec<String>, String> {
            r.list(i)
                .map_err(at)?
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or_else(|| format!("line {}: field {} holds a non-string", r.line, i + 1))
        };
        out.push(Vcell {
            lib: r.str(0).map_err(at)?.to_owned(),
            cell: r.str(1).map_err(at)?.to_owned(),
            model: r.str(2).map_err(at)?.to_owned(),
            order: names(3)?,
            params: names(4)?,
            terms: names(5)?,
        });
    }
    Ok(out)
}

/// The `interop.map` text mapping Schemify library `lib`'s scanned cells `scan` onto the
/// dump's cells `kit`, and a report row per scanned cell: `mapped` (the choice and why) or
/// `needs_review:unmapped` (no candidate, or none with the scan's pin count). Rows go in
/// cell-name order; libraries rank in their order in `kit`.
///
/// # Errors
///
/// The text fails `Mapping::parse` (a name it cannot carry), with its errors.
pub fn map(lib: &str, scan: &[Scanned], kit: &[Vcell]) -> Result<(String, Notes), Vec<String>> {
    let mut libs: Vec<&str> = Vec::new();
    for k in kit {
        if !libs.contains(&k.lib.as_str()) {
            libs.push(&k.lib);
        }
    }
    let rank = |k: &Vcell| libs.iter().position(|l| *l == k.lib);
    let mut scan: Vec<&Scanned> = scan.iter().collect();
    scan.sort_by(|a, b| a.name.cmp(&b.name));
    let (mut text, mut notes) = (String::new(), Notes::default());
    for s in scan {
        let cellref = format!("{lib}:{}", s.name);
        let cands: Vec<&Vcell> = kit
            .iter()
            .filter(|k| {
                k.model.eq_ignore_ascii_case(&s.name)
                    || s.netlist_name
                        .as_ref()
                        .is_some_and(|n| k.model.eq_ignore_ascii_case(n))
            })
            .collect();
        let mut fit: Vec<&Vcell> = cands
            .iter()
            .copied()
            .filter(|k| k.order.len() == s.pins.len())
            .collect();
        fit.sort_by_key(|k| (!k.symbol_fits(), rank(k), &k.cell));
        let Some(&best) = fit.first() else {
            let why = if cands.is_empty() {
                let same: Vec<String> = kit
                    .iter()
                    .filter(|k| k.cell == s.name)
                    .map(|k| match k.model.as_str() {
                        "" => format!("{} has no CDF model", k.name()),
                        m => format!("{}'s model is {m}", k.name()),
                    })
                    .collect();
                let hint = if same.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", same.join(", "))
                };
                format!(
                    "no candidate: no Virtuoso cell's CDF model is {}{hint}",
                    s.name
                )
            } else {
                let counts: Vec<String> = cands
                    .iter()
                    .map(|k| format!("{} has {}", k.name(), k.order.len()))
                    .collect();
                format!(
                    "pin-count mismatch: {} pins; termOrder: {}",
                    s.pins.len(),
                    counts.join(", ")
                )
            };
            notes.review("unmapped", &cellref, why);
            continue;
        };
        let mut why = match fit.get(1) {
            None => "the only candidate".to_owned(),
            Some(&next) => {
                let rule = if best.symbol_fits() != next.symbol_fits() {
                    "its symbol's terminals are its termOrder"
                } else if rank(best) != rank(next) {
                    "library order"
                } else {
                    "name order"
                };
                let others: Vec<String> = fit[1..].iter().map(|k| k.name()).collect();
                format!("over {} by {rule}", others.join(", "))
            }
        };
        let skipped: Vec<String> = cands
            .iter()
            .filter(|k| k.order.len() != s.pins.len())
            .map(|k| k.name())
            .collect();
        if !skipped.is_empty() {
            why.push_str(&format!(
                "; pin count not {}: {}",
                s.pins.len(),
                skipped.join(", ")
            ));
        }
        if !best.symbol_fits() {
            why.push_str(&format!(
                "; its symbol's terminals ({}) are not its termOrder ({})",
                best.terms.join(" "),
                best.order.join(" ")
            ));
        }
        text.push_str(&format!(
            "map {} virtuoso {} {}\n",
            word(&cellref),
            word(&best.lib),
            word(&best.cell)
        ));
        for (p, t) in s.pins.iter().zip(&best.order) {
            text.push_str(&format!("  pin {} {}\n", word(p), word(t)));
        }
        let mut unshared = Vec::new();
        for p in &s.params {
            let v = best
                .params
                .iter()
                .find(|v| *v == p)
                .or_else(|| best.params.iter().find(|v| v.eq_ignore_ascii_case(p)));
            // Matched through the netlist name, the two `model` values differ by the
            // cell's prefix (`nfet_01v8` against `sky130_fd_pr__nfet_01v8`): never crossed.
            let by_netlist_name = !best.model.eq_ignore_ascii_case(&s.name);
            match v {
                Some(_) if p == "model" && by_netlist_name => {
                    text.push_str("  drop model\n");
                    why.push_str(&format!(
                        "; by its netlist name {}: model dropped",
                        best.model
                    ));
                }
                Some(v) if s.micro.contains(p) => {
                    text.push_str(&format!("  param {} {} scale 1e-6\n", word(p), word(v)));
                }
                Some(v) => text.push_str(&format!("  param {} {}\n", word(p), word(v))),
                None => unshared.push(p.as_str()),
            }
        }
        if !s.micro.is_empty() {
            why.push_str(&format!(
                "; {} in micrometres (D14: a plain default of 1e-3 or more): scale 1e-6",
                s.micro.join(" ")
            ));
        }
        if !unshared.is_empty() {
            why.push_str(&format!("; not in its CDF: {}", unshared.join(" ")));
        }
        notes.add("mapped", &cellref, format!("{}: {why}", best.name()));
    }
    let rows = Mapping::parse(&text)?.maps.len();
    assert_eq!(rows, notes.counts().get("mapped").copied().unwrap_or(0));
    Ok((text, notes))
}

/// `v` as one line-record value, quoted as the format's writer quotes (format.md §2): empty,
/// leading `#` or `|`, or a space, control, `"` or `\\` byte. A name holding a control byte
/// fails [`map`]'s own check.
fn word(v: &str) -> String {
    let bare = |c: u8| c > b' ' && c != b'"' && c != b'\\' && c != 0x7f;
    if !v.is_empty() && !v.starts_with(['#', '|']) && v.bytes().all(bare) {
        v.to_owned()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(name: &str, pins: &[&str], params: &[&str]) -> Scanned {
        Scanned {
            name: name.into(),
            pins: pins.iter().map(|p| p.to_string()).collect(),
            params: params.iter().map(|p| p.to_string()).collect(),
            ..Scanned::default()
        }
    }

    fn vc(lib: &str, cell: &str, model: &str, order: &[&str], terms: &[&str]) -> Vcell {
        let v = |x: &[&str]| x.iter().map(|p| p.to_string()).collect();
        Vcell {
            lib: lib.into(),
            cell: cell.into(),
            model: model.into(),
            order: v(order),
            params: v(&["model", "W", "l"]),
            terms: v(terms),
        }
    }

    const DGSB: [&str; 4] = ["D", "G", "S", "B"];

    #[test]
    fn the_symbol_rule_beats_library_and_name_order() {
        let kit = [
            vc("a", "nfoo", "nfoo", &DGSB, &["D", "G", "S"]),
            vc("b", "nfoo_x", "NFOO", &DGSB, &["B", "D", "G", "S"]),
        ];
        let (text, notes) = map(
            "k",
            &[sc("nfoo", &["d", "g", "s", "b"], &["w", "l", "nf"])],
            &kit,
        )
        .unwrap();
        assert_eq!(
            text,
            "map k:nfoo virtuoso b nfoo_x\n  pin d D\n  pin g G\n  pin s S\n  pin b B\n  param w W\n  param l l\n"
        );
        assert_eq!(
            notes.text(),
            "mapped\tk:nfoo\tb/nfoo_x: over a/nfoo by its symbol's terminals are its termOrder; not in its CDF: nf\n"
        );
    }

    #[test]
    fn library_order_then_name_order() {
        let two = ["P", "N"];
        let kit = [
            vc("b", "r1", "r", &two, &two),
            vc("a", "r2", "r", &two, &two),
            vc("a", "r0", "r", &two, &two),
        ];
        let (text, notes) = map("k", &[sc("r", &["p", "n"], &[])], &kit).unwrap();
        assert!(text.starts_with("map k:r virtuoso b r1\n"), "{text}");
        assert!(notes.text().contains("over a/r0, a/r2 by library order"));
        let (text, notes) = map("k", &[sc("r", &["p", "n"], &[])], &kit[1..]).unwrap();
        assert!(text.starts_with("map k:r virtuoso a r0\n"), "{text}");
        assert!(notes.text().contains("over a/r2 by name order"));
    }

    #[test]
    fn misses_say_why() {
        let kit = [
            vc("a", "pbar", "pbar", &["D", "G", "S"], &["D", "G", "S"]),
            vc("a", "nocdf", "", &[], &["A", "B"]),
            vc("a", "nfoo", "nfoo", &DGSB, &["D", "G", "S"]),
        ];
        let scan = [
            sc("pbar", &["d", "g", "s", "b"], &[]),
            sc("nocdf", &["a", "b"], &[]),
            sc("qux", &["a"], &[]),
            sc("nfoo", &["d", "g", "s", "b"], &[]),
        ];
        let (text, notes) = map("k", &scan, &kit).unwrap();
        assert_eq!(text.matches("map ").count(), 1);
        assert_eq!(
            notes.text(),
            "mapped\tk:nfoo\ta/nfoo: the only candidate; its symbol's terminals (D G S) are not its termOrder (D G S B)\n\
             needs_review:unmapped\tk:nocdf\tno candidate: no Virtuoso cell's CDF model is nocdf (a/nocdf has no CDF model)\n\
             needs_review:unmapped\tk:pbar\tpin-count mismatch: 4 pins; termOrder: a/pbar has 3\n\
             needs_review:unmapped\tk:qux\tno candidate: no Virtuoso cell's CDF model is qux\n"
        );
    }

    /// inv_xschem row 3: an xschem-imported sky130 device (µm W/L, `sky130_fd_pr__{model}`)
    /// maps onto a kit cell whose model is the SPICE name, with W/L scaled to metres.
    #[test]
    fn an_xschem_sky130_cell_matches_by_netlist_name_and_scales_micrometres() {
        let nfet = "schemify 1 shprim\n  provenance \"xschem sky130_fd_pr/nfet_01v8.sym\"\n\npin D inout 20 -30\npin G in -20 0\npin S inout 20 30\npin B in 20 0\n\nparam W 1\nparam L 0.15\nparam nf 1\nparam model nfet_01v8\n  kind string\n\nimpl spice spice\n  format spice \"X{name} {pins} sky130_fd_pr__{model} L={L} W={W} nf={nf}\"\n";
        let s = scanned("nfet_01v8", nfet.as_bytes()).unwrap();
        assert_eq!(s.micro, ["W", "L"]);
        assert_eq!(s.netlist_name.as_deref(), Some("sky130_fd_pr__nfet_01v8"));
        let mut k = vc("s8", "nfet_01v8", "sky130_fd_pr__nfet_01v8", &DGSB, &DGSB);
        k.params = ["w", "l", "nf", "m"].map(String::from).to_vec();
        let (text, notes) = map("sky130_fd_pr", &[s], &[k]).unwrap();
        assert_eq!(
            text,
            "map sky130_fd_pr:nfet_01v8 virtuoso s8 nfet_01v8\n  pin D D\n  pin G G\n  pin S S\n  pin B B\n  param W w scale 1e-6\n  param L l scale 1e-6\n  param nf nf\n"
        );
        assert!(
            notes.text().contains("W L in micrometres"),
            "{}",
            notes.text()
        );
        // A metre default (gf180 `0.28u`) is not scaled.
        let gf = nfet.replace("param L 0.15", "param L 0.28u");
        assert_eq!(scanned("n", gf.as_bytes()).unwrap().micro, ["W"]);
    }

    #[test]
    fn odd_names_are_quoted_and_still_parse() {
        let kit = [vc("a b", "c\"d", "x", &["vdd!", "B<0>"], &[])];
        let (text, _) = map("k", &[sc("x", &["p", "#n"], &[])], &kit).unwrap();
        assert!(
            text.starts_with(
                "map k:x virtuoso \"a b\" \"c\\\"d\"\n  pin p vdd!\n  pin \"#n\" B<0>\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn the_dump_reads_back() {
        let t = "(\"sfyx\" 1 \"schemify.il 1\")\n(\"cdfnames\" \"a\" \"n\" \"n\" (\"D\") (\"model\") (\"D\"))\n(\"end\")\n";
        let k = read_dump(t).unwrap();
        assert_eq!(k[0].order, ["D"]);
        assert!(read_dump(&t.replace("(\"end\")\n", "")).is_err());
        assert!(read_dump(&t.replace("(\"D\") (\"model\")", "(1) (\"model\")")).is_err());
        assert!(read_dump(&t.replace("cdfnames", "cell")).is_err());
    }
}
