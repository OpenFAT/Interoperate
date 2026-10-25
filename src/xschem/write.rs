//! Schemify → xschem `.sch`/`.sym` export (spec `interop.md` §11).
//!
//! Spine ([`export`]), one line per transform:
//! 1. `closure`: the cells reachable from the tops through instance references.
//! 2. `target_of` per reachable cell: written here, an xschem `devices/` symbol (a `core`
//!    cell, by inverting [`DEVICE_MAP`]), a global label symbol (`core:gnd`/`core:vdd`), or
//!    the original xschem symbol (a cell whose provenance is xschem).
//! 3. per written cell: `write_symbol` (`.shprim`, `.shcomp`) and `write_schematic`
//!    (`.shcomp`, `.shbench`).
//! 4. `xschemrc`: the fragment that puts the output directory on `XSCHEM_LIBRARY_PATH`.
//!
//! The translations at the end of the file are tables of spellings: values (§3.4, xschem
//! paragraph), templates (§3.5, import.md §4 inverted), orientations (import.md §3.1
//! inverted). Every construct without an xschem spelling becomes a [`Notes`] row and is
//! left at the symbol default; nothing is dropped silently.
//!
//! Data: in = a loaded [`Design`] (≤ 2^16 cells, each its own `Document`); out = one or
//! two text files per written cell. Every transform is one sequential pass over one cell's
//! records; everything lives for one call. Per-cell work is independent, but at a few
//! kilobytes per cell it stays sequential.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;

use schemify_format::{FileKind, Orient, RecordKind, format_coord, parse_coord};

use crate::design::{Cell, Design, SymPins, sym_pins};
use crate::notes::Notes;
use crate::xschem::devices::{DEVICE_MAP, DeviceMap};
use crate::xschem::file::ORIENT_TABLE;

mod schematic;
mod symbol;
mod values;
use schematic::*;
use symbol::*;
use values::*;

/// The files of one export and what did not carry over.
#[derive(Clone, Debug, Default)]
pub struct XschemExport {
    /// `(path, bytes)` sorted by path: `<lib>/<cell>.sym`, `<lib>/<cell>.sch` and
    /// `xschemrc`, paths relative to the output directory.
    pub files: Vec<(String, Vec<u8>)>,
    /// One row per construct that has no xschem spelling (spec §12 classes).
    pub notes: Notes,
}

/// Output directory of cells in the project root (they have no library name); `_2`, `_3`, …
/// is appended while a library has the same name.
pub const ROOT_DIR: &str = "root";

/// Output directory of `core` cells that no xschem `devices/` symbol stands for.
pub const CORE_DIR: &str = "schemify_core";

/// The version record every written file starts with (xschem 3.4.7's file format).
const VERSION: &str = "v {xschem version=3.4.7 file_version=1.2}\n";

/// Nesting bound for inlining vars and computed params (a cycle is a load error already).
const INLINE_DEPTH_MAX: u32 = 16;

/// Functions a SPICE-quoted `'…'` value may call (ngspice's expression functions).
const SPICE_FUNCS: &[&str] = &[
    "abs", "sqrt", "exp", "ln", "log10", "sin", "cos", "tan", "asin", "acos", "atan", "sinh",
    "cosh", "tanh", "floor", "ceil", "int", "min", "max", "pow",
];

/// What an instance of a cell points at in xschem.
#[derive(Clone, Debug)]
enum Target {
    /// Written by this export into this output directory.
    Own(String),
    /// A `core` cell standing for an xschem `devices/` symbol.
    Device(&'static DeviceMap),
    /// `core:gnd`/`core:vdd`: xschem's global label symbol of that name.
    Label(&'static str),
    /// A cell imported from xschem: its original symbol reference.
    Original(String),
}

/// Exports cells `tops` (indices into `d.cells`) and every cell they instantiate as
/// xschem files.
///
/// Pure and deterministic: the same Design gives byte-identical files. `core` cells are
/// never written when an xschem `devices/` symbol stands for them; cells imported from
/// xschem (header `provenance xschem …`) are referenced by their original symbol unless
/// they are a top.
///
/// # Errors
///
/// No tops, an index outside `d.cells`, or a `core` cell named as a top: one line each.
pub fn export(d: &Design, tops: &[usize]) -> Result<XschemExport, Vec<String>> {
    let mut problems = Vec::new();
    if tops.is_empty() {
        problems.push("no cell to export".to_owned());
    }
    for &t in tops {
        match d.cells.get(t) {
            None => problems.push(format!("top {t}: no such cell")),
            Some(c) if c.lib == "core" => problems.push(format!("{}: a core cell", c.path)),
            Some(_) => {}
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let live = crate::design::closure(d, tops);
    let mut root_dir = ROOT_DIR.to_owned();
    for k in 2..d.libs.len() + 3 {
        if !d.libs.contains_key(&root_dir) {
            break;
        }
        root_dir = format!("{ROOT_DIR}_{k}");
    }
    assert!(
        !d.libs.contains_key(&root_dir),
        "a free root directory name"
    );
    let targets: Vec<Option<Target>> = (0..d.cells.len())
        .map(|ci| live[ci].then(|| target_of(d, ci, tops.contains(&ci), &root_dir)))
        .collect();
    let mut notes = Notes::default();
    let mut files = Vec::new();
    let mut dirs = BTreeSet::new();
    for (ci, t) in targets.iter().enumerate() {
        let Some(Target::Own(dir)) = t else { continue };
        let c = &d.cells[ci];
        dirs.insert(dir.as_str());
        let sc = Scope::of(c, &mut notes);
        lost_attrs(c, &mut notes);
        if c.kind() != FileKind::Shbench {
            let text = write_symbol(c, &sc, &mut notes);
            files.push((format!("{dir}/{}.sym", c.name), text.into_bytes()));
        }
        if c.kind() != FileKind::Shprim {
            let text = write_schematic(d, ci, &targets, &sc, &mut notes);
            files.push((format!("{dir}/{}.sch", c.name), text.into_bytes()));
        }
    }
    files.push(("xschemrc".to_owned(), xschemrc(&dirs).into_bytes()));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        files.windows(2).all(|w| w[0].0 != w[1].0),
        "output paths are unique"
    );
    assert!(
        files.len() > tops.len(),
        "every top writes at least one file"
    );
    Ok(XschemExport { files, notes })
}

fn target_of(d: &Design, ci: usize, top: bool, root_dir: &str) -> Target {
    let c = &d.cells[ci];
    if c.lib == "core" {
        return match c.name.as_str() {
            "gnd" => Target::Label("gnd.sym"),
            "vdd" => Target::Label("vdd.sym"),
            name => match DEVICE_MAP.iter().find(|m| m.cell == name) {
                Some(m) => Target::Device(m),
                None => Target::Own(CORE_DIR.to_owned()),
            },
        };
    }
    if let Some(p) = c
        .header("provenance")
        .and_then(|p| p.strip_prefix("xschem "))
        && !top
    {
        // ponytail: the importer names a library after its directory, so `<lib>/<file>`
        // is the reference relative to the library's parent; `devices` is on xschem's
        // default path itself. A library imported from elsewhere needs its parent on the
        // user's XSCHEM_LIBRARY_PATH.
        let r = p.strip_prefix("devices/").unwrap_or(p);
        return Target::Original(r.to_owned());
    }
    match c.lib.as_str() {
        "" => Target::Own(root_dir.to_owned()),
        lib => Target::Own(lib.to_owned()),
    }
}

// ---------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------

/// Writes `C {symref} x y rot flip {props}`.
fn component(out: &mut String, symref: &str, at: [i32; 2], o: Orient, props: &[String]) {
    let (rot, flip) = rot_flip(o);
    let mut line = format!("C {}", brace(symref));
    for v in at {
        line.push(' ');
        format_coord(v, &mut line);
    }
    writeln!(out, "{line} {rot} {flip} {}", brace(&props.join(" "))).unwrap();
}

/// Writes `T {text} x y rot flip size size {props}`.
fn text_record(
    out: &mut String,
    text: &str,
    at: [Option<i32>; 2],
    o: Orient,
    size: &str,
    props: &[String],
) {
    let (rot, flip) = rot_flip(o);
    let mut line = format!("T {}", brace(text));
    for v in at {
        line.push(' ');
        format_coord(v.unwrap_or(0), &mut line);
    }
    writeln!(
        out,
        "{line} {rot} {flip} {size} {size} {}",
        brace(&props.join(" "))
    )
    .unwrap();
}

/// Writes a line/rect/arc/circle/poly (`vals` = its coordinate values) as `L B A P`.
/// In a symbol, layer 5 (xschem's pin layer) moves to 4 so a box never becomes a pin.
fn shape(
    out: &mut String,
    c: &Cell,
    r: u32,
    kind: RecordKind,
    vals: &[i32],
    symbol: bool,
    notes: &mut Notes,
) {
    assert!(
        vals.len() >= 3,
        "every graphic form has at least three values"
    );
    let mut layer: u32 = c.attr(r, "layer").and_then(|l| l.parse().ok()).unwrap_or(4);
    if symbol && layer == 5 {
        notes.review(
            "layer",
            &c.path,
            "graphic on layer 5 (xschem's pin layer) moved to 4",
        );
        layer = 4;
    }
    let fill = c.attr(r, "fill") == Some("true");
    let mut props = Vec::new();
    if let Some(dash) = c.attr(r, "dash") {
        props.push(kv("dash", dash));
    }
    if c.attr(r, "width").is_some() {
        notes.review("shape", &c.path, "line width has no xschem form");
    }
    let (tag, coords): (char, Vec<i32>) = match kind {
        RecordKind::Line => ('L', vals.to_vec()),
        RecordKind::Rect => {
            props.push(kv("fill", if fill { "true" } else { "false" }));
            ('B', vals.to_vec())
        }
        RecordKind::Arc => ('A', vals.to_vec()),
        RecordKind::Circle => {
            let mut v = vals.to_vec();
            v.extend([0, 360_000]);
            ('A', v)
        }
        _ => {
            let mut v = vals.to_vec();
            // xschem closes a polygon whose last point is its first.
            let open = v.len() >= 4 && v[..2] != v[v.len() - 2..];
            if c.attr(r, "closed") == Some("true") && open {
                v.extend([v[0], v[1]]);
            }
            ('P', v)
        }
    };
    if fill && tag != 'B' {
        props.push(kv("fill", "true"));
    }
    let mut line = format!("{tag} {layer}");
    if tag == 'P' {
        write!(line, " {}", coords.len() / 2).unwrap();
    }
    for v in coords {
        line.push(' ');
        format_coord(v, &mut line);
    }
    writeln!(out, "{line} {}", brace(&props.join(" "))).unwrap();
}

/// xschem `(rot, flip)` of `o`: the inverse of the importer's [`ORIENT_TABLE`].
fn rot_flip(o: Orient) -> (usize, usize) {
    for (rot, row) in ORIENT_TABLE.iter().enumerate() {
        if let Some(flip) = row.iter().position(|x| *x == o) {
            return (rot, flip);
        }
    }
    unreachable!("ORIENT_TABLE holds all eight orientations")
}

/// The `xschemrc` fragment: the output directory on the symbol search path (so
/// `<lib>/<cell>.sym` references resolve), the libraries listed for the reader.
fn xschemrc(dirs: &BTreeSet<&str>) -> String {
    let mut out = String::from(
        "# Schemify export (interop.md §11). Source this file from your xschemrc, or pass it\n\
         # with --rcfile: it puts this directory on the symbol search path.\n",
    );
    for d in dirs {
        writeln!(out, "# library: {d}").unwrap();
    }
    out.push_str("append XSCHEM_LIBRARY_PATH :[file dirname [file normalize [info script]]]\n");
    out
}

// ---------------------------------------------------------------------------------------
// Attribute strings (feature inventory §5.3, §5.4)
// ---------------------------------------------------------------------------------------

/// `key=value`, the value quoted when it holds a separator, a quote, a backslash or `=`, or
/// is empty; `\` and `"` are escaped (xschem strips one level when it reads the value).
fn kv(key: &str, value: &str) -> String {
    assert!(
        !key.is_empty() && !key.contains(['=', ' ']),
        "attribute key {key:?}"
    );
    let quote = value.is_empty()
        || value.contains(|c: char| c.is_whitespace() || matches!(c, ';' | '"' | '\\' | '='));
    let mut out = format!("{key}=");
    if quote {
        out.push('"');
    }
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    if quote {
        out.push('"');
    }
    out
}

/// A brace string: `{…}` with `\`, `{` and `}` escaped.
fn brace(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('{');
    for ch in s.chars() {
        if matches!(ch, '\\' | '{' | '}') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('}');
    assert!(out.len() >= s.len() + 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(keep: &[&str], inline: &[(&str, Option<&str>)]) -> Scope {
        Scope {
            keep: keep.iter().map(|s| (*s).to_owned()).collect(),
            inline: inline
                .iter()
                .map(|(n, d)| ((*n).to_owned(), d.map(str::to_owned)))
                .collect(),
        }
    }

    #[test]
    fn values_follow_the_xschem_paragraph() {
        let sc = scope(
            &["wn", "vdd"],
            &[("w2", Some("wn*2")), ("p", None), ("c", Some("2"))],
        );
        assert_eq!(spice_value("1.5u", &sc, false), Ok("1.5u".into()));
        assert_eq!(spice_value("-40", &sc, false), Ok("-40".into()));
        assert_eq!(spice_value("vdd/2", &sc, false), Ok("'vdd/2'".into()));
        assert_eq!(spice_value("vdd/2", &sc, true), Ok("vdd/2".into()));
        assert_eq!(spice_value("wn^2", &sc, false), Ok("'wn**2'".into()));
        assert_eq!(spice_value("w2+c", &sc, false), Ok("'(wn*2)+2'".into()));
        assert_eq!(spice_value("sqrt(wn)", &sc, false), Ok("'sqrt(wn)'".into()));
        for bad in ["a ? b : c", "\"s\"", "gmid.w(1)", "if(a,b,c)", "p*2", "a<b"] {
            assert_eq!(spice_value(bad, &sc, false), Err("expression"), "{bad}");
        }
        assert_eq!(string_value("'{WP}*2'"), Ok("'WP*2'".into()));
        assert_eq!(string_value("{{x}}"), Ok("{x}".into()));
        assert_eq!(string_value("{a*2}"), Err("template"));
        assert_eq!(xschem_value("bool", "true", &sc, false), Ok("true".into()));
    }

    #[test]
    fn formats_and_texts_convert_by_the_token_rules() {
        let ctx = FmtCtx {
            pins: vec![("D", "D"), ("B_2", "B")],
            inherited: vec!["VDD"],
            params: vec!["W", "L", "savecurrent"],
            computed: vec![("ad", "W*0.29")],
            model: None,
            raw: vec!["W"],
        };
        assert_eq!(
            convert_format("M{name} {D} {B_2} {pins} W={W}u L={L} ad={ad}p {W}", &ctx).unwrap(),
            "M@name @@D @@B @pinlist @VDD W='(@W\\)*1u' L=@L ad='((@W\\)*0.29)*1p' '(@W\\)'"
        );
        assert_eq!(glued_suffix("meg x"), 3);
        assert_eq!(glued_suffix("um"), 0);
        assert_eq!(
            convert_format(
                "{name} {savecurrent ? \"\\n.save i(\" + name + \")\" : \"\"}",
                &ctx
            )
            .unwrap(),
            "@name @savecurrent"
        );
        assert_eq!(convert_format("a@b 50%", &ctx).unwrap(), "a\\@b 50\\%");
        assert!(convert_format("{params}", &ctx).is_err());
        assert!(convert_format("{Z}", &ctx).is_err());
        assert_eq!(
            convert_stext("W={W} {eng(W)}", &ctx),
            ("W=@W ".into(), true)
        );
        assert_eq!(kv("format", "@a\\b"), "format=\"@a\\\\b\"");
        assert_eq!(brace("a{b}\\"), "{a\\{b\\}\\\\}");
    }

    #[test]
    fn orientation_inverts_the_import_table() {
        for (rot, row) in ORIENT_TABLE.iter().enumerate() {
            for (flip, o) in row.iter().enumerate() {
                assert_eq!(rot_flip(*o), (rot, flip));
                assert_eq!(crate::xschem::file::orient(rot as i16, flip as i16), *o);
            }
        }
    }
}
