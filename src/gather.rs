//! Which files an xschem import reads (import spec §1, `--root`): every `.sym`/`.sch`/`.cir`
//! under the converted library directories, and from the rest of the search path only what
//! those reference, transitively. Pure over `/`-separated path strings, so the command line
//! (host paths) and the plugin (`${PDK_ROOT}/…` and project paths) share it through [`Job`].
//!
//! ```text
//! Gather::new(convert, roots) → unlisted() → listed(dir, files)…  (one listing per element)
//!   → next(files) → paths to read → next(files) … → [] : inputs() is the import's input set
//! ```
//!
//! Existence is a lookup in the listings, so resolving a reference never costs a read; each
//! round reads what the previous one found referenced.
//!
//! [`Job`]: crate::job::Job

use std::collections::{BTreeMap, BTreeSet};

use crate::design::Files;
use crate::translate::{dir_of, join, normalize};

/// Inputs one import may gather (the translator's own input cap).
pub const INPUTS_MAX: usize = crate::translate::MAX_FILES;

/// The gathering state of one import.
#[derive(Debug)]
pub struct Gather {
    /// The search path in order; the first `convert` elements are converted wholesale.
    path: Vec<String>,
    convert: usize,
    /// Files under each search-path element, relative to it (`None`: not listable).
    listing: BTreeMap<String, Option<BTreeSet<String>>>,
    inputs: BTreeSet<String>,
    /// Inputs whose references were followed.
    scanned: BTreeSet<String>,
    /// Directories whose `.cir` side files were added.
    side_dirs: BTreeSet<String>,
}

impl Gather {
    /// An import converting the library directories `convert` (in search-path order) with
    /// `roots` after them on the search path.
    pub fn new(convert: &[String], roots: &[String]) -> Gather {
        let path: Vec<String> = convert
            .iter()
            .chain(roots)
            .map(|d| normalize(d.trim_end_matches('/')))
            .collect();
        assert!(path.len() == convert.len() + roots.len());
        Gather {
            path,
            convert: convert.len(),
            listing: BTreeMap::new(),
            inputs: BTreeSet::new(),
            scanned: BTreeSet::new(),
            side_dirs: BTreeSet::new(),
        }
    }

    /// The search path, normalised, in order (the translator's `Options::roots`).
    pub fn search_path(&self) -> &[String] {
        &self.path
    }

    /// Search-path elements not listed yet.
    pub fn unlisted(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .path
            .iter()
            .filter(|d| !self.listing.contains_key(*d))
            .cloned()
            .collect();
        v.dedup();
        v
    }

    /// Hands in the listing of `dir` (paths relative to it), `None` when it cannot be listed.
    /// Listing a converted directory makes its `.sym`/`.sch`/`.cir` files inputs.
    pub fn listed(&mut self, dir: &str, files: Option<Vec<String>>) {
        let dir = normalize(dir.trim_end_matches('/'));
        let set: Option<BTreeSet<String>> = files.map(|f| f.into_iter().collect());
        let converted = self.path[..self.convert].contains(&dir);
        if let (true, Some(set)) = (converted, &set) {
            for rel in set.iter().filter(|r| wanted(r)) {
                self.add(join(&dir, rel));
            }
        }
        self.listing.insert(dir, set);
    }

    /// Follows the references of every input in `files` not followed yet and returns the
    /// inputs still to read (neither in `files` nor in `absent`); empty when the input set is
    /// complete. Inputs that could not be read (`absent`) are dropped.
    ///
    /// # Panics
    ///
    /// When a search-path element is still unlisted ([`Gather::unlisted`]).
    pub fn next(&mut self, files: &Files, absent: &BTreeSet<String>) -> Vec<String> {
        assert!(self.unlisted().is_empty(), "list the search path first");
        self.inputs.retain(|p| !absent.contains(p));
        // Bounded: each round only adds inputs, at most INPUTS_MAX of them.
        loop {
            let todo: Vec<String> = self
                .inputs
                .iter()
                .filter(|p| files.contains_key(*p) && !self.scanned.contains(*p))
                .cloned()
                .collect();
            if todo.is_empty() || self.inputs.len() >= INPUTS_MAX {
                break;
            }
            for p in todo {
                self.scanned.insert(p.clone());
                for r in self.references(&p, &files[&p]) {
                    self.add(r);
                }
            }
        }
        self.inputs
            .iter()
            .filter(|p| !files.contains_key(*p) && !absent.contains(*p))
            .cloned()
            .collect()
    }

    /// The gathered inputs, in path order.
    pub fn inputs(&self) -> impl Iterator<Item = &String> {
        self.inputs.iter()
    }

    /// Whether `path` is a file of some listed search-path element.
    fn exists(&self, path: &str) -> bool {
        self.listing.iter().any(|(dir, set)| {
            let rel = path
                .strip_prefix(dir.as_str())
                .and_then(|r| r.strip_prefix('/'));
            matches!((rel, set), (Some(r), Some(s)) if s.contains(r))
        })
    }

    /// `path` relative to the first search-path element holding it.
    fn rel_on_path(&self, path: &str) -> Option<String> {
        self.path.iter().find_map(|d| {
            let r = path.strip_prefix(d.as_str())?.strip_prefix('/')?;
            self.exists(path).then(|| r.to_owned())
        })
    }

    /// Adds `path` and, once per directory, the `.cir` files beside it (side inputs).
    fn add(&mut self, path: String) {
        if self.inputs.len() >= INPUTS_MAX || !self.inputs.insert(path.clone()) {
            return;
        }
        let dir = dir_of(&path).to_owned();
        if !self.side_dirs.insert(dir.clone()) {
            return;
        }
        let side: Vec<String> = self
            .listing
            .iter()
            .filter_map(|(root, set)| {
                let sub = if dir == *root {
                    ""
                } else {
                    dir.strip_prefix(root.as_str())?.strip_prefix('/')?
                };
                Some((root, sub, set.as_ref()?))
            })
            .flat_map(|(root, sub, set)| {
                set.iter()
                    .filter(move |r| r.ends_with(".cir") && dir_of(r) == sub)
                    .map(move |r| join(root, r))
            })
            .collect();
        for c in side {
            self.inputs.insert(c);
        }
    }

    /// The files `path` references that exist on the search path: placed symbols and
    /// schematics (as `translate` resolves them) and, for a symbol, its schematic.
    fn references(&self, path: &str, bytes: &[u8]) -> Vec<String> {
        let Some(file) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|t| crate::xschem::read(t).ok())
        else {
            return Vec::new();
        };
        let here = dir_of(path);
        let mut out: Vec<String> = file
            .instances
            .symref
            .iter()
            .filter_map(|s| self.resolve(here, file.str(*s)))
            .collect();
        if path.ends_with(".sym") {
            let iface = crate::symbol::extract(&file);
            let named = iface
                .attrs
                .get("schematic")
                .map(str::trim)
                .filter(|v| !v.is_empty() && !v.contains('(') && !v.contains('['));
            let stem = path.rsplit_once('.').map_or(path, |(s, _)| s);
            let sch = match named {
                Some(v) => {
                    let v = v.replace("@symname", stem.rsplit('/').next().unwrap_or(stem));
                    self.resolve(here, &v)
                        .or_else(|| self.resolve(here, &format!("{v}.sch")))
                }
                None => Some(format!("{stem}.sch"))
                    .filter(|b| self.exists(b))
                    .or_else(|| {
                        let rel = self.rel_on_path(path)?;
                        let rel = rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s);
                        self.path
                            .iter()
                            .map(|d| join(d, &format!("{rel}.sch")))
                            .find(|c| self.exists(c))
                    }),
            };
            out.extend(sch);
        }
        out
    }

    /// Resolves a reference made from directory `here` the way `translate` does: absolute,
    /// `./` / `../` from `here`, else the first search-path element holding it, then beside.
    /// An extension-less reference means its `.sym`.
    fn resolve(&self, here: &str, symref: &str) -> Option<String> {
        let r = symref.trim();
        if r.is_empty() || r.contains('@') || r.contains('(') {
            return None; // generated or Tcl: translate reviews these
        }
        let r = if r.ends_with(".sym") || r.ends_with(".sch") {
            r.to_owned()
        } else {
            format!("{r}.sym")
        };
        if r.starts_with('/') {
            return Some(normalize(&r)).filter(|p| self.exists(p));
        }
        if r.starts_with("./") || r.starts_with("../") {
            return Some(join(here, &r)).filter(|p| self.exists(p));
        }
        self.path
            .iter()
            .map(|d| join(d, &r))
            .chain(std::iter::once(join(here, &r)))
            .find(|p| self.exists(p))
    }
}

/// Files a converted directory contributes: symbols, schematics and side files.
fn wanted(rel: &str) -> bool {
    rel.ends_with(".sym") || rel.ends_with(".sch") || rel.ends_with(".cir")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBCKT: &str = "v {xschem version=3.4.7 file_version=1.2}\nK {type=subcircuit\nformat=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\n";

    fn ls(paths: &[&str]) -> Option<Vec<String>> {
        Some(paths.iter().map(|p| (*p).to_owned()).collect())
    }

    /// Runs the gathering against an in-memory tree; returns the inputs.
    fn gather(convert: &[&str], roots: &[&str], tree: &[(&str, &str)]) -> Vec<String> {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<String>>();
        let mut g = Gather::new(&s(convert), &s(roots));
        for d in g.unlisted() {
            let prefix = format!("{d}/");
            let files: Vec<&str> = tree
                .iter()
                .filter_map(|(p, _)| p.strip_prefix(prefix.as_str()))
                .collect();
            g.listed(&d, ls(&files));
        }
        let mut files = Files::new();
        let absent = BTreeSet::new();
        for _ in 0..16 {
            let want = g.next(&files, &absent);
            if want.is_empty() {
                return g.inputs().cloned().collect();
            }
            for w in want {
                let body = tree.iter().find(|(p, _)| *p == w).map(|(_, b)| *b).unwrap();
                files.insert(w, body.as_bytes().to_vec());
            }
        }
        panic!("did not converge");
    }

    #[test]
    fn roots_contribute_only_what_is_referenced_transitively() {
        let tb = "v {xschem version=3.4.7 file_version=1.2}\nC {lib/cell.sym} 0 0 0 0 {name=x1}\nC {lib/missing.sym} 0 9 0 0 {name=x2}\n";
        let cell_sch =
            "v {xschem version=3.4.7 file_version=1.2}\nC {lib/leaf.sym} 0 0 0 0 {name=x1}\n";
        let tree = [
            ("${D}/tb.sch", tb),
            ("${P}/lib/cell.sym", SUBCKT),
            ("${P}/lib/cell.sch", cell_sch),
            ("${P}/lib/leaf.sym", SUBCKT),
            ("${P}/lib/models.cir", "* side"),
            ("${P}/lib/unused.sym", SUBCKT),
            ("${P}/lib/demo_tb.sch", tb),
            ("${P}/other/x.cir", "* not beside anything used"),
        ];
        let got = gather(&["${D}"], &["${P}"], &tree);
        assert_eq!(
            got,
            [
                "${D}/tb.sch",
                "${P}/lib/cell.sch",
                "${P}/lib/cell.sym",
                "${P}/lib/leaf.sym",
                "${P}/lib/models.cir",
            ]
        );
    }

    #[test]
    fn a_symbol_pairs_with_its_schematic_along_the_search_path() {
        let tb = "v {xschem version=3.4.7 file_version=1.2}\nC {amp.sym} 0 0 0 0 {name=x1}\n";
        let tree = [
            ("b/schematics/tb.sch", tb),
            (
                "b/schematics/amp.sch",
                "v {xschem version=3.4.7 file_version=1.2}\n",
            ),
            ("b/symbols/amp.sym", SUBCKT),
        ];
        // Both converted: every file is an input anyway.
        assert_eq!(gather(&["b/schematics", "b/symbols"], &[], &tree).len(), 3);
        // Symbols as a root: amp.sym is pulled by tb.sch, amp.sch is converted already.
        let got = gather(&["b/schematics"], &["b/symbols"], &tree);
        assert!(got.contains(&"b/symbols/amp.sym".to_owned()), "{got:?}");
        // Schematics as a root: the symbol's schematic is pulled along the path.
        let tree2 = [
            ("b/tb/tb.sch", tb),
            (
                "b/schematics/amp.sch",
                "v {xschem version=3.4.7 file_version=1.2}\n",
            ),
            ("b/symbols/amp.sym", SUBCKT),
        ];
        let got = gather(&["b/tb"], &["b/symbols", "b/schematics"], &tree2);
        assert!(got.contains(&"b/schematics/amp.sch".to_owned()), "{got:?}");
    }

    #[test]
    fn the_first_search_path_element_wins_and_unreadable_inputs_drop() {
        let tb = "v {xschem version=3.4.7 file_version=1.2}\nC {x.sym} 0 0 0 0 {name=x1}\n";
        let tree = [("d/tb.sch", tb), ("r1/x.sym", SUBCKT), ("r2/x.sym", SUBCKT)];
        let got = gather(&["d"], &["r1", "r2"], &tree);
        assert_eq!(got, ["d/tb.sch", "r1/x.sym"]);

        let mut g = Gather::new(&["d".to_owned()], &[]);
        g.listed("d", ls(&["a.sch"]));
        let mut absent = BTreeSet::new();
        assert_eq!(g.next(&Files::new(), &absent), ["d/a.sch"]);
        absent.insert("d/a.sch".to_owned());
        assert!(g.next(&Files::new(), &absent).is_empty());
        assert_eq!(g.inputs().count(), 0);
    }
}
