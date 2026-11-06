//! ADE L states ⇄ benches (spec `interop.md` §9.8, lane L7).
//!
//! ICADVM20.1 has no ADE L session, but its states still live on disk: a directory state
//! `<root>/<lib>/<cell>/<simulator>/<state>/` holds one file per component, a cellview
//! state is the view `<state>` of the cell (`<component>.state` files and `master.tag`).
//! The component files are SKILL source that rebuilds ADE's structs when evaluated.
//! Interop never evaluates them: `sfy_adel.il` dumps every file of a state verbatim
//! (base64) as `adel`/`adelfile` records, and this module reads them as data, through a
//! closed grammar (literals, `let`, `unless`, assignments, `->` fields, `list`, `nconc`,
//! `ncons`, `quote`, `make_sev…Struct()`); a file using anything else is reported and only
//! kept.
//!
//! Spine of the import ([`split`] before `read::import`, [`attach`] after): the components
//! a Maestro test also has (design variables, analyses, outputs, model files, the
//! temperature) become that test's records, named after the state, and `ade::import` maps
//! them onto the bench like any Maestro test; then every file of the state rides in the
//! bench header with the fingerprint of the bench's setup records as the import wrote them:
//!
//! - `vadel <state> <lib> <cell> <simulator> dir|cellview <root> <fingerprint>`;
//! - `vadelfile <name> text|base64 <content>`, one per file, in name order.
//!
//! Export ([`restore`], after `write::export`): while the bench's setup records still have
//! that fingerprint, its Maestro test is replaced by the state's files, which
//! `SfyAdeLImport` writes back byte for byte; once they differ, the bench goes to Maestro
//! as the test (its variables as Maestro variables, which a migrated state would lose) and
//! the report says the state became Maestro.

use std::borrow::Cow;

use schemify_format::{Document, FileKind, RecordKind};

use super::{Export, Import, ImportOptions};
use crate::design::{Design, Files, cell_path, copy_header, manifest_libs};
use crate::map::{CORE_LIB, ROOT_LIB};
use crate::sfyx::{self, Error, Record, i, pairs, s, text};

// Data design (six questions): in = one dump's `adel` groups (a state: ≤ FILES_MAX files,
// ≤ STATE_BYTES_MAX bytes) or one bench header; out = maestro records, or the state's
// records. Rows: a state's files in one `Vec` in dump order (SKILL sorts them). Lifetime:
// one call. Access: each file read once by the component it is.

/// Header attribute: the state and the fingerprint of the bench as imported.
pub const VADEL: &str = "vadel";
/// Header attribute: one file of the state.
pub const VADELFILE: &str = "vadelfile";

/// Files per state; more is refused.
const FILES_MAX: usize = 256;
/// Bytes of all files of one state; more is refused.
const STATE_BYTES_MAX: usize = 1 << 24;
/// Nesting of a component's forms; deeper is not data.
const DEPTH_MAX: usize = 64;

/// One analysis: `(name, enabled, options)`.
type Analysis = (String, bool, Vec<(String, String)>);
/// One output: `(name, point|net, text)`.
type Output = (String, String, String);

/// One ADE L state of a dump.
#[derive(Clone, Debug, Default)]
pub struct State {
    name: String,
    lib: String,
    cell: String,
    sim: String,
    /// `dir` or `cellview`.
    from: String,
    /// The state directory's root (`""` for a cellview state).
    root: String,
    /// `(file name, bytes)` in dump order.
    files: Vec<(String, Vec<u8>)>,
    /// `(reason, detail)` for the report.
    notes: Vec<(&'static str, String)>,
}

// ---------------------------------------------------------------------------- import

/// Import side, before `read::import`: every `adel` group of `sfyx` replaced by the Maestro
/// test it describes, and the states for [`attach`]. A dump without one comes back
/// borrowed.
///
/// # Errors
///
/// The text is not records, or an `adel`/`adelfile` record is malformed (with its line).
pub fn split(sfyx: &str) -> Result<(Cow<'_, str>, Vec<State>), Vec<String>> {
    if !sfyx.contains("(\"adel") {
        return Ok((Cow::Borrowed(sfyx), Vec::new()));
    }
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    if !recs.iter().any(|r| r.tag == "adel") && !recs.iter().any(|r| r.tag == "adelfile") {
        return Ok((Cow::Borrowed(sfyx), Vec::new()));
    }
    let mut out: Vec<Record> = Vec::with_capacity(recs.len());
    let mut states: Vec<State> = Vec::new();
    let mut cur: Option<State> = None;
    let flush = |cur: &mut Option<State>, out: &mut Vec<Record>, states: &mut Vec<State>| {
        if let Some(mut st) = cur.take() {
            out.extend(test_of(&mut st));
            states.push(st);
        }
    };
    for r in recs {
        match r.tag.as_str() {
            "adel" => {
                flush(&mut cur, &mut out, &mut states);
                cur = Some(state_of(&r).map_err(|e| vec![e.to_string()])?);
            }
            "adelfile" => {
                let Some(st) = cur.as_mut() else {
                    return Err(vec![r.err("outside an adel state").to_string()]);
                };
                file_of(&r, st).map_err(|e| vec![e.to_string()])?;
            }
            _ => {
                flush(&mut cur, &mut out, &mut states);
                out.push(r);
            }
        }
    }
    flush(&mut cur, &mut out, &mut states);
    Ok((Cow::Owned(sfyx::write(&out)), states))
}

/// `("adel" state lib cell simulator from root)`.
fn state_of(r: &Record) -> Result<State, Error> {
    r.arity(6)?;
    let st = State {
        name: r.str(0)?.to_owned(),
        lib: r.str(1)?.to_owned(),
        cell: r.str(2)?.to_owned(),
        sim: r.str(3)?.to_owned(),
        from: r.str(4)?.to_owned(),
        root: r.str(5)?.to_owned(),
        ..State::default()
    };
    let word = |x: &str| !x.is_empty() && !x.contains(['/', '\\', '\'']) && x != "." && x != "..";
    if ![&st.name, &st.lib, &st.cell, &st.sim]
        .iter()
        .all(|x| word(x))
    {
        return Err(r.err("state, library, cell and simulator are names, not paths"));
    }
    if !matches!(st.from.as_str(), "dir" | "cellview") {
        return Err(r.err(format!("from `{}` (dir or cellview)", st.from)));
    }
    if st.from == "dir" && !st.root.starts_with('/') {
        return Err(r.err("a directory state's root is an absolute path"));
    }
    Ok(st)
}

/// `("adelfile" name base64)`.
fn file_of(r: &Record, st: &mut State) -> Result<(), Error> {
    r.arity(2)?;
    let name = r.str(0)?;
    if name.is_empty() || name.contains(['/', '\\', '\'']) || name == "." || name == ".." {
        return Err(r.err(format!("file `{name}`: not a file name")));
    }
    if st.files.iter().any(|f| f.0 == name) {
        return Err(r.err(format!("file {name} twice")));
    }
    if st.files.len() >= FILES_MAX {
        return Err(r.err(format!("more than {FILES_MAX} files in one state")));
    }
    let bytes = b64_decode(&r.text(1)?).ok_or_else(|| r.err(format!("{name}: not base64")))?;
    if st.files.iter().map(|f| f.1.len()).sum::<usize>() + bytes.len() > STATE_BYTES_MAX {
        return Err(r.err("a state of more than 16 MiB"));
    }
    st.files.push((name.to_owned(), bytes));
    Ok(())
}

/// The component a file of a state is (`analyses.state` → `analyses`,
/// `outputs.stateScripts` → `outputsScripts`).
fn component(file: &str) -> String {
    if let Some(c) = file.strip_suffix(".state") {
        return c.to_owned();
    }
    file.replace(".stateScripts", "Scripts")
}

/// The text of component `c` of `st`, if it has that file and it is text.
fn comp<'s>(st: &'s State, c: &str) -> Option<&'s str> {
    let f = st.files.iter().find(|f| component(&f.0) == c)?;
    std::str::from_utf8(&f.1).ok()
}

/// The components `test_of` maps.
const MAPPED: [&str; 7] = [
    "ADE_state.info",
    "variables",
    "analyses",
    "outputs",
    "modelSetup",
    "simulatorOptions",
    "simulationFiles",
];

/// The Maestro test records of state `st` (named after it), its report rows in `st.notes`.
fn test_of(st: &mut State) -> Vec<Record> {
    let mut notes: Vec<(&'static str, String)> = Vec::new();
    let name = st.name.clone();
    let fail = |c: &str, e: String, notes: &mut Vec<(&'static str, String)>| {
        notes.push((
            "config",
            format!(
                "ADE L state {name}: component {c} is not data ({e}): kept verbatim, not mapped"
            ),
        ));
    };
    // ADE_state.info: designInfo = '(lib cell view simulator).
    let mut view = "schematic".to_owned();
    let mut sim = st.sim.clone();
    if let Some(t) = comp(st, "ADE_state.info") {
        match run(t).map(|g| global(&g, "designInfo")) {
            Ok(Val::List(v)) if v.len() == 4 => {
                if let (Val::Str(vw), Val::Str(sm)) = (&v[2], &v[3]) {
                    view = vw.clone();
                    sim = sm.clone();
                }
            }
            Ok(_) => fail("ADE_state.info", "no designInfo".into(), &mut notes),
            Err(e) => fail("ADE_state.info", e, &mut notes),
        }
    }
    let mut out = vec![Record::new(
        "maestro",
        vec![s(&st.name), s(&st.lib), s(&st.cell), s(&view), s(&sim)],
    )];
    // Design variables, then the temperature (simulator option temp).
    let mut vars: Vec<(String, String)> = Vec::new();
    if let Some(t) = comp(st, "variables") {
        match variables(t) {
            Ok(v) => vars = v,
            Err(e) => fail("variables", e, &mut notes),
        }
    }
    if let Some(t) = comp(st, "simulatorOptions") {
        match table(t) {
            Ok(rows) => {
                if let Some(Val::Str(tp)) =
                    lookup(&rows, &["opts", "temp"]).filter(|v| **v != Val::Str(String::new()))
                {
                    vars.retain(|v| v.0 != "temperature");
                    vars.push(("temperature".into(), tp.clone()));
                }
            }
            Err(e) => fail("simulatorOptions", e, &mut notes),
        }
    }
    vars.sort_by(|a, b| a.0.cmp(&b.0));
    vars.dedup_by(|a, b| {
        let dup = a.0 == b.0;
        if dup {
            notes.push((
                "config",
                format!("ADE L state {name}: variable {} twice: the first kept", a.0),
            ));
        }
        dup
    });
    for (n, v) in &vars {
        out.push(Record::new("var", vec![s(n), s(v), s("")]));
    }
    if let Some(t) = comp(st, "analyses") {
        match analyses(t) {
            Ok(a) => {
                for (n, en, opts) in a {
                    let kv = pairs(opts.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                    out.push(Record::new("analysis", vec![s(&n), i(i32::from(en)), kv]));
                }
            }
            Err(e) => fail("analyses", e, &mut notes),
        }
    }
    if let Some(t) = comp(st, "modelSetup") {
        match models(t) {
            Ok(m) => {
                for (f, sec) in m {
                    out.push(Record::new("model", vec![s(""), s(&f), s(&sec)]));
                }
            }
            Err(e) => fail("modelSetup", e, &mut notes),
        }
    }
    if let Some(t) = comp(st, "outputs") {
        match outputs(t) {
            Ok((outs, skipped)) => {
                let mut seen: Vec<String> = Vec::new();
                for (n, k, e) in outs {
                    if seen.contains(&n) {
                        notes.push((
                            "output",
                            format!("ADE L state {name}: output {n} twice: the first kept"),
                        ));
                        continue;
                    }
                    out.push(Record::new("output", vec![s(&n), s(&k), s(&e)]));
                    seen.push(n);
                }
                for x in skipped {
                    notes.push((
                        "output",
                        format!("ADE L state {name}: {x}: kept in the state files only"),
                    ));
                }
            }
            Err(e) => fail("outputs", e, &mut notes),
        }
    }
    if let Some(t) = comp(st, "simulationFiles") {
        match table(t) {
            Ok(rows) => {
                if let Some(Val::List(v)) = lookup(&rows, &["definitionFiles"]) {
                    notes.push((
                        "code",
                        format!(
                            "ADE L state {name}: definition files {} are paths on the Virtuoso host: not mapped",
                            v.iter().map(atom_text).collect::<Vec<_>>().join(" ")
                        ),
                    ));
                }
            }
            Err(e) => fail("simulationFiles", e, &mut notes),
        }
    }
    // The rest: what is data stays for the way back; what is not is named.
    let mut kept: Vec<String> = Vec::new();
    for (f, bytes) in &st.files {
        let c = component(f);
        if MAPPED.contains(&c.as_str()) || matches!(f.as_str(), "master.tag" | ".sevSaveDir") {
            continue;
        }
        match std::str::from_utf8(bytes)
            .ok()
            .filter(|t| !t.contains('\0'))
        {
            None => {
                let what = match tar_members(bytes) {
                    Some(m) if !m.is_empty() => {
                        let each: Vec<String> = m
                            .iter()
                            .map(|(n, b)| {
                                match std::str::from_utf8(b)
                                    .map_err(|_| "binary".to_owned())
                                    .and_then(is_data)
                                {
                                    Ok(()) => format!("{n} (data)"),
                                    Err(e) => format!("{n} ({e})"),
                                }
                            })
                            .collect();
                        format!("a tar archive of {}", each.join(", "))
                    }
                    _ => "binary".to_owned(),
                };
                notes.push((
                    "config",
                    format!(
                        "ADE L state {name}: component {c} is {what}: kept verbatim, not mapped"
                    ),
                ));
            }
            Some(t) => match is_data(t) {
                Ok(()) => kept.push(c),
                Err(e) => fail(&c, e, &mut notes),
            },
        }
    }
    if !kept.is_empty() {
        notes.push((
            "config",
            format!(
                "ADE L state {name}: not mapped, kept verbatim for the way back while the bench is unchanged: {}",
                kept.join(" ")
            ),
        ));
    }
    st.notes = notes;
    out
}

/// Ok when `t` reads as data: forms, the statements among them in the closed grammar, and
/// no top-level call of an ADE function (`asiAddToRestorePool(…)` does something only when
/// evaluated).
fn is_data(t: &str) -> Result<(), String> {
    let f = forms(t)?;
    for x in &f {
        if let F::Call(h, _) = x
            && ["asi", "sev", "axl", "mae"]
                .iter()
                .any(|p| h.starts_with(p))
        {
            return Err(format!("calls `{h}`, which only evaluating applies"));
        }
    }
    if f.iter().any(|x| matches!(x, F::Assign(..))) {
        eval_all(&f).map(|_| ())
    } else {
        f.iter().try_for_each(|x| data(x).map(|_| ()))
    }
}

/// The members of a ustar archive, `(name, bytes)`; `None` when `b` is not one.
fn tar_members(b: &[u8]) -> Option<Vec<(String, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= b.len() {
        let h = &b[at..at + 512];
        if h.iter().all(|&x| x == 0) {
            break;
        }
        if &h[257..262] != b"ustar" {
            return None;
        }
        let name = std::str::from_utf8(&h[..100])
            .ok()?
            .trim_end_matches('\0')
            .to_owned();
        let size = std::str::from_utf8(&h[124..136])
            .ok()?
            .trim_matches(['\0', ' ']);
        let size = usize::from_str_radix(size, 8).ok()?;
        out.push((name, b.get(at + 512..at + 512 + size)?));
        at += 512 + size.div_ceil(512) * 512;
    }
    Some(out)
}

/// Import side, after `read::import`: each state's report rows, and its files and the
/// fingerprint of its bench (as written) in that bench's header.
pub fn attach(states: &[State], imp: &mut Import, existing: &Files, opts: &ImportOptions) {
    for st in states {
        let path = bench_path(&st.lib, &st.cell, imp, existing, opts);
        for (why, d) in &st.notes {
            imp.notes.review(why, &path, d);
        }
        let Some(k) = imp.files.iter().position(|f| f.0 == path) else {
            imp.notes.review(
                "config",
                &path,
                format!(
                    "ADE L state {}: no bench was written for {}/{}: the state is not kept",
                    st.name, st.lib, st.cell
                ),
            );
            continue;
        };
        let doc = std::str::from_utf8(&imp.files[k].1)
            .ok()
            .and_then(|t| crate::merge::parse_stash(t, FileKind::Shbench).ok())
            .expect("the import writes a bench that parses");
        let mut out = Document::new(FileKind::Shbench);
        copy_header(&mut out, &doc, &[VADEL, VADELFILE]);
        out.push_attr(VADEL);
        for v in [&st.name, &st.lib, &st.cell, &st.sim, &st.from, &st.root] {
            out.push_value(v);
        }
        out.push_value(&fingerprint(&doc));
        for (f, bytes) in &st.files {
            out.push_attr(VADELFILE);
            out.push_value(f);
            match std::str::from_utf8(bytes)
                .ok()
                .filter(|t| !t.contains('\0'))
            {
                Some(t) => {
                    out.push_value("text");
                    out.push_value(t);
                }
                None => {
                    out.push_value("base64");
                    out.push_value(&b64_encode(bytes));
                }
            }
        }
        for r in 1..doc.record_count() {
            out.copy_record(&doc, r);
        }
        let bytes = schemify_format::write(&out).into_bytes();
        let same = existing.get(&path) == Some(&bytes);
        imp.notes
            .rows
            .retain(|n| !(n.class == "updated" && n.cell == path));
        if existing.contains_key(&path) && !same {
            imp.notes
                .add("updated", &path, "the import changed this file");
        }
        imp.files[k].1 = bytes;
    }
}

/// The bench file the import writes for Virtuoso cell `lib/cell` (as `read` names it:
/// `--into`, then the manifest's directory, then `--lib-dir`).
pub(crate) fn bench_path(
    lib: &str,
    cell: &str,
    imp: &Import,
    existing: &Files,
    opts: &ImportOptions,
) -> String {
    let slib = opts.lib_map.get(lib).cloned().unwrap_or_else(|| match lib {
        ROOT_LIB => String::new(),
        CORE_LIB => "core".into(),
        l => l.into(),
    });
    let manifest = imp
        .files
        .iter()
        .find(|f| f.0 == "schemify.project")
        .map(|f| &f.1)
        .or_else(|| existing.get("schemify.project"))
        .and_then(|m| manifest_libs(m).ok())
        .unwrap_or_default();
    let dir = if slib.is_empty() {
        String::new()
    } else {
        manifest
            .get(&slib)
            .or_else(|| opts.lib_dir.get(&slib))
            .cloned()
            .unwrap_or(slib)
    };
    cell_path(&dir, cell, "shbench")
}

/// The bench records a Maestro test is written from.
const SETUP: &[RecordKind] = &[
    #[cfg(feature = "cadence-records")]
    RecordKind::Test,
    #[cfg(feature = "cadence-records")]
    RecordKind::BenchParam,
    RecordKind::Var,
    RecordKind::Corner,
    RecordKind::Config,
    RecordKind::Sweep,
    RecordKind::Montecarlo,
    RecordKind::Optimize,
    RecordKind::Output,
    RecordKind::Spec,
    RecordKind::Code,
];

/// The fingerprint of `doc`'s setup records (FNV-1a 64 of their canonical text, attributes
/// sorted): what "unchanged since the import" means for an ADE L state and for a Maestro
/// view's stored files (`ade::viewfiles`).
pub fn fingerprint(doc: &Document<'_>) -> String {
    let mut text = String::new();
    for r in 1..doc.record_count() {
        if !SETUP.contains(&doc.record_kind(r)) {
            continue;
        }
        text.push_str(doc.record_word(r));
        for f in doc.fields(r) {
            text.push(' ');
            text.push_str(f);
        }
        text.push('\n');
        let mut attrs: Vec<String> = doc
            .attrs(r)
            .map(|a| {
                let mut l = format!("  {}", doc.attr_key(a));
                for v in doc.attr_values(a) {
                    l.push(' ');
                    l.push_str(v);
                }
                l + "\n"
            })
            .collect();
        attrs.sort();
        text.push_str(&attrs.concat());
        for v in doc.verbatim(r) {
            text.push_str("  | ");
            text.push_str(v);
            text.push('\n');
        }
    }
    crate::merge::digest(&text)
}

// ---------------------------------------------------------------------------- export

/// Export side, after `write::export`: every exported bench with a `vadel` header whose
/// setup records still have the stored fingerprint goes back as its state (the `adel`
/// records in place of its Maestro test, in the library the export writes); one that
/// changed keeps its Maestro test and is reported.
pub fn restore(d: &Design, e: &mut Export) {
    if !d.cells.iter().any(|c| c.header(VADEL).is_some()) {
        return;
    }
    let mut recs = sfyx::read(&e.sfyx).expect("the export's own text reads");
    for c in d.cells.iter().filter(|c| c.kind() == FileKind::Shbench) {
        let Some(a) = c.doc.find_attr(0, VADEL) else {
            continue;
        };
        let row: Vec<&str> = c.doc.attr_values(a).collect();
        let [state, _lib, _cell, sim, from, root, fp] = row[..] else {
            continue;
        };
        // The library the dump writes this bench's cell into.
        let mut vlib: Option<String> = None;
        let mut lib = "";
        for r in &recs {
            match r.tag.as_str() {
                "lib" => lib = r.str(0).unwrap_or(""),
                "cell"
                    if r.str(0).ok() == Some(c.name.as_str())
                        && r.str(1).ok() == Some("design") =>
                {
                    vlib = Some(lib.to_owned());
                }
                _ => {}
            }
        }
        let Some(vlib) = vlib else { continue };
        let at = recs.iter().position(|r| {
            r.tag == "maestro"
                && r.str(1).ok() == Some(vlib.as_str())
                && r.str(2).ok() == Some(c.name.as_str())
        });
        if fingerprint(&c.doc) != fp {
            let test = at
                .and_then(|k| recs[k].str(0).ok())
                .unwrap_or("(none: no analysis)");
            e.notes.review(
                "config",
                &c.path,
                format!(
                    "ADE L state {state} was changed in Schemify: it goes back as Maestro test {test} of {vlib}/{}/maestro, its variables as Maestro variables; the state files are not rewritten",
                    c.name
                ),
            );
            continue;
        }
        let mut st = vec![Record::new(
            "adel",
            vec![s(state), s(&vlib), s(&c.name), s(sim), s(from), s(root)],
        )];
        for a in c.doc.attrs(0).filter(|&a| c.doc.attr_key(a) == VADELFILE) {
            let v: Vec<&str> = c.doc.attr_values(a).collect();
            let [name, enc, content] = v[..] else {
                continue;
            };
            let b64 = match enc {
                "base64" => content.to_owned(),
                _ => b64_encode(content.as_bytes()),
            };
            st.push(Record::new("adelfile", vec![s(name), text(&b64)]));
        }
        let (from_k, to_k) = match at {
            Some(k) => {
                let end = recs[k + 1..]
                    .iter()
                    .position(|r| {
                        r.tag == "maestro"
                            || super::kinds::scope(&r.tag) != Some(super::kinds::Scope::Maestro)
                    })
                    .map_or(recs.len(), |n| k + 1 + n);
                (k, end)
            }
            None => (recs.len() - 1, recs.len() - 1),
        };
        recs.splice(from_k..to_k, st);
    }
    e.sfyx = sfyx::write(&recs);
}

// ---------------------------------------------------------------------- components

/// The design variables of a `variables` component: `(name, expression)` per struct.
fn variables(t: &str) -> Result<Vec<(String, String)>, String> {
    let g = run(t)?;
    let mut out = Vec::new();
    for v in global(&g, "savedComponent").items() {
        let Val::Struct(f) = v else {
            return Err("a variable that is not a struct".into());
        };
        let Some(Val::Str(n)) = field(f, "name") else {
            return Err("a variable without a name".into());
        };
        let e = match field(f, "expression") {
            Some(Val::Str(e)) => e.clone(),
            None | Some(Val::Nil) => String::new(),
            Some(x) => atom_text(x),
        };
        out.push((n.clone(), e));
    }
    Ok(out)
}

/// The outputs of an `outputs` component (`savedComponent` = (outputs envSaveOptions)):
/// `(name, point|net, text)`, and what was skipped.
fn outputs(t: &str) -> Result<(Vec<Output>, Vec<String>), String> {
    let g = run(t)?;
    let comp = global(&g, "savedComponent");
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for o in comp.items().first().map(Val::items).unwrap_or_default() {
        let Val::Struct(f) = o else {
            return Err("an output that is not a struct".into());
        };
        let Some(Val::Str(n)) = field(f, "name").filter(|n| **n != Val::Str(String::new())) else {
            skipped.push("an output without a name".to_owned());
            continue;
        };
        match (field(f, "expression"), field(f, "signal")) {
            (Some(x), _) if *x != Val::Nil => match ade_text(x) {
                Some(e) => out.push((n.clone(), "point".to_owned(), e)),
                None => skipped.push(format!("output {n} (an expression with no ADE text)")),
            },
            (_, Some(Val::Str(sig))) => out.push((n.clone(), "net".to_owned(), sig.clone())),
            _ => skipped.push(format!("output {n} (neither an expression nor a signal)")),
        }
    }
    Ok((out, skipped))
}

/// The analyses of an `analyses` table (`analysis(<name> fields|options <key>) <value>`):
/// `(name, enabled, options)` in order of first appearance, the options those with a
/// value (`t` as `"t"`; `""`, `nil` and lists left out, like `maeGetAnalysis`).
fn analyses(t: &str) -> Result<Vec<Analysis>, String> {
    let mut out: Vec<Analysis> = Vec::new();
    for (k, v) in table(t)? {
        let [
            Val::Atom(head),
            Val::Atom(name),
            Val::Atom(_),
            Val::Atom(key),
        ] = k.items()
        else {
            return Err("a key that is not analysis(<name> <partition> <field>)".into());
        };
        if head != "analysis" {
            return Err(format!("a key {head}(…) in the analyses"));
        }
        let k = match out.iter().position(|a| &a.0 == name) {
            Some(k) => k,
            None => {
                out.push((name.clone(), false, Vec::new()));
                out.len() - 1
            }
        };
        if key == "enable" {
            out[k].1 = v.items() == [Val::T];
            continue;
        }
        let value = match &v {
            Val::Str(x) if !x.is_empty() => x.clone(),
            Val::T => "t".to_owned(),
            Val::Atom(x) => x.clone(),
            _ => continue,
        };
        if !out[k].2.iter().any(|o| &o.0 == key) {
            out[k].2.push((key.clone(), value));
        }
    }
    Ok(out)
}

/// The model files of a `modelSetup` component: `((file section) …)`.
fn models(t: &str) -> Result<Vec<(String, String)>, String> {
    let f = forms(t)?;
    let [one] = &f[..] else {
        return Err("not one list".into());
    };
    let mut out = Vec::new();
    for m in data(one)?.items() {
        match m.items() {
            [Val::Str(file), Val::Str(sec)] => out.push((file.clone(), sec.clone())),
            [Val::Str(file)] => out.push((file.clone(), String::new())),
            _ => return Err("a model that is not (file section)".into()),
        }
    }
    Ok(out)
}

/// The value of table key `key` (`(opts temp)` is `["opts", "temp"]`, a bare word one).
fn lookup<'v>(rows: &'v [(Val, Val)], key: &[&str]) -> Option<&'v Val> {
    rows.iter()
        .find(|(k, _)| match k {
            Val::Atom(a) => key == [a.as_str()],
            Val::List(v) => {
                v.len() == key.len()
                    && v.iter()
                        .zip(key)
                        .all(|(x, y)| *x == Val::Atom((*y).to_owned()))
            }
            _ => false,
        })
        .map(|(_, v)| v)
}

/// The ADE text of an output expression, as SKILL prints it (`ymax(VT("/out"))`): a call
/// `f(a b)`, the four arithmetic operators infix.
fn ade_text(v: &Val) -> Option<String> {
    Some(match v {
        Val::Str(x) => {
            let mut o = String::from("\"");
            for ch in x.chars() {
                if matches!(ch, '"' | '\\') {
                    o.push('\\');
                }
                o.push(ch);
            }
            o + "\""
        }
        Val::Atom(a) => a.clone(),
        Val::List(items) => {
            let (Val::Atom(f), args) = items.split_first()? else {
                return None;
            };
            let args: Vec<String> = args.iter().map(ade_text).collect::<Option<_>>()?;
            let op = match f.as_str() {
                "plus" => "+",
                "difference" => "-",
                "times" => "*",
                "quotient" => "/",
                _ => "",
            };
            match (op, &args[..]) {
                ("", _) => format!("{f}({})", args.join(" ")),
                (op, [a, b]) => format!("({a} {op} {b})"),
                ("-", [a]) => format!("-{a}"),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn atom_text(v: &Val) -> String {
    match v {
        Val::Str(x) | Val::Atom(x) => x.clone(),
        Val::T => "t".into(),
        _ => String::new(),
    }
}

// ------------------------------------------------------------ SKILL source as data

/// A form of a state file, as read.
#[derive(Clone, Debug)]
enum F {
    Str(String),
    Word(String),
    Quote(Box<F>),
    /// `(a b …)`.
    List(Vec<F>),
    /// `f(a b …)`.
    Call(String, Vec<F>),
    /// `x->field`.
    Arrow(Box<F>, String),
    /// `x = e`.
    Assign(Box<F>, Box<F>),
}

/// A value the closed grammar builds.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    Nil,
    T,
    Unbound,
    Str(String),
    /// A symbol or a number, as written.
    Atom(String),
    List(Vec<Val>),
    /// A `make_sev…Struct()`, its fields in assignment order.
    Struct(Vec<(String, Val)>),
}

impl Val {
    fn items(&self) -> &[Val] {
        match self {
            Val::List(v) => v,
            _ => &[],
        }
    }
    fn truthy(&self) -> bool {
        !matches!(self, Val::Nil | Val::Unbound)
    }
}

fn field<'v>(f: &'v [(String, Val)], k: &str) -> Option<&'v Val> {
    f.iter().find(|x| x.0 == k).map(|x| &x.1)
}

fn global(g: &[(String, Val)], k: &str) -> Val {
    field(g, k).cloned().unwrap_or(Val::Nil)
}

/// Reads the forms of `t`.
fn forms(t: &str) -> Result<Vec<F>, String> {
    let mut r = Reader { s: t, i: 0 };
    let mut out = Vec::new();
    loop {
        r.ws();
        if r.i == t.len() {
            return Ok(out);
        }
        out.push(r.form(0)?);
    }
}

struct Reader<'a> {
    s: &'a str,
    i: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.i).copied()
    }

    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b';' {
                self.i = self.s[self.i..]
                    .find('\n')
                    .map_or(self.s.len(), |n| self.i + n);
            } else {
                break;
            }
        }
    }

    fn form(&mut self, depth: usize) -> Result<F, String> {
        if depth > DEPTH_MAX {
            return Err(format!("nested deeper than {DEPTH_MAX}"));
        }
        let mut f = self.primary(depth)?;
        loop {
            self.ws();
            if !self.s[self.i..].starts_with("->") {
                break;
            }
            self.i += 2;
            f = F::Arrow(Box::new(f), self.word()?);
        }
        if self.peek() == Some(b'=') && self.s.as_bytes().get(self.i + 1) != Some(&b'=') {
            self.i += 1;
            let rhs = self.form(depth + 1)?;
            f = F::Assign(Box::new(f), Box::new(rhs));
        }
        Ok(f)
    }

    fn primary(&mut self, depth: usize) -> Result<F, String> {
        self.ws();
        match self.peek() {
            None => Err("unexpected end".into()),
            Some(b'"') => self.string(),
            Some(b'\'') => {
                self.i += 1;
                Ok(F::Quote(Box::new(self.primary(depth + 1)?)))
            }
            Some(b'(') => {
                self.i += 1;
                Ok(F::List(self.items(depth)?))
            }
            Some(b')') => Err(format!("unbalanced `)` at byte {}", self.i)),
            Some(_) => {
                let w = self.word()?;
                if self.peek() == Some(b'(') {
                    self.i += 1;
                    Ok(F::Call(w, self.items(depth)?))
                } else {
                    Ok(F::Word(w))
                }
            }
        }
    }

    fn items(&mut self, depth: usize) -> Result<Vec<F>, String> {
        let mut out = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                None => return Err("an unclosed `(`".into()),
                Some(b')') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => out.push(self.form(depth + 1)?),
            }
        }
    }

    fn word(&mut self) -> Result<String, String> {
        let st = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace()
                || matches!(c, b'(' | b')' | b'"' | b'\'' | b';' | b'=')
                || self.s[self.i..].starts_with("->")
            {
                break;
            }
            self.i += self.s[self.i..].chars().next().map_or(1, char::len_utf8);
        }
        if st == self.i {
            return Err(format!("unexpected `{}`", &self.s[self.i..self.i + 1]));
        }
        Ok(self.s[st..self.i].to_owned())
    }

    fn string(&mut self) -> Result<F, String> {
        self.i += 1;
        let mut out = String::new();
        let mut it = self.s[self.i..].char_indices();
        while let Some((k, c)) = it.next() {
            match c {
                '"' => {
                    self.i += k + 1;
                    return Ok(F::Str(out));
                }
                '\\' => match it.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, x)) => out.push(x),
                    None => break,
                },
                x => out.push(x),
            }
        }
        Err("an unclosed string".into())
    }
}

fn is_number(w: &str) -> bool {
    let w = w.strip_prefix(['-', '+']).unwrap_or(w);
    w.starts_with(|c: char| c.is_ascii_digit())
        || (w.starts_with('.') && w[1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// A form as data, never evaluated: `f(a)` is the list `(f a)`.
fn data(f: &F) -> Result<Val, String> {
    Ok(match f {
        F::Str(x) => Val::Str(x.clone()),
        F::Word(w) => match w.as_str() {
            "nil" => Val::Nil,
            "t" => Val::T,
            "unbound" => Val::Unbound,
            _ => Val::Atom(w.clone()),
        },
        F::Quote(x) => data(x)?,
        F::List(v) => Val::List(v.iter().map(data).collect::<Result<_, _>>()?),
        F::Call(h, v) => {
            let mut out = vec![Val::Atom(h.clone())];
            for x in v {
                out.push(data(x)?);
            }
            Val::List(out)
        }
        F::Arrow(..) | F::Assign(..) => return Err("an assignment where data was expected".into()),
    })
}

/// A table component: forms in (key value) pairs, as data.
fn table(t: &str) -> Result<Vec<(Val, Val)>, String> {
    let f = forms(t)?;
    if f.len() % 2 != 0 {
        return Err("not (key value) pairs".into());
    }
    f.chunks(2)
        .map(|p| Ok((data(&p[0])?, data(&p[1])?)))
        .collect()
}

/// The globals after the statements of `t`, in the closed grammar.
fn run(t: &str) -> Result<Vec<(String, Val)>, String> {
    eval_all(&forms(t)?)
}

fn eval_all(f: &[F]) -> Result<Vec<(String, Val)>, String> {
    let mut env = vec![Vec::new()];
    for x in f {
        eval(x, &mut env, 0)?;
    }
    Ok(env.swap_remove(0))
}

/// Scopes: the globals first, the innermost `let` last.
type Env = Vec<Vec<(String, Val)>>;

fn slot<'e>(env: &'e mut Env, n: &str) -> &'e mut Val {
    let k = (0..env.len())
        .rev()
        .find(|&k| env[k].iter().any(|x| x.0 == n))
        .unwrap_or(0);
    let sc = &mut env[k];
    let j = match sc.iter().position(|x| x.0 == n) {
        Some(j) => j,
        None => {
            sc.push((n.to_owned(), Val::Nil));
            sc.len() - 1
        }
    };
    &mut sc[j].1
}

fn eval(f: &F, env: &mut Env, depth: usize) -> Result<Val, String> {
    if depth > DEPTH_MAX {
        return Err(format!("nested deeper than {DEPTH_MAX}"));
    }
    match f {
        F::Word(w) if !matches!(w.as_str(), "nil" | "t") && !is_number(w) => env
            .iter()
            .rev()
            .find_map(|sc| field(sc, w).cloned())
            .ok_or_else(|| format!("`{w}` is not bound")),
        F::Str(_) | F::Word(_) | F::Quote(_) => data(f),
        F::Arrow(b, k) => match eval(b, env, depth + 1)? {
            Val::Struct(v) => Ok(field(&v, k).cloned().unwrap_or(Val::Nil)),
            _ => Err(format!("->{k} of something that is not a struct")),
        },
        F::Assign(lhs, rhs) => {
            let v = eval(rhs, env, depth + 1)?;
            match &**lhs {
                F::Word(n) => *slot(env, n) = v.clone(),
                F::Arrow(b, k) => {
                    let F::Word(n) = &**b else {
                        return Err("->field of an expression".into());
                    };
                    let Val::Struct(fs) = slot(env, n) else {
                        return Err(format!("{n}->{k} = …: {n} is not a struct"));
                    };
                    match fs.iter_mut().find(|x| &x.0 == k) {
                        Some(x) => x.1 = v.clone(),
                        None => fs.push((k.clone(), v.clone())),
                    }
                }
                _ => return Err("an assignment to an expression".into()),
            }
            Ok(v)
        }
        F::List(v) => match v.split_first() {
            Some((F::Word(h), args)) => apply(h, args, env, depth),
            _ => Err("a list that is not a call".into()),
        },
        F::Call(h, args) => apply(h, args, env, depth),
    }
}

fn apply(h: &str, args: &[F], env: &mut Env, depth: usize) -> Result<Val, String> {
    let each = |env: &mut Env| -> Result<Vec<Val>, String> {
        args.iter().map(|a| eval(a, env, depth + 1)).collect()
    };
    match h {
        "let" => {
            let Some((F::List(names), body)) = args.split_first() else {
                return Err("let without its variable list".into());
            };
            let mut sc = Vec::new();
            for n in names {
                let F::Word(n) = n else {
                    return Err("a let variable that is not a name".into());
                };
                sc.push((n.clone(), Val::Nil));
            }
            env.push(sc);
            let mut last = Val::Nil;
            for b in body {
                last = eval(b, env, depth + 1)?;
            }
            env.pop();
            Ok(last)
        }
        "unless" => {
            let Some((c, body)) = args.split_first() else {
                return Err("unless without its condition".into());
            };
            let mut last = Val::Nil;
            if !eval(c, env, depth + 1)?.truthy() {
                for b in body {
                    last = eval(b, env, depth + 1)?;
                }
            }
            Ok(last)
        }
        "list" => Ok(Val::List(each(env)?)),
        "nconc" => {
            let mut out = Vec::new();
            for v in each(env)? {
                match v {
                    Val::List(x) => out.extend(x),
                    Val::Nil => {}
                    _ => return Err("nconc of something that is not a list".into()),
                }
            }
            Ok(Val::List(out))
        }
        "ncons" => Ok(Val::List(each(env)?)),
        "quote" => match args {
            [x] => data(x),
            _ => Err("quote of more than one form".into()),
        },
        _ if h.starts_with("make_sev") && h.ends_with("Struct") && args.is_empty() => {
            Ok(Val::Struct(Vec::new()))
        }
        _ => Err(format!("calls `{h}`")),
    }
}

// ----------------------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as `base64 -w 0` writes it.
pub(crate) fn b64_encode(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(char::from(B64[(n >> (18 - 6 * k)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes base64 (white space ignored, padding optional); `None` for any other byte.
pub(crate) fn b64_decode(t: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(t.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in t.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for n in 0..70u8 {
            let b: Vec<u8> = (0..n).map(|k| k.wrapping_mul(37)).collect();
            assert_eq!(b64_decode(&b64_encode(&b)).unwrap(), b);
        }
        assert_eq!(b64_encode(b"ab"), "YWI=");
        assert_eq!(b64_decode("YW\nI=").unwrap(), b"ab");
        assert!(b64_decode("a*b").is_none());
    }

    #[test]
    fn the_grammar_is_closed() {
        assert!(is_data("x = 1\ny = list(x \"a\")").is_ok());
        let e = is_data("asiAddToRestorePool( 'nodeSets 'objectList quote(\nnil\n))").unwrap_err();
        assert!(e.contains("calls `asiAddToRestorePool`"), "{e}");
        let e = run("x = system(\"rm -rf /\")").unwrap_err();
        assert!(e.contains("calls `system`"), "{e}");
        assert!(run("x = y").unwrap_err().contains("not bound"));
        let deep = format!("x = {}1{}", "list(".repeat(100), ")".repeat(100));
        assert!(run(&deep).unwrap_err().contains("deeper"));
    }

    #[test]
    fn an_output_expression_prints_as_skill_does() {
        let t = "savedComponent = (let\n(tmp1)\ntmp1 = make_sevOutputStruct()\ntmp1->name = \"d\"\ntmp1->expression = (let\n(tmp1 tmp2 tmp3)\ntmp1 = 'difference\ntmp2 = (let (tmp1 tmp2) tmp1 = 'VT tmp2 = \"/a\" list(tmp1 tmp2))\ntmp3 = 2\nlist(tmp1 tmp2 tmp3)\n)\nlist(tmp1)\n)\nsavedComponent = list(savedComponent nil)";
        let (o, skipped) = outputs(t).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(o, [("d".into(), "point".into(), "(VT(\"/a\") - 2)".into())]);
    }
}
