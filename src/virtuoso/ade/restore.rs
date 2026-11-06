//! A Maestro view's own setup, kept for the way back (spec `interop.md` §9.7): the dump's
//! `maesdb` (the `axlExportSetup` XML) and `maestate` (each test's `active.state` element)
//! ride in the bench header, so every Maestro setting survives a Schemify round trip,
//! mapped or not. On export they go back to Virtuoso, whose loader restores them with
//! `axlImportSetup … 'overwrite` and then applies only the records Schemify changed.
//!
//! Bench header attributes (written by [`attach`], read by [`with_stored`] and
//! [`only_changed`]):
//! - `vmaesdb <test> <xml>`: the mapped test's name and the setup XML, verbatim;
//! - `vmaestate <test> <xml>`: one per test of the view;
//! - `vmaerec <digest> <tag> <key>…`: one per record of each test as Virtuoso had it
//!   (FNV-1a of its canonical form): what "unchanged" means on export. A test's rows follow
//!   its row `vmaerec <digest> maestro <test>`; rows before any such row are the mapped
//!   test's (a bench imported before tests had rows of their own).

use std::collections::BTreeSet;

use super::*;

/// Header attribute: the setup XML.
pub const VMAESDB: &str = "vmaesdb";
/// Header attribute: one test's state.
pub const VMAESTATE: &str = "vmaestate";
/// Header attribute: one record's digest and identity.
pub const VMAEREC: &str = "vmaerec";

/// The identity of a test record: its tag and the fields that name it (§9 records).
fn rec_key(r: &Record) -> Option<Vec<String>> {
    let f = |k: usize| r.str(k).ok().map(str::to_owned);
    let keys = match r.tag.as_str() {
        "var" | "parameter" => vec![f(0)?, f(2)?],
        "testvar" => vec![f(0)?],
        "testenable" => vec![],
        "model" => vec![f(0)?, f(1)?],
        "spec" => vec![f(0)?, f(1)?],
        "analysis" | "corner" | "output" | "defn" => vec![f(0)?],
        _ => return None,
    };
    Some(std::iter::once(r.tag.clone()).chain(keys).collect())
}

fn rec_digest(r: &Record) -> String {
    let mut s = String::new();
    let mut fields = r.fields.clone();
    // An analysis is its fields as a set, form defaults left out: the Spectre block writes
    // the choosers back in its own order (forms.rs).
    if let (true, Ok(kind), Some(Value::List(opts))) =
        (r.tag == "analysis", r.str(0), fields.get_mut(2))
    {
        opts.retain(|o| match o.items() {
            [Value::Str(k), Value::Str(v)] => !v.is_empty() && !super::is_form_default(kind, k, v),
            _ => true,
        });
        opts.sort_by_key(|o| format!("{o:?}"));
    }
    crate::sfyx::write_record(&Record::new(&r.tag, fields), &mut s);
    crate::merge::digest(&s)
}

/// The group a keyed record is applied with: a test's model files are one list, a corner
/// goes with its models and vars, an output with its specs; the rest stand alone.
fn group(key: &[String]) -> String {
    match key[0].as_str() {
        "model" if key[1].is_empty() => "models".into(),
        "corner" => format!("corner {}", key[1]),
        "model" => format!("corner {}", key[1]),
        "var" if !key[2].is_empty() => format!("corner {}", key[2]),
        "spec" => format!("output {}", key[1]),
        _ => key.join(" "),
    }
}

/// The `(digest, key)` rows of `records` (one test's, `maestro` first): the test's own
/// row `(digest maestro <test>)`, then one per keyed record.
fn digests(records: &[Record]) -> Vec<Vec<String>> {
    let head = records.first().and_then(|m| {
        let n = m.str(0).ok()?;
        Some(vec![rec_digest(m), "maestro".to_owned(), n.to_owned()])
    });
    head.into_iter()
        .chain(records.iter().skip(1).filter_map(|r| {
            let k = rec_key(r)?;
            Some(std::iter::once(rec_digest(r)).chain(k).collect())
        }))
        .collect()
}

/// `rows` (each `digest tag key…`) by test: `(test, (digest, key))`, a row before any
/// `maestro` row belonging to `first`.
fn by_test(rows: Vec<Vec<String>>, first: &str) -> Vec<(String, String, Vec<String>)> {
    let mut test = first.to_owned();
    let mut out = Vec::new();
    for row in rows.into_iter().filter(|row| row.len() >= 2) {
        if row[1] == "maestro" {
            test = row.get(2).cloned().unwrap_or_default();
            continue;
        }
        out.push((test.clone(), row[0].clone(), row[1..].to_vec()));
    }
    out
}

/// Import side: when the first of `tests` (one view's, each `maestro` first; `ts` their
/// parse) carries the view's `maesdb`, the bench header gains `vmaesdb`, a `vmaestate` per
/// test and the `vmaerec` rows of every test: those the `maesdb` record holds (a Rust
/// export carries the rows it was restored from), else this dump's own. Nothing changes
/// without `maesdb`.
pub(super) fn attach(tests: &[&[Record]], ts: &[Test], bench: &mut Document<'static>) {
    let (Some(records), Some(t)) = (tests.first(), ts.first()) else {
        return;
    };
    let Some(sdb) = records.iter().find(|r| r.tag == "maesdb") else {
        return;
    };
    let xml = sdb.text(0).expect("parse checked maesdb");
    let carried: Vec<Vec<String>> = sdb
        .list(1)
        .expect("parse checked maesdb")
        .iter()
        .map(|d| {
            d.items()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .collect();
    let rows = if carried.is_empty() {
        ts.iter().flat_map(|t| digests(&t.records())).collect()
    } else {
        carried
    };
    let mut states: Vec<(String, String)> = records
        .iter()
        .filter(|r| r.tag == "maestate")
        .map(|r| {
            let n = r.str(0).expect("checked").to_owned();
            (n, r.text(1).expect("checked"))
        })
        .collect();
    states.sort();
    let mut out = Document::new(bench.kind());
    crate::design::copy_header(&mut out, bench, &[VMAESDB, VMAESTATE, VMAEREC]);
    out.push_attr(VMAESDB);
    out.push_value(&t.name);
    out.push_value(&xml);
    for (n, st) in &states {
        out.push_attr(VMAESTATE);
        out.push_value(n);
        out.push_value(st);
    }
    for row in &rows {
        out.push_attr(VMAEREC);
        for v in row {
            out.push_value(v);
        }
    }
    for r in 1..bench.record_count() {
        out.copy_record(bench, r);
    }
    assert_eq!(out.record_count(), bench.record_count());
    *bench = out;
}

/// The values of every header attribute `key` of `c`.
fn header_rows<'c>(c: &'c Cell, key: &str) -> Vec<Vec<&'c str>> {
    c.doc
        .attrs(0)
        .filter(|&a| c.doc.attr_key(a) == key)
        .map(|a| c.doc.attr_values(a).collect())
        .collect()
}

/// Export side, before the image import: `records` (bench `c`'s test, `maestro` first)
/// gain the stored `maesdb` (with the `vmaerec` rows) and `maestate` records right after
/// `maestro`, when `c`'s header holds them. A test named after the cell for want of an
/// analysis block takes the stored test's name, so the load updates that test in place.
pub fn with_stored(c: &Cell, records: &mut Vec<Record>) {
    let rows = header_rows(c, VMAESDB);
    let Some(&[test, xml]) = rows.first().map(Vec::as_slice) else {
        return;
    };
    if records.first().is_none_or(|r| r.tag != "maestro") {
        return;
    }
    let name = records[0].str(0).unwrap_or("").to_owned();
    let one = records.iter().filter(|r| r.tag == "maestro").count() == 1;
    if one && name != test && !c.records(RecordKind::Code).any(|r| c.field(r, 0) == name) {
        records[0].fields[0] = s(test);
    }
    let rows: Vec<Value> = header_rows(c, VMAEREC)
        .into_iter()
        .map(|row| Value::List(row.into_iter().map(s).collect()))
        .collect();
    let mut extra = vec![Record::new("maesdb", vec![text(xml), Value::List(rows)])];
    for row in header_rows(c, VMAESTATE) {
        if let [n, st] = row[..] {
            extra.push(Record::new("maestate", vec![s(n), text(st)]));
        }
    }
    records.splice(1..1, extra);
    // §9.11: the outputs' enable where it differs from the stored states (lane L11).
    disabled::out_flags(c, records);
}

/// Export side, after the image import: when bench `c` carries its Maestro setup
/// (`vmaesdb`), each test of `records` (one after another, each from its `maestro` record)
/// keeps only what Schemify changed since the import, since the loader restores the rest
/// from the XML: `maestro`, `maesdb`, `maestate`, then every record of a group (see
/// `group`) holding a record whose digest is not stored for that test. A test with no
/// stored rows (one made in Schemify) is sent whole.
///
/// A stored record that is not exported (removed in Schemify, or one the import could not
/// map) is reported (`needs_review`) and Maestro keeps it: nothing is deleted on the
/// Virtuoso side for it, nor for a stored test the bench no longer has. A changed test
/// model list or output (with its specs) missing such a record is not sent either, since
/// the loader replaces those whole.
pub fn only_changed(c: &Cell, records: &mut Vec<Record>, notes: &mut Notes) {
    if c.header(VMAESDB).is_none() || records.get(1).is_none_or(|r| r.tag != "maesdb") {
        return;
    }
    let cell = c.path.as_str();
    let first = records[0].str(0).unwrap_or("").to_owned();
    let stored = by_test(
        header_rows(c, VMAEREC)
            .into_iter()
            .map(|row| row.into_iter().map(str::to_owned).collect())
            .collect(),
        &first,
    );
    let mut tests: Vec<Vec<Record>> = Vec::new();
    for r in std::mem::take(records) {
        match tests.last_mut() {
            Some(t) if r.tag != "maestro" => t.push(r),
            _ => tests.push(vec![r]),
        }
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for mut t in tests {
        let name = t[0].str(0).unwrap_or("").to_owned();
        let mine: Vec<(String, Vec<String>)> = stored
            .iter()
            .filter(|x| x.0 == name)
            .map(|x| (x.1.clone(), x.2.clone()))
            .collect();
        if !mine.is_empty() || name == first {
            changed_only(cell, &name, &mut t, &mine, notes);
        }
        seen.insert(name);
        records.extend(t);
    }
    let gone: BTreeSet<&str> = stored
        .iter()
        .map(|x| x.0.as_str())
        .filter(|n| !seen.contains(*n))
        .collect();
    for n in gone {
        notes.review(
            "config",
            cell,
            format!("maestro test {n} is not in the bench: Maestro keeps it"),
        );
    }
}

/// [`only_changed`] for one test's `records` (`maestro` first) against its `stored`
/// `(digest, key)` rows.
fn changed_only(
    cell: &str,
    test: &str,
    records: &mut Vec<Record>,
    stored: &[(String, Vec<String>)],
    notes: &mut Notes,
) {
    let head = records
        .iter()
        .take_while(|r| matches!(r.tag.as_str(), "maestro" | "maesdb" | "maestate"))
        .count();
    let keyed: Vec<(Record, Option<Vec<String>>)> = records
        .split_off(head)
        .into_iter()
        .map(|r| {
            let k = rec_key(&r);
            (r, k)
        })
        .collect();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for (r, k) in &keyed {
        let Some(k) = k else { continue };
        let d = rec_digest(r);
        if !stored.iter().any(|(sd, sk)| sk == k && *sd == d) {
            changed.insert(group(k));
        }
    }
    for (_, k) in stored {
        if keyed.iter().any(|(_, x)| x.as_ref() == Some(k)) {
            continue;
        }
        let g = group(k);
        notes.review(
            if k[0] == "output" || k[0] == "spec" { "output" } else { "code" },
            cell,
            format!(
                "maestro {}{} is not in the bench (removed, or it has no Schemify form): Maestro keeps it",
                k.join(" "),
                if k[0] == "testvar" || k[0] == "testenable" { format!(" of {test}") } else { String::new() }
            ),
        );
        if (g == "models" || g.starts_with("output ")) && changed.remove(&g) {
            notes.review(
                "code",
                cell,
                format!("maestro {g}: changed in Schemify, but sent whole it would drop {}: Maestro keeps its own", k.join(" ")),
            );
        }
    }
    records.extend(
        keyed
            .into_iter()
            .filter(|(_, k)| k.as_ref().is_none_or(|k| changed.contains(&group(k))))
            .map(|(r, _)| r),
    );
}
