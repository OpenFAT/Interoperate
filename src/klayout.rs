//! Layout through KLayout (gap H): Schemify does not model layout, it carries it.
//!
//! Virtuoso's side (`skill/sfy_layout.il`) writes a **bundle**, a directory: the view
//! streamed with `strmout -ignoreCurrentTimestamp` (byte-identical runs) as `layout.gds`,
//! the layer map strmout used (`layermap`), the session's merged display resources
//! (`display.drf`, `drDumpDrf`), the technology's LPPs and what the view holds that GDS
//! cannot (`layout.sfyx`), and the view directory's files (`oa/`). [`export`] turns it into
//! project files: the GDS beside the cell as `<cell>.<view>.gds` (format.md §1: library
//! data, not a cell), a KLayout `.lyp`/`.lyt` for the library's technology, header
//! attribute `vlayout <view> <sha256>` on the cell file when there is one, and the stash
//! `.schemify/virtuoso/<lib>/<cell>/<view>/` (`oa/`, `layermap`, `layout.sfyx`, the
//! exported GDS to compare an edit with, `SHA256SUMS`).
//!
//! [`import`] goes back: when the GDS's sha256 is the one exported, the stashed OA files
//! return byte for byte (mode `restore`); otherwise the GDS goes through `strmin` with the
//! export's own layer map (mode `strmin`; without the map a read-only tech fails with
//! XSTRM-74), and the report lists what GDS did not carry. The SKILL side applies the
//! import bundle.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use schemify_format::{Document, RecordKind, parse, write};

use crate::design::{copy_attr, copy_header};
use crate::notes::Notes;
use crate::sfyx::{self, Record};

/// The strmout/strmin `-pinAttNum`: the GDS property that carries pins (probe `stream`).
pub const PIN_ATT: u32 = 100;

/// A bundle or stash: relative path → bytes.
pub type Files = BTreeMap<String, Vec<u8>>;

// ---------------------------------------------------------------------------------- digests

/// The SHA-256 of `data`, as 64 lowercase hex digits (FIPS 180-4).
pub fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut block = |b: &[u8]| {
        let mut w = [0u32; 64];
        for (i, c) in b.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    };
    let full = data.len() / 64 * 64;
    for b in data[..full].chunks_exact(64) {
        block(b);
    }
    let mut tail = data[full..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for b in tail.chunks_exact(64) {
        block(b);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// `sha256sum` text for `files` (path → bytes): `<hex>  <path>` per file, sorted by path,
/// so `sha256sum -c` checks it.
pub fn sha256sums<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut rows: Vec<(String, &str)> = files.into_iter().map(|(p, b)| (sha256(b), p)).collect();
    rows.sort_by(|a, b| a.1.cmp(b.1));
    rows.iter().map(|(h, p)| format!("{h}  {p}\n")).collect()
}

/// Reads `sha256sum` text: path → hex digest.
pub fn read_sha256sums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once("  "))
        .map(|(h, p)| (p.to_owned(), h.to_owned()))
        .collect()
}

// -------------------------------------------------------------------------------- layer map

/// One XStream layer map row: OA layer and purpose ↔ GDS layer and datatype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRow {
    /// OA layer name.
    pub layer: String,
    /// OA purpose name.
    pub purpose: String,
    /// GDS layer number.
    pub gds_layer: u32,
    /// GDS datatype.
    pub datatype: u32,
}

/// Reads an XStream layer map (`<layer> <purpose> <gdsLayer> <datatype> [qualifiers]`, `#`
/// comments); rows whose numbers do not parse are skipped.
pub fn read_layer_map(text: &str) -> Vec<MapRow> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            Some(MapRow {
                layer: (*w.first()?).to_owned(),
                purpose: (*w.get(1)?).to_owned(),
                gds_layer: w.get(2)?.parse().ok()?,
                datatype: w.get(3)?.parse().ok()?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------- display resources

/// A display packet (`drDefinePacket`): how one LPP is drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    /// Stipple name.
    pub stipple: String,
    /// Line style name.
    pub line_style: String,
    /// Fill colour name.
    pub fill: String,
    /// Outline colour name.
    pub outline: String,
    /// `outline`, `solid`, `stipple`, `outlineStipple`, `X` (default `outline`).
    pub fill_style: String,
}

/// The display resources of one display device, from a `drDumpDrf` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drf {
    /// Colour name → r g b.
    pub colors: BTreeMap<String, [u8; 3]>,
    /// Stipple name → bit rows, top row first.
    pub stipples: BTreeMap<String, Vec<Vec<bool>>>,
    /// Line style name → (thickness, pattern bits).
    pub line_styles: BTreeMap<String, (u32, Vec<bool>)>,
    /// Packet name → packet.
    pub packets: BTreeMap<String, Packet>,
}

/// A drf form: an atom or a list (`f(…)` reads as `(f …)`).
#[derive(Clone, Debug, PartialEq)]
enum Sx {
    Atom(String),
    List(Vec<Sx>),
}

impl Sx {
    fn atom(&self) -> Option<&str> {
        match self {
            Sx::Atom(a) => Some(a),
            Sx::List(_) => None,
        }
    }
    fn items(&self) -> &[Sx] {
        match self {
            Sx::List(v) => v,
            Sx::Atom(_) => &[],
        }
    }
}

/// The forms of a SKILL data text: `;` comments, `"…"` strings (unescaped as written),
/// atoms, `( … )`, and `name(` as a list headed by `name`.
fn forms(text: &str) -> Result<Vec<Sx>, String> {
    let mut stack: Vec<Vec<Sx>> = vec![vec![]];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => {
                stack.push(vec![]);
                i += 1;
            }
            b')' => {
                let l = stack.pop().filter(|_| !stack.is_empty());
                let l = l.ok_or_else(|| format!("unbalanced ) at byte {i}"))?;
                stack.last_mut().expect("outer level").push(Sx::List(l));
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                if b[i] == b'"' {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                    i = (i + 1).min(b.len());
                } else {
                    while i < b.len() && !b[i].is_ascii_whitespace() && !b"();".contains(&b[i]) {
                        i += 1;
                    }
                }
                let a = Sx::Atom(text[start..i].trim_matches('"').to_owned());
                if b.get(i) == Some(&b'(') {
                    stack.push(vec![a]);
                    i += 1;
                } else {
                    stack.last_mut().expect("a level").push(a);
                }
            }
        }
    }
    match stack.len() {
        1 => Ok(stack.pop().expect("top level")),
        _ => Err("unbalanced ( at end of file".into()),
    }
}

/// Reads a `drDumpDrf` file: the rows of display `display` (the first `drDefineDisplay`
/// name when there is no `display`).
///
/// # Errors
///
/// Unbalanced parentheses, or a row whose numbers do not parse.
pub fn read_drf(text: &str) -> Result<Drf, String> {
    let all = forms(text)?;
    fn rows<'a>(all: &'a [Sx], head: &'a str) -> impl Iterator<Item = &'a [Sx]> {
        all.iter()
            .filter(move |f| f.items().first().and_then(Sx::atom) == Some(head))
            .flat_map(|f| f.items()[1..].iter().map(Sx::items))
    }
    let names: Vec<&str> = rows(&all, "drDefineDisplay")
        .filter_map(|r| r.first()?.atom())
        .collect();
    let disp = if names.is_empty() || names.contains(&"display") {
        "display"
    } else {
        names[0]
    };
    let ours = |r: &&[Sx]| r.first().and_then(Sx::atom) == Some(disp);
    let at = |r: &[Sx], i: usize| r.get(i).and_then(Sx::atom).unwrap_or("").to_owned();
    let num = |r: &[Sx], i: usize| -> Result<u32, String> {
        r.get(i)
            .and_then(Sx::atom)
            .and_then(|a| a.parse().ok())
            .ok_or_else(|| format!("drf row {}: field {} is not a number", at(r, 1), i + 1))
    };
    let bits = |s: Option<&Sx>| -> Vec<bool> {
        s.map_or(&[][..], Sx::items)
            .iter()
            .map(|b| b.atom() == Some("1"))
            .collect()
    };
    let mut d = Drf::default();
    for r in rows(&all, "drDefineColor").filter(ours) {
        let c = [num(r, 2)?, num(r, 3)?, num(r, 4)?].map(|v| v.min(255) as u8);
        d.colors.insert(at(r, 1), c);
    }
    for r in rows(&all, "drDefineStipple").filter(ours) {
        let rows = r.get(2).map_or(&[][..], Sx::items).iter();
        d.stipples
            .insert(at(r, 1), rows.map(|row| bits(Some(row))).collect());
    }
    for r in rows(&all, "drDefineLineStyle").filter(ours) {
        d.line_styles.insert(at(r, 1), (num(r, 2)?, bits(r.get(3))));
    }
    for r in rows(&all, "drDefinePacket").filter(ours) {
        let fill_style = match at(r, 6) {
            s if s.is_empty() => "outline".to_owned(),
            s => s,
        };
        d.packets.insert(
            at(r, 1),
            Packet {
                stipple: at(r, 2),
                line_style: at(r, 3),
                fill: at(r, 4),
                outline: at(r, 5),
                fill_style,
            },
        );
    }
    Ok(d)
}

// --------------------------------------------------------------------------- the bundle

/// What `layout.sfyx` says: the view, its technology and what it holds beyond GDS.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// Virtuoso library.
    pub lib: String,
    /// Cell.
    pub cell: String,
    /// View.
    pub view: String,
    /// The technology library attached to `lib`.
    pub tech: String,
    /// `techGetDBUPerUU(tech "maskLayout")` as printed (`1000.0`).
    pub dbu_per_uu: String,
    /// The technology's LPPs: (layer, purpose) → (packet, priority, visible).
    pub lps: BTreeMap<(String, String), (String, i32, bool)>,
    /// Object counts of the view that GDS does not carry as such: kind → count.
    pub counts: BTreeMap<String, i32>,
    /// PCell instances: (instance, master library, master cell, parameters as text).
    pub pcells: Vec<[String; 4]>,
}

/// Reads `layout.sfyx`: `("layout" lib cell view tech)`, `("dbu" text)`,
/// `("lp" layer purpose packet priority visible)`, `("oa" kind count)`,
/// `("pcell" inst lib cell params)`, between the `("sfyx" 1 …)` header and `("end")`.
///
/// # Errors
///
/// Not a complete `.sfyx` text, no `layout` record, or a record with wrong fields.
pub fn read_info(text: &str) -> Result<Info, String> {
    let recs = sfyx::read(text).map_err(|e| format!("layout.sfyx: {e}"))?;
    sfyx::require_complete(&recs).map_err(|e| format!("layout.sfyx: {e}"))?;
    let mut info = Info::default();
    let e = |e: sfyx::Error| format!("layout.sfyx: {e}");
    for r in &recs {
        let s = |i| r.str(i).map(str::to_owned);
        match r.tag.as_str() {
            "layout" => {
                r.arity(4).map_err(e)?;
                [info.lib, info.cell, info.view, info.tech] =
                    [s(0), s(1), s(2), s(3)].map(|x| x.unwrap_or_default());
            }
            "dbu" => info.dbu_per_uu = s(0).map_err(e)?,
            "lp" => {
                r.arity(5).map_err(e)?;
                info.lps.insert(
                    (s(0).map_err(e)?, s(1).map_err(e)?),
                    (
                        s(2).map_err(e)?,
                        r.int(3).map_err(e)?,
                        r.str(4).map_err(e)? == "t",
                    ),
                );
            }
            "oa" => {
                info.counts.insert(s(0).map_err(e)?, r.int(1).map_err(e)?);
            }
            "pcell" => {
                r.arity(4).map_err(e)?;
                info.pcells
                    .push([s(0), s(1), s(2), s(3)].map(|x| x.unwrap_or_default()));
            }
            _ => {}
        }
    }
    if info.lib.is_empty() || info.cell.is_empty() || info.view.is_empty() {
        return Err("layout.sfyx: no (\"layout\" lib cell view tech) record".into());
    }
    Ok(info)
}

// ------------------------------------------------------------------------- KLayout files

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn hex(c: Option<&[u8; 3]>) -> String {
    c.map_or(String::new(), |c| {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    })
}

/// The KLayout layer properties (`.lyp`) of `map`: one `<properties>` per row, in LPP
/// priority order (rows without an LPP last, in map order), named `<layer>.<purpose>`,
/// source `<gdsLayer>/<datatype>@1`. Colours come from the packet's fill and outline, the
/// line style's thickness is the width, visibility is the LPP's. Fill style `solid` is
/// dither `I0`, `outline` hollow (`I1`), `X` hollow with a cross (`xfill`), `stipple` and
/// `outlineStipple` the stipple bit for bit as a custom pattern; a line style with a gap is a
/// custom line style. KLayout's `C<k>` names the custom entry of `<order>` k+1.
pub fn lyp(map: &[MapRow], info: &Info, drf: &Drf) -> String {
    let mut rows: Vec<(i32, usize, &MapRow)> = map
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let lp = info.lps.get(&(m.layer.clone(), m.purpose.clone()));
            (lp.map_or(i32::MAX, |l| l.1), i, m)
        })
        .collect();
    rows.sort_by_key(|r| (r.0, r.1));
    let mut stipples: Vec<&str> = vec![];
    let mut lines: Vec<&str> = vec![];
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<layer-properties>\n");
    for (_, _, m) in rows {
        let lp = info.lps.get(&(m.layer.clone(), m.purpose.clone()));
        let pk = lp.and_then(|l| drf.packets.get(&l.0));
        let (mut dither, mut xfill) = ("I1".to_owned(), false);
        let (mut line, mut width) = ("I0".to_owned(), 1);
        if let Some(p) = pk {
            match p.fill_style.as_str() {
                "solid" => dither = "I0".into(),
                "X" => xfill = true,
                "stipple" | "outlineStipple" if drf.stipples.contains_key(&p.stipple) => {
                    let k = index_of(&mut stipples, &p.stipple);
                    dither = format!("C{k}");
                }
                _ => {}
            }
            if let Some((t, bits)) = drf.line_styles.get(&p.line_style) {
                width = *t;
                if bits.contains(&false) {
                    line = format!("C{}", index_of(&mut lines, &p.line_style));
                }
            }
        }
        let _ = write!(
            out,
            " <properties>\n  <frame-color>{}</frame-color><fill-color>{}</fill-color>\n  \
             <dither-pattern>{dither}</dither-pattern><line-style>{line}</line-style>\n  \
             <valid>true</valid><visible>{}</visible><transparent>false</transparent>\
             <width>{width}</width><xfill>{xfill}</xfill>\n  \
             <name>{}.{}</name><source>{}/{}@1</source>\n </properties>\n",
            hex(pk.and_then(|p| drf.colors.get(&p.outline))),
            hex(pk.and_then(|p| drf.colors.get(&p.fill))),
            lp.is_none_or(|l| l.2),
            xml(&m.layer),
            xml(&m.purpose),
            m.gds_layer,
            m.datatype,
        );
    }
    let pat = |bits: &[bool]| {
        bits.iter()
            .map(|&b| if b { '*' } else { '.' })
            .collect::<String>()
    };
    for (k, s) in stipples.iter().enumerate() {
        out.push_str(" <custom-dither-pattern><pattern>");
        for row in &drf.stipples[*s] {
            let _ = write!(out, "<line>{}</line>", pat(row));
        }
        let _ = writeln!(
            out,
            "</pattern><order>{}</order><name>{}</name></custom-dither-pattern>",
            k + 1,
            xml(s)
        );
    }
    for (k, s) in lines.iter().enumerate() {
        let _ = writeln!(
            out,
            " <custom-line-style><pattern>{}</pattern><order>{}</order><name>{}</name></custom-line-style>",
            pat(&drf.line_styles[*s].1),
            k + 1,
            xml(s)
        );
    }
    out.push_str("</layer-properties>\n");
    out
}

fn index_of<'a>(v: &mut Vec<&'a str>, s: &'a str) -> usize {
    v.iter().position(|x| *x == s).unwrap_or_else(|| {
        v.push(s);
        v.len() - 1
    })
}

/// The KLayout technology (`.lyt`) of `info`'s tech: its dbu (1 / dbuPerUU µm), the `.lyp`
/// file `lyp_file`, and `map` as reader layer map (`'<l>/<d> : <layer>.<purpose> (<l>/<d>)'`,
/// so layers read named and write back with their numbers), properties and texts kept.
///
/// # Errors
///
/// A dbuPerUU that is not a positive number.
pub fn lyt(map: &[MapRow], info: &Info, lyp_file: &str) -> Result<String, String> {
    let per: f64 = info
        .dbu_per_uu
        .parse()
        .ok()
        .filter(|v: &f64| *v > 0.0)
        .ok_or_else(|| {
            format!(
                "dbu per user unit {:?} is not a positive number",
                info.dbu_per_uu
            )
        })?;
    let rows: Vec<String> = map
        .iter()
        .map(|m| {
            format!(
                "'{l}/{d} : {}.{} ({l}/{d})'",
                m.layer,
                m.purpose,
                l = m.gds_layer,
                d = m.datatype
            )
        })
        .collect();
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<technology>\n \
         <name>{n}</name><description>{n} (Virtuoso, schemify-interop layout-export)</description><dbu>{}</dbu>\n \
         <layer-properties_file>{}</layer-properties_file><add-other-layers>true</add-other-layers>\n \
         <reader-options><common><create-other-layers>true</create-other-layers>\
         <layer-map>{}</layer-map><enable-properties>true</enable-properties>\
         <enable-text-objects>true</enable-text-objects></common></reader-options>\n</technology>\n",
        1.0 / per,
        xml(lyp_file),
        xml(&format!("layer_map({})", rows.join(";"))),
        n = xml(&info.tech),
    ))
}

// ------------------------------------------------------------------------------- export

/// The stash directory of a view, project-relative.
pub fn stash_dir(lib: &str, cell: &str, view: &str) -> String {
    format!(".schemify/virtuoso/{lib}/{cell}/{view}")
}

/// The project files of an export bundle (relative path → bytes): `bundle` holds
/// `layout.sfyx`, `display.drf`, `layermap`, `layout.gds`, `oa/<file>…` and optionally
/// `extracted/<view>.scs` (an extracted netlist, see [`with_code`]). `lib_dir` is the
/// Schemify directory of the Virtuoso library; `cell_file` the cell's file there, if any
/// (path, bytes), which gets header `vlayout <view> <sha256>` (and the extracted netlists).
///
/// # Errors
///
/// A bundle file missing or malformed, or a cell file that does not parse.
pub fn export(
    bundle: &Files,
    lib_dir: &str,
    cell_file: Option<(&str, &[u8])>,
) -> Result<Files, String> {
    let get = |p: &str| bundle.get(p).ok_or_else(|| format!("bundle: no {p}"));
    let text = |p: &str| -> Result<&str, String> {
        std::str::from_utf8(get(p)?).map_err(|_| format!("bundle: {p} is not UTF-8"))
    };
    let info = read_info(text("layout.sfyx")?)?;
    let map = read_layer_map(text("layermap")?);
    if map.is_empty() {
        return Err("bundle: layermap has no rows".into());
    }
    let drf = read_drf(text("display.drf")?)?;
    let gds = get("layout.gds")?;
    let oa: Vec<(&str, &[u8])> = bundle
        .iter()
        .filter(|(p, _)| p.starts_with("oa/"))
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    if oa.is_empty() {
        return Err("bundle: no oa/ files (the view directory's)".into());
    }
    let (cell, view) = (&info.cell, &info.view);
    let gds_name = format!("{cell}.{view}.gds");
    let digest = sha256(gds);
    let stash = stash_dir(&info.lib, cell, view);
    let mut out = Files::new();
    let lyp_name = format!("{}.lyp", info.tech);
    out.insert(
        format!("{lib_dir}/{lyp_name}"),
        lyp(&map, &info, &drf).into_bytes(),
    );
    out.insert(
        format!("{lib_dir}/{}.lyt", info.tech),
        lyt(&map, &info, &lyp_name)?.into_bytes(),
    );
    out.insert(format!("{lib_dir}/{gds_name}"), gds.clone());
    out.insert(format!("{stash}/{gds_name}"), gds.clone());
    let mut sums: Vec<(&str, &[u8])> = oa.clone();
    sums.push((&gds_name, gds));
    out.insert(format!("{stash}/SHA256SUMS"), sha256sums(sums).into_bytes());
    for (p, b) in oa {
        out.insert(format!("{stash}/{p}"), b.to_vec());
    }
    for p in ["layermap", "layout.sfyx"] {
        out.insert(format!("{stash}/{p}"), get(p)?.clone());
    }
    if let Some((path, bytes)) = cell_file {
        let mut doc = with_vlayout(bytes, view, &digest)?;
        for (p, b) in bundle {
            if let Some(v) = p
                .strip_prefix("extracted/")
                .and_then(|v| v.strip_suffix(".scs"))
            {
                let netlist = String::from_utf8_lossy(b);
                doc = with_code(&doc, v, &netlist, cell)?;
            }
        }
        out.insert(path.to_owned(), doc);
    }
    Ok(out)
}

/// `cell` (a cell file's bytes) with header attribute `vlayout <view> <digest>`, replacing
/// the one of the same view.
///
/// # Errors
///
/// The file does not parse.
pub fn with_vlayout(cell: &[u8], view: &str, digest: &str) -> Result<Vec<u8>, String> {
    let doc = parse(cell).map_err(|e| format!("cell file: {e}"))?;
    let mut out = Document::new(doc.kind());
    copy_header(&mut out, &doc, &["vlayout"]);
    for a in doc.attrs(0) {
        if doc.attr_key(a) == "vlayout" && doc.attr_value(a, 0) != Some(view) {
            copy_attr(&mut out, &doc, a);
        }
    }
    out.push_attr("vlayout");
    out.push_value(view);
    out.push_value(digest);
    for r in 1..doc.record_count() {
        out.copy_record(&doc, r);
    }
    Ok(write(&out).into_bytes())
}

/// `cell` (a cell file's bytes) with record `code <view> spectre` holding an extracted
/// netlist (format.md rank 6 `code`: "an extracted netlist"), replacing the one of that
/// name. The text is emitted inside the cell's definition, so of a netlist with
/// `subckt <cell_name> …` only that subcircuit's body is kept.
///
/// # Errors
///
/// The file does not parse.
pub fn with_code(
    cell: &[u8],
    view: &str,
    netlist: &str,
    cell_name: &str,
) -> Result<Vec<u8>, String> {
    let doc = parse(cell).map_err(|e| format!("cell file: {e}"))?;
    let mut out = Document::new(doc.kind());
    copy_header(&mut out, &doc, &[]);
    for r in 1..doc.record_count() {
        if !(doc.record_kind(r) == RecordKind::Code && doc.field(r, 0) == Some(view)) {
            out.copy_record(&doc, r);
        }
    }
    out.begin_record(RecordKind::Code);
    out.push_field(view);
    out.push_field("spectre");
    for line in subckt_body(netlist, cell_name).lines() {
        out.push_verbatim(line.trim_end_matches('\r'));
    }
    Ok(write(&out).into_bytes())
}

/// The lines between `subckt <name> …` and its `ends`, or all of `netlist` without one.
fn subckt_body<'a>(netlist: &'a str, name: &str) -> &'a str {
    let head = |l: &str| {
        let mut w = l.split_whitespace();
        matches!(w.next(), Some("subckt" | ".subckt")) && w.next() == Some(name)
    };
    let mut off = 0;
    let mut start = None;
    for l in netlist.split_inclusive('\n') {
        match start {
            None if head(l) => start = Some(off + l.len()),
            Some(s) if matches!(l.split_whitespace().next(), Some("ends" | ".ends")) => {
                return &netlist[s..off];
            }
            _ => {}
        }
        off += l.len();
    }
    netlist
}

// ------------------------------------------------------------------------------- import

/// How an import bundle brings the view back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The GDS is the one exported: the stashed OA files, byte for byte.
    Restore,
    /// The GDS was edited: `strmin` with the export's layer map.
    Strmin,
}

/// An import bundle: its files (relative path → bytes) and the report.
#[derive(Clone, Debug)]
pub struct Import {
    /// Restore or strmin.
    pub mode: Mode,
    /// `layout.sfyx` (the plan), `oa.sha256`, and `oa/…` (restore) or `layout.gds` and
    /// `layermap` (strmin).
    pub files: Files,
    /// What a strmin import does not carry back (empty for a restore).
    pub notes: Notes,
}

/// The import bundle of a view: `stash` is its stash directory's files (relative to it),
/// `gds` the project's `<cell>.<view>.gds`.
///
/// # Errors
///
/// A stash file missing or malformed, or a stashed OA file whose digest is not the one
/// `SHA256SUMS` recorded (the stash was changed: nothing to restore from).
pub fn import(stash: &Files, gds: &[u8]) -> Result<Import, String> {
    let text = |p: &str| -> Result<&str, String> {
        let b = stash.get(p).ok_or_else(|| format!("stash: no {p}"))?;
        std::str::from_utf8(b).map_err(|_| format!("stash: {p} is not UTF-8"))
    };
    let info = read_info(text("layout.sfyx")?)?;
    let sums = read_sha256sums(text("SHA256SUMS")?);
    let gds_name = format!("{}.{}.gds", info.cell, info.view);
    let exported = sums
        .get(&gds_name)
        .ok_or_else(|| format!("stash: SHA256SUMS has no {gds_name}"))?;
    let mut oa_sums = String::new();
    for (p, h) in sums.iter().filter(|(p, _)| p.starts_with("oa/")) {
        let b = stash.get(p).ok_or_else(|| format!("stash: no {p}"))?;
        if sha256(b) != *h {
            return Err(format!(
                "stash: {p} is not the file exported (sha256 differs)"
            ));
        }
        let _ = writeln!(oa_sums, "{h}  {}", &p[3..]);
    }
    let mode = if sha256(gds) == *exported {
        Mode::Restore
    } else {
        Mode::Strmin
    };
    let mut files = Files::new();
    let plan = [
        Record::new(
            "sfyx",
            vec![sfyx::i(1), sfyx::s("schemify-interop layout-import")],
        ),
        Record::new(
            "layout",
            [&info.lib, &info.cell, &info.view, &info.tech]
                .map(|s| sfyx::s(s))
                .to_vec(),
        ),
        Record::new(
            "mode",
            vec![sfyx::s(if mode == Mode::Restore {
                "restore"
            } else {
                "strmin"
            })],
        ),
        Record::new("end", vec![]),
    ];
    files.insert("layout.sfyx".into(), sfyx::write(&plan).into_bytes());
    files.insert("oa.sha256".into(), oa_sums.into_bytes());
    let mut notes = Notes::default();
    match mode {
        Mode::Restore => {
            for (p, b) in stash
                .iter()
                .filter(|(p, _)| sums.contains_key(*p) && p.starts_with("oa/"))
            {
                files.insert(p.clone(), b.clone());
            }
        }
        Mode::Strmin => {
            files.insert("layout.gds".into(), gds.to_vec());
            files.insert("layermap".into(), text("layermap")?.as_bytes().to_vec());
            losses(&info, &mut notes);
        }
    }
    Ok(Import { mode, files, notes })
}

/// The report rows of a strmin import: what the exported view held that GDS does not carry.
fn losses(info: &Info, notes: &mut Notes) {
    let cell = format!("{}/{}", info.lib, info.cell);
    let mut row = |d: String| notes.review("layout", &cell, d);
    row(format!(
        "{}/{} edited outside Virtuoso: rebuilt from GDS by strmin, not restored",
        info.cell, info.view
    ));
    for [inst, lib, master, params] in &info.pcells {
        row(format!(
            "PCell instance {inst} of {lib}/{master} ({params}) becomes an instance of a plain cell: its parameters are lost"
        ));
    }
    let what = |k: &str| match k {
        "net" => "nets: only pins come back (GDS property 100), the other connectivity is lost",
        "prop" => "cellview properties: lost",
        "shapeprop" => "shape properties: lost",
        "label" => "labels: font and height do not survive KLayout (font 0)",
        "via" => "vias: come back as instances of their via cells",
        "blockage" | "row" | "marker" | "group" | "boundary" => "not carried by GDS: lost",
        _ => "not carried by GDS",
    };
    for (k, n) in info.counts.iter().filter(|(_, n)| **n > 0) {
        row(format!("{n} {k}: {}", what(k)));
    }
    row("constraints of the view: not carried by GDS".into());
    row("cells below the top that already exist in the library are kept (strmin -writeMode noOverwrite): an edit inside one is not imported".into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_fips_vectors() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 1000 bytes: several full blocks and a tail.
        let a = vec![b'a'; 1000];
        assert_eq!(
            sha256(&a),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn a_subckt_body_is_cut_from_a_netlist() {
        let n = "// x\nsubckt inv a y\nM0 (y a 0 0) nch\nends inv\n";
        assert_eq!(subckt_body(n, "inv"), "M0 (y a 0 0) nch\n");
        assert_eq!(subckt_body(n, "other"), n);
    }
}
