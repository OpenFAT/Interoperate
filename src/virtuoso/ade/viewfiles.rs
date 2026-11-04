//! A Maestro view's setup files, kept verbatim (spec `interop.md` §9.11, lane L11).
//!
//! What neither the `axlExportSetup` XML (`maesdb`) nor the test states (`maestate`) hold
//! lives only in the view's own files: run plans and reliability setups in `maestro.sdb`,
//! optimizer starting states in `states/<name>.sdb`. `sfy_ade.il` dumps every setup file of
//! the view (not its histories or results) as `("maefile" path base64)` records after the
//! view's first `maestro` record; this module keeps them in the bench header ([`split`]
//! before `read::import`, [`attach`] after) with the fingerprint of the bench's setup
//! records as the import wrote them:
//!
//! - `vmaefile <path> text|base64 <content>`, one per file, in dump order;
//! - `vmaefiles <fingerprint>`.
//!
//! Export ([`restore`], after `only_changed`): while the bench still has that fingerprint,
//! the view goes back as its files alone (the loader writes them byte for byte and opens no
//! session); once it differs, the files go first and L1's in-place restore applies
//! Schemify's changes on top, so what has no other carrier still comes from the files.

use std::borrow::Cow;

use schemify_format::{Document, FileKind};

use super::super::adel::{b64_decode, b64_encode, bench_path, fingerprint};
use super::super::{Import, ImportOptions};
use crate::design::{Cell, Files, copy_header};
use crate::notes::Notes;
use crate::sfyx::{self, Error, Record, s, text};

// Data design (six questions): in = one dump's `maefile` records (per view ≤ FILES_MAX
// files, ≤ VIEW_BYTES_MAX bytes) or one bench header; out = header rows or `maefile`
// records. Rows: a view's files in one `Vec` in dump order. Lifetime: one call. Access:
// each file copied once.

/// Header attribute: one file of the view.
pub const VMAEFILE: &str = "vmaefile";
/// Header attribute: the fingerprint of the bench as imported.
pub const VMAEFILES: &str = "vmaefiles";

/// Files per view; more is refused.
const FILES_MAX: usize = 1024;
/// Bytes of all files of one view; more is refused.
const VIEW_BYTES_MAX: usize = 1 << 26;

/// The setup files of one Maestro view of a dump.
#[derive(Clone, Debug, Default)]
pub struct View {
    lib: String,
    cell: String,
    /// `(path under the view, bytes)` in dump order.
    files: Vec<(String, Vec<u8>)>,
}

/// Import side, before `read::import`: every `maefile` record of `sfyx` taken out, grouped
/// by the view of the `maestro` record before it. A dump without one comes back borrowed.
///
/// # Errors
///
/// The text is not records, or a `maefile` record is malformed (with its line).
pub fn split(sfyx: &str) -> Result<(Cow<'_, str>, Vec<View>), Vec<String>> {
    if !sfyx.contains("(\"maefile\" ") {
        return Ok((Cow::Borrowed(sfyx), Vec::new()));
    }
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    let mut out: Vec<Record> = Vec::with_capacity(recs.len());
    let mut views: Vec<View> = Vec::new();
    let mut at: Option<(String, String)> = None;
    for r in recs {
        match r.tag.as_str() {
            "maestro" => {
                at = Some((
                    r.str(1).unwrap_or("").to_owned(),
                    r.str(2).unwrap_or("").to_owned(),
                ));
                out.push(r);
            }
            "maefile" => file_of(&r, at.as_ref(), &mut views).map_err(|e| vec![e.to_string()])?,
            _ => out.push(r),
        }
    }
    Ok((Cow::Owned(sfyx::write(&out)), views))
}

/// `("maefile" path base64)`, into the view of `at`.
fn file_of(r: &Record, at: Option<&(String, String)>, views: &mut Vec<View>) -> Result<(), Error> {
    r.arity(2)?;
    let Some((lib, cell)) = at else {
        return Err(r.err("outside a maestro view"));
    };
    let path = r.str(0)?;
    let word = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains(['\\', '\'']);
    if path.starts_with('/') || !path.split('/').all(word) {
        return Err(r.err(format!("`{path}`: not a path under the view")));
    }
    if !views
        .last()
        .is_some_and(|v| &v.lib == lib && &v.cell == cell)
    {
        views.push(View {
            lib: lib.clone(),
            cell: cell.clone(),
            files: Vec::new(),
        });
    }
    let v = views.last_mut().expect("pushed");
    if v.files.iter().any(|f| f.0 == path) {
        return Err(r.err(format!("file {path} twice")));
    }
    if v.files.len() >= FILES_MAX {
        return Err(r.err(format!("more than {FILES_MAX} files in one view")));
    }
    let bytes = b64_decode(&r.text(1)?).ok_or_else(|| r.err(format!("{path}: not base64")))?;
    if v.files.iter().map(|f| f.1.len()).sum::<usize>() + bytes.len() > VIEW_BYTES_MAX {
        return Err(r.err("a view of more than 64 MiB"));
    }
    v.files.push((path.to_owned(), bytes));
    Ok(())
}

/// Import side, after `read::import`: each view's files and the fingerprint of its bench
/// (as written) in that bench's header.
pub fn attach(views: &[View], imp: &mut Import, existing: &Files, opts: &ImportOptions) {
    for v in views {
        let path = bench_path(&v.lib, &v.cell, imp, existing, opts);
        let Some(k) = imp.files.iter().position(|f| f.0 == path) else {
            imp.notes.review(
                "config",
                &path,
                format!(
                    "maestro view {}/{}: no bench was written for it: its files are not kept",
                    v.lib, v.cell
                ),
            );
            continue;
        };
        let doc = std::str::from_utf8(&imp.files[k].1)
            .ok()
            .and_then(|t| crate::merge::parse_stash(t, FileKind::Shbench).ok())
            .expect("the import writes a bench that parses");
        let mut out = Document::new(FileKind::Shbench);
        copy_header(&mut out, &doc, &[VMAEFILE, VMAEFILES]);
        for (f, bytes) in &v.files {
            out.push_attr(VMAEFILE);
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
        out.push_attr(VMAEFILES);
        out.push_value(&fingerprint(&doc));
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

/// Export side, after `only_changed`: when bench `c` carries its view's files, `records`
/// (the view's tests, `maestro` first) gain them as `maefile` records. While the bench has
/// the fingerprint it was imported with, they replace every other record (the view goes
/// back as its files); otherwise they go right after the first test's stored setup, and the
/// report says which settings came from them.
pub fn restore(c: &Cell, records: &mut Vec<Record>, notes: &mut Notes) {
    if records.first().is_none_or(|r| r.tag != "maestro") {
        return;
    }
    let files: Vec<Record> = c
        .doc
        .attrs(0)
        .filter(|&a| c.doc.attr_key(a) == VMAEFILE)
        .filter_map(|a| {
            let v: Vec<&str> = c.doc.attr_values(a).collect();
            let [path, enc, content] = v[..] else {
                return None;
            };
            let b64 = match enc {
                "base64" => content.to_owned(),
                _ => b64_encode(content.as_bytes()),
            };
            Some(Record::new("maefile", vec![s(path), text(&b64)]))
        })
        .collect();
    if files.is_empty() {
        return;
    }
    if c.header(VMAEFILES) == Some(fingerprint(&c.doc).as_str()) {
        records.truncate(1);
        records.extend(files);
        return;
    }
    let head = 1 + records[1..]
        .iter()
        .take_while(|r| matches!(r.tag.as_str(), "maesdb" | "maestate"))
        .count();
    records.splice(head..head, files);
    notes.review(
        "runplan",
        c.path.as_str(),
        "the bench changed in Schemify: the maestro view's run plans, reliability setups and optimizer starting states come back from its stored files (maestro.sdb, states/) as dumped; Schemify's changes are applied on top",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str =
        "(\"sfyx\" 1 \"t\")\n(\"maestro\" \"T1\" \"l\" \"c\" \"schematic\" \"spectre\")\n";

    fn refused(body: &str) -> String {
        split(&format!("{HEAD}{body}(\"end\")\n"))
            .unwrap_err()
            .join("\n")
    }

    #[test]
    fn the_files_leave_the_dump_grouped_by_view() {
        let text = format!(
            "{HEAD}(\"maefile\" \"states/s.sdb\" \"YWI=\")\n(\"var\" \"A\" \"1\" \"\")\n(\"end\")\n"
        );
        let (rest, views) = split(&text).unwrap();
        assert!(
            !rest.contains("maefile") && rest.contains("(\"var\" \"A\""),
            "{rest}"
        );
        assert_eq!((views[0].lib.as_str(), views[0].cell.as_str()), ("l", "c"));
        assert_eq!(
            views[0].files,
            [("states/s.sdb".to_owned(), b"ab".to_vec())]
        );
        assert!(matches!(split(HEAD), Ok((Cow::Borrowed(_), _))));
    }

    #[test]
    fn malformed_files_are_refused_with_their_line() {
        for (body, want) in [
            (
                "(\"maefile\" \"../x\" \"\")\n",
                "line 3: maefile: `../x`: not a path under the view",
            ),
            (
                "(\"maefile\" \"/etc/x\" \"\")\n",
                "not a path under the view",
            ),
            ("(\"maefile\" \"a//b\" \"\")\n", "not a path under the view"),
            ("(\"maefile\" \"a\" \"*\")\n", "a: not base64"),
            (
                "(\"maefile\" \"a\" \"\")\n(\"maefile\" \"a\" \"\")\n",
                "file a twice",
            ),
        ] {
            let got = refused(body);
            assert!(got.contains(want), "{body}: {got}");
        }
        let got = split("(\"sfyx\" 1 \"t\")\n(\"maefile\" \"a\" \"\")\n(\"end\")\n").unwrap_err();
        assert!(got[0].contains("outside a maestro view"), "{got:?}");
    }
}
