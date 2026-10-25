//! The module DAG (ARCHITECTURE.md): the root leaves (`design`, `notes`, `check`, `decimal`)
//! and `pdk` name no tool and no driver; the tools never name each other or a driver. Only
//! `use` lines count: doc links may point anywhere.

use std::path::Path;

/// `(directory or file under src/, prefixes its `use crate::…` lines must not have)`.
const RULES: &[(&str, &[&str])] = &[
    (
        "design.rs",
        &["xschem", "virtuoso", "pdk", "job", "cli", "plugin"],
    ),
    (
        "notes.rs",
        &["xschem", "virtuoso", "pdk", "job", "cli", "plugin"],
    ),
    (
        "check.rs",
        &["xschem", "virtuoso", "pdk", "job", "cli", "plugin"],
    ),
    (
        "decimal.rs",
        &["xschem", "virtuoso", "pdk", "job", "cli", "plugin"],
    ),
    (
        "pdk",
        &[
            "xschem", "virtuoso", "job", "cli", "plugin", "design", "check",
        ],
    ),
    ("xschem", &["virtuoso", "job", "cli", "plugin"]),
    ("virtuoso", &["xschem", "job", "cli", "plugin"]),
];

fn rust_files(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        v.iter().for_each(|q| rust_files(q, out));
    } else if p.extension().is_some_and(|x| x == "rs") {
        out.push(p.to_owned());
    }
}

#[test]
fn modules_depend_only_downward() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    for (unit, banned) in RULES {
        let mut files = Vec::new();
        rust_files(&src.join(unit), &mut files);
        assert!(!files.is_empty(), "{unit}: no files (moved? update RULES)");
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, line) in text.lines().enumerate() {
                let l = line.trim_start();
                let Some(rest) = l.strip_prefix("use crate::") else {
                    continue;
                };
                if banned.iter().any(|b| {
                    rest.starts_with(b) && !rest[b.len()..].starts_with(char::is_alphanumeric)
                }) {
                    bad.push(format!(
                        "{}:{}: {l}",
                        f.strip_prefix(&src).unwrap().display(),
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "imports against the DAG:\n{}",
        bad.join("\n")
    );
}
