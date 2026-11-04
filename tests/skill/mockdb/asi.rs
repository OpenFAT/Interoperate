//! Mock of what `sfy_adel.il` calls to copy ADE L states (spec `interop.md` §9.8): the
//! state files on disk, since ICADVM20.1 has no ADE L session to ask. Its rows go in
//! [`ROWS`], its bodies (all of them file functions) in [`call_files`].
//!
//! Model: directories are implicit in the interpreter's files (a path is a directory when a
//! file lies under it; `getDirFiles` of an empty one answers `("." "..")`). `system` runs only the four commands the script issues, with
//! single-quoted paths: `base64 -w 0 'a' > 'b'`, `base64 -d 'a' > 'b'`, `mkdir -p 'd'`,
//! `rm -f 'f'`; it answers the exit status (0 on success), as Virtuoso's does. A file's
//! bytes are a `String` here, so a decoded file must be UTF-8 (the probe's tar is ASCII).
//! `envGetVal` answers nil (no `.cdsenv`): the scripts fall back to their default.

use std::collections::BTreeMap;

use super::super::V;
use super::MockDb;

/// The rows of this area in the assembled `mockdb::SIGNATURES` (SKILL Language Reference;
/// `envGetVal` from the Cadence Application Infrastructure reference).
pub const ROWS: &[(&str, &str)] = &[
    ("getDirFiles", "t"),
    ("isDir", "t"),
    ("system", "t"),
    ("envGetVal", "t t [S]"),
];

/// Handles a function of this area without files; `None` when `name` is not one.
pub fn call(_db: &mut MockDb, _name: &str, _args: &[V]) -> Option<Result<V, String>> {
    None
}

/// Handles a function of this area; `None` when `name` is not one.
pub fn call_files(
    db: &mut MockDb,
    name: &str,
    args: &[V],
    files: &mut BTreeMap<String, String>,
) -> Option<Result<V, String>> {
    if !ROWS.iter().any(|(n, _)| *n == name) {
        return None;
    }
    if let Some(Err(e)) = db.call_db(name, args) {
        return Some(Err(e));
    }
    let a0 = match args.first() {
        Some(V::Str(x)) => x.to_string(),
        _ => String::new(),
    };
    let dir = a0.trim_end_matches('/').to_owned();
    let under = |f: &String| {
        f.strip_prefix(&dir)
            .and_then(|r| r.strip_prefix('/'))
            .map(str::to_owned)
    };
    Some(Ok(match name {
        "getDirFiles" => {
            let mut names: Vec<String> = files
                .keys()
                .filter_map(under)
                .map(|r| r.split('/').next().unwrap_or_default().to_owned())
                .collect();
            names.sort();
            names.dedup();
            let mut out = vec![V::str("."), V::str("..")];
            out.extend(names.iter().map(|n| V::str(n)));
            V::list(out)
        }
        "isDir" if files.keys().any(|f| under(f).is_some()) => V::T,
        "isDir" => V::Nil,
        "envGetVal" => V::Nil,
        _ => V::Int(system(&a0, files)),
    }))
}

/// Runs one of the script's shell commands on `files`; the exit status.
fn system(cmd: &str, files: &mut BTreeMap<String, String>) -> i64 {
    let w = words(cmd);
    let w: Vec<&str> = w.iter().map(String::as_str).collect();
    match w[..] {
        ["base64", "-w", "0", src, ">", dst] => match files.get(src) {
            Some(t) => {
                let b = b64(t.as_bytes());
                files.insert(dst.to_owned(), b);
                0
            }
            None => 1,
        },
        ["base64", "-d", src, ">", dst] => {
            let Some(b) = files.get(src).and_then(|t| unb64(t)) else {
                return 1;
            };
            match String::from_utf8(b) {
                Ok(t) => {
                    files.insert(dst.to_owned(), t);
                    0
                }
                Err(_) => 1,
            }
        }
        ["mkdir", "-p", _] => 0,
        ["rm", "-f", f] => {
            files.remove(f);
            0
        }
        _ => panic!("mock system: a command the script does not issue: {cmd}"),
    }
}

/// Shell words: blanks split, single quotes group (no escapes inside).
fn words(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut quoted = false;
    for c in cmd.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                cur.get_or_insert_with(String::new);
            }
            ' ' if !quoted => out.extend(cur.take()),
            c => cur.get_or_insert_with(String::new).push(c),
        }
    }
    out.extend(cur);
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(b: &[u8]) -> String {
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            out.push(if k <= c.len() {
                char::from(B64[(n >> (18 - 6 * k)) as usize & 63])
            } else {
                '='
            });
        }
    }
    out
}

fn unb64(t: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in t.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | B64.iter().position(|&x| x == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}
