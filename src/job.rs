//! One conversion as a pure state machine (spec `interop.md` §1): the command line answers
//! its reads from the file system, the wasm guest through the host's inverted I/O, and both
//! get identical results because neither holds any logic of its own.
//!
//! ```text
//! Job::new(command) → loop { step() → Read(paths) → supply(path, bytes|None)…
//!                                   | Done { writes, summary } | Failed(problems) }
//! ```

use std::collections::BTreeSet;

use crate::design::{Files, LoadError, load};
use crate::map::Mapping;
use crate::notes::Notes;
use crate::virtuoso::{self, ExportOptions, ImportOptions};

/// What to convert. Paths are project-relative.
#[derive(Clone, Debug)]
pub enum Command {
    /// Schemify cells → one `.sfyx` file.
    VirtuosoExport {
        /// Cell files to export (with everything they instantiate).
        tops: Vec<String>,
        /// Output `.sfyx` path.
        out: String,
        /// Export settings.
        opts: ExportOptions,
    },
    /// A `.sfyx` file → Schemify files, merged with existing ones.
    VirtuosoImport {
        /// The dump to read.
        sfyx: String,
        /// Import settings.
        opts: ImportOptions,
    },
    /// Schemify cells → xschem `.sym`/`.sch` files under `out_dir`.
    XschemExport {
        /// Cell files to export.
        tops: Vec<String>,
        /// Output directory.
        out_dir: String,
    },
    /// Writes `schemify.il` (and the probe) into `dir`.
    InstallSkill {
        /// Output directory.
        dir: String,
    },
    /// xschem libraries → Schemify files under `out_dir` (import spec; `--root` as
    /// [`crate::gather`] gathers it).
    XschemImport(Box<XschemImport>),
}

/// The settings of [`Command::XschemImport`]. Paths are whatever the driver reads: host
/// paths on the command line, project or `${VAR}/…` paths in the plugin.
#[derive(Clone, Debug, Default)]
pub struct XschemImport {
    /// Library directories converted wholesale, in search-path order.
    pub convert: Vec<String>,
    /// Search-path elements after them, contributing only what is referenced.
    pub roots: Vec<String>,
    /// `xschemrc` files: PDK path variables and `drc` procs.
    pub rcs: Vec<String>,
    /// Where the outputs go (`<out_dir>/<lib>/<cell>.<ext>`, `schemify.project`, reports).
    pub out_dir: String,
    /// Map xschem `devices/` symbols onto the `core` library (import spec §6).
    pub map_core: bool,
    /// Model-card scans for device limits (the command line reads them beforehand).
    pub models: Vec<crate::spice::ModelScan>,
    /// Manufacturing grid in picometres (from a tech file).
    pub grid_pm: Option<u32>,
    /// A Cadence `cdfDump`.
    pub cdf: Option<crate::cdf::CdfDump>,
}

/// What the driver must do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Read these paths and [`Job::supply`] each (bytes, or `None` when absent).
    Read(Vec<String>),
    /// List the files under these directories and [`Job::supply_list`] each (paths relative
    /// to the directory, or `None` when it cannot be listed).
    List(Vec<String>),
    /// Write these files; the job is finished.
    Done {
        /// (project-relative path, bytes), sorted by path.
        writes: Vec<(String, Vec<u8>)>,
        /// One line for the log or terminal.
        summary: String,
    },
    /// The conversion cannot proceed; nothing is written.
    Failed(Vec<String>),
}

/// Rounds of reads a job may ask for (each round is one level of the reference closure).
pub const ROUNDS_MAX: u32 = 4096;

/// A conversion in progress.
pub struct Job {
    command: Command,
    files: Files,
    absent: BTreeSet<String>,
    asked: BTreeSet<String>,
    /// Directories asked for by [`Step::List`] and not yet supplied.
    asked_lists: BTreeSet<String>,
    gather: Option<crate::gather::Gather>,
    /// Problems the output check found in the finished import (0 for other commands).
    problems: usize,
    rounds: u32,
}

impl Job {
    /// Starts `command`.
    pub fn new(command: Command) -> Job {
        Job {
            command,
            files: Files::new(),
            absent: BTreeSet::new(),
            asked: BTreeSet::new(),
            asked_lists: BTreeSet::new(),
            gather: None,
            problems: 0,
            rounds: 0,
        }
    }

    /// Hands in the result of one read asked for by [`Step::Read`].
    ///
    /// # Panics
    ///
    /// When `path` was not asked for (a driver bug).
    pub fn supply(&mut self, path: &str, bytes: Option<Vec<u8>>) {
        assert!(
            self.asked.remove(path),
            "supply of {path}, which was not asked for"
        );
        match bytes {
            Some(b) => {
                self.files.insert(path.to_owned(), b);
            }
            None => {
                self.absent.insert(path.to_owned());
            }
        }
    }

    /// Hands in the listing of one directory asked for by [`Step::List`].
    ///
    /// # Panics
    ///
    /// When `dir` was not asked for (a driver bug).
    pub fn supply_list(&mut self, dir: &str, files: Option<Vec<String>>) {
        assert!(
            self.asked_lists.remove(dir),
            "listing of {dir}, which was not asked for"
        );
        self.gather
            .as_mut()
            .expect("lists are asked by an import")
            .listed(dir, files);
    }

    /// Paths and directories asked for and not yet supplied.
    pub fn pending(&self) -> usize {
        self.asked.len() + self.asked_lists.len()
    }

    /// How many problems the output check found in a finished xschem import: a driver
    /// reports them as a failure (the outputs are written all the same).
    pub fn problems(&self) -> usize {
        self.problems
    }

    /// Advances the job. Call again after supplying every path of a [`Step::Read`].
    ///
    /// # Panics
    ///
    /// When reads are still pending.
    pub fn step(&mut self) -> Step {
        assert!(
            self.pending() == 0,
            "step with {} reads pending",
            self.pending()
        );
        self.rounds += 1;
        if self.rounds > ROUNDS_MAX {
            return Step::Failed(vec![format!("more than {ROUNDS_MAX} read rounds")]);
        }
        if let Command::XschemImport(x) = &self.command {
            let x = x.clone();
            return self.xschem_import(&x);
        }
        let command = self.command.clone();
        match command {
            Command::XschemImport(_) => unreachable!("handled above"),
            Command::InstallSkill { dir } => {
                let dir = dir.trim_end_matches('/');
                let join = |f: &str| {
                    if dir.is_empty() {
                        f.to_owned()
                    } else {
                        format!("{dir}/{f}")
                    }
                };
                Step::Done {
                    writes: vec![
                        (
                            join("schemify.il"),
                            crate::skill::schemify_il().into_bytes(),
                        ),
                        (
                            join("sfy_probe.il"),
                            crate::skill::PROBE.as_bytes().to_vec(),
                        ),
                    ],
                    summary: format!(
                        "wrote {} and {}; in Virtuoso: (load \"<absolute path>/schemify.il\")",
                        join("schemify.il"),
                        join("sfy_probe.il")
                    ),
                }
            }
            Command::VirtuosoExport { tops, out, opts } => {
                let map = match self.mapping() {
                    Ok(m) => m,
                    Err(s) => return s,
                };
                let refs: Vec<&str> = tops.iter().map(String::as_str).collect();
                let d = match load(&self.files, &self.absent, &refs) {
                    Ok(d) => d,
                    Err(LoadError::Missing(p)) => return self.ask(p),
                    Err(LoadError::Invalid(p)) => return Step::Failed(p),
                };
                let top_ix: Vec<usize> = (0..tops.len()).collect();
                match virtuoso::export(&d, &top_ix, &map, &opts) {
                    Ok(e) => {
                        // OCEAN scripts beside the .sfyx, under `ocean/` (§9.6).
                        let dir = out.rsplit_once('/').map_or("", |(d, _)| d);
                        let mut writes = vec![(out.clone(), e.sfyx.into_bytes())];
                        for (p, t) in e.ocean {
                            writes.push((join(dir, &format!("ocean/{p}")), t.into_bytes()));
                        }
                        finish(writes, &out, &e.notes, "Virtuoso export")
                    }
                    Err(p) => Step::Failed(p),
                }
            }
            Command::VirtuosoImport { sfyx, opts } => {
                let map = match self.mapping() {
                    Ok(m) => m,
                    Err(s) => return s,
                };
                let Some(text) = self.files.get(&sfyx) else {
                    if self.absent.contains(&sfyx) {
                        return Step::Failed(vec![format!("{sfyx}: cannot read")]);
                    }
                    return self.ask(vec![sfyx]);
                };
                let Ok(text) = String::from_utf8(text.clone()) else {
                    return Step::Failed(vec![format!("{sfyx}: not UTF-8")]);
                };
                let targets = match virtuoso::import_targets(&text) {
                    Ok(t) => t,
                    Err(p) => return Step::Failed(p),
                };
                let need: Vec<String> = targets.into_iter().filter(|p| !self.known(p)).collect();
                if !need.is_empty() {
                    return self.ask(need);
                }
                match virtuoso::import(&text, &self.files, &map, &opts) {
                    Ok(imp) => finish(imp.files, &sfyx, &imp.notes, "Virtuoso import"),
                    Err(p) => Step::Failed(p),
                }
            }
            Command::XschemExport { tops, out_dir } => {
                let refs: Vec<&str> = tops.iter().map(String::as_str).collect();
                let d = match load(&self.files, &self.absent, &refs) {
                    Ok(d) => d,
                    Err(LoadError::Missing(p)) => return self.ask(p),
                    Err(LoadError::Invalid(p)) => return Step::Failed(p),
                };
                let top_ix: Vec<usize> = (0..tops.len()).collect();
                match crate::xschem_out::export(&d, &top_ix) {
                    Ok(x) => {
                        let dir = out_dir.trim_end_matches('/');
                        let writes = x
                            .files
                            .into_iter()
                            .map(|(p, b)| (join(dir, &p), b))
                            .collect();
                        finish(writes, &join(dir, REPORT), &x.notes, "xschem export")
                    }
                    Err(p) => Step::Failed(p),
                }
            }
        }
    }

    /// One step of [`Command::XschemImport`]: list the search path, gather the inputs round
    /// by round, then translate them.
    fn xschem_import(&mut self, x: &XschemImport) -> Step {
        use crate::gather::Gather;
        if x.convert.is_empty() {
            return Step::Failed(vec!["no library directory to import".into()]);
        }
        let g = self
            .gather
            .get_or_insert_with(|| Gather::new(&x.convert, &x.roots));
        let unlisted = g.unlisted();
        if !unlisted.is_empty() {
            self.asked_lists.extend(unlisted.iter().cloned());
            return Step::List(unlisted);
        }
        let rcs: Vec<String> = x.rcs.iter().filter(|p| !self.known(p)).cloned().collect();
        if !rcs.is_empty() {
            return self.ask(rcs);
        }
        let g = self.gather.as_mut().expect("created above");
        let want = g.next(&self.files, &self.absent);
        if !want.is_empty() {
            return self.ask(want);
        }
        let g = self.gather.as_ref().expect("created above");
        let mut vars: Vec<(String, String)> = Vec::new();
        let mut procs = Vec::new();
        for rc in &x.rcs {
            let Some(text) = self.files.get(rc) else {
                return Step::Failed(vec![format!("{rc}: cannot read")]);
            };
            let text = String::from_utf8_lossy(text);
            for v in crate::tcl::pdk_vars(&text) {
                if !vars.iter().any(|(n, _)| *n == v.0) {
                    vars.push(v);
                }
            }
            procs.extend(crate::tcl::drc_procs(&text));
        }
        let inputs: Vec<(&str, &[u8])> = g
            .inputs()
            .filter_map(|p| Some((p.as_str(), self.files.get(p)?.as_slice())))
            .collect();
        if !inputs
            .iter()
            .any(|(p, _)| p.ends_with(".sym") || p.ends_with(".sch"))
        {
            return Step::Failed(vec![format!(
                "no .sym or .sch files under {}",
                x.convert.join(", ")
            )]);
        }
        let opts = crate::Options {
            roots: g.search_path(),
            pdk_vars: &vars,
            map_core: x.map_core,
            models: &x.models,
            drc: &procs,
            grid_pm: x.grid_pm,
            cdf: x.cdf.as_ref(),
        };
        let (outputs, report) = crate::translate_library(&inputs, &opts);
        let problems = crate::check_output(&outputs);
        self.problems = problems.len();
        let mut notes = Notes::default();
        report.to_notes(&mut notes);
        for p in &problems {
            notes.add("check", "-", p.clone());
        }
        // `.` is the project root itself: the import becomes the project.
        let dir = match x.out_dir.trim_end_matches('/') {
            "." => "",
            d => d,
        };
        let writes = outputs
            .into_iter()
            .map(|(p, b)| (join(dir, &p), b))
            .collect();
        let Step::Done {
            writes,
            mut summary,
        } = finish(writes, &join(dir, REPORT), &notes, "xschem import")
        else {
            unreachable!("finish is Done")
        };
        for p in problems.iter().take(60) {
            summary.push_str(&format!("\ncheck: {p}"));
        }
        summary.push_str(&format!("\ncheck: {} problems", problems.len()));
        Step::Done { writes, summary }
    }

    fn known(&self, p: &str) -> bool {
        self.files.contains_key(p) || self.absent.contains(p)
    }

    /// `interop.map`, read once (absent = identity mapping).
    fn mapping(&mut self) -> Result<Mapping, Step> {
        const MAP: &str = "interop.map";
        if !self.known(MAP) {
            return Err(self.ask(vec![MAP.to_owned()]));
        }
        match self.files.get(MAP) {
            None => Ok(Mapping::default()),
            Some(b) => match std::str::from_utf8(b) {
                Ok(t) => Mapping::parse(t).map_err(|p| {
                    Step::Failed(p.into_iter().map(|e| format!("{MAP}: {e}")).collect())
                }),
                Err(_) => Err(Step::Failed(vec![format!("{MAP}: not UTF-8")])),
            },
        }
    }

    fn ask(&mut self, paths: Vec<String>) -> Step {
        let fresh: Vec<String> = paths.into_iter().filter(|p| !self.known(p)).collect();
        assert!(
            !fresh.is_empty(),
            "asked for nothing new: the job would loop"
        );
        self.asked.extend(fresh.iter().cloned());
        Step::Read(fresh)
    }
}

/// Stem of the report a conversion writes inside a directory output (spec `interop.md` §12).
const REPORT: &str = "interop";

/// `rel` under `dir` (`""`: the project root).
fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_owned()
    } else {
        format!("{dir}/{rel}")
    }
}

/// Adds the report beside `base` (`amp.sfyx` → `amp.report.md`, `out/interop` →
/// `out/interop.report.md`; spec `interop.md` §12) and builds the summary.
fn finish(mut writes: Vec<(String, Vec<u8>)>, base: &str, notes: &Notes, what: &str) -> Step {
    let base = base.strip_suffix(".sfyx").unwrap_or(base);
    let n = writes.len();
    writes.push((
        format!("{base}.report.md"),
        notes.markdown(what).into_bytes(),
    ));
    writes.push((format!("{base}.report.txt"), notes.text().into_bytes()));
    writes.sort_by(|a, b| a.0.cmp(&b.0));
    let summary = format!(
        "{what}: {n} files, {} report rows ({base}.report.md)",
        notes.rows.len()
    );
    Step::Done { writes, summary }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_skill_writes_both_files() {
        let mut j = Job::new(Command::InstallSkill {
            dir: "interop".into(),
        });
        match j.step() {
            Step::Done { writes, .. } => {
                let names: Vec<&str> = writes.iter().map(|w| w.0.as_str()).collect();
                assert_eq!(names, ["interop/schemify.il", "interop/sfy_probe.il"]);
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn export_asks_for_the_map_and_the_closure() {
        let mut j = Job::new(Command::XschemExport {
            tops: vec!["a.shcomp".into()],
            out_dir: "out".into(),
        });
        let Step::Read(p) = j.step() else { panic!() };
        assert_eq!(p, ["a.shcomp", "schemify.project"]);
        for x in p {
            j.supply(&x, None);
        }
        assert!(matches!(j.step(), Step::Failed(_)));
    }
}
