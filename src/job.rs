//! One conversion as a pure state machine (spec `interop.md` §1): the command line answers
//! its reads from the file system, the wasm guest through the host's inverted I/O, and both
//! get identical results because neither holds any logic of its own.
//!
//! ```text
//! Job::new(command) → loop { step() → Read(paths) | List(dirs) → supply(path, bytes|None)…
//!                                   | Done { writes, summary } | Failed(problems) }
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::cdf::CdfDump;
use crate::design::{Files, LoadError, load};
use crate::map::Mapping;
use crate::notes::Notes;
use crate::report::Report;
use crate::spice::{self, ModelScan};
use crate::virtuoso::{self, ExportOptions, ImportOptions};
use crate::{Options, tcl, translate_library};

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
    /// xschem libraries → Schemify libraries under `out_dir` (spec `import.md`).
    XschemImport(Box<XschemImport>),
}

/// The inputs of an xschem import. Paths are as the driver reads them: absolute on the
/// command line, project-relative or `${NAME}/…` in the guest.
#[derive(Clone, Debug)]
pub struct XschemImport {
    /// The xschem library search path in order: (directory, convert every `.sym`/`.sch`
    /// under it). A `false` element (`--root`) is only a prefix references resolve against.
    pub dirs: Vec<(String, bool)>,
    /// `xschemrc` files: PDK path variables and `drc` procs.
    pub rcs: Vec<String>,
    /// Model-card entry files, each with its `.lib` section.
    pub models: Vec<(String, Option<String>)>,
    /// Where relative model includes are also looked up (ngspice `sourcepath`).
    pub search_dirs: Vec<PathBuf>,
    /// Manufacturing grid in picometres (a tech file the command line read).
    pub grid_pm: Option<u32>,
    /// A Cadence `cdfDump` the command line read.
    pub cdf: Option<CdfDump>,
    /// Map xschem `devices/*.sym` to `core` (`--no-core` clears it).
    pub core: bool,
    /// Output directory.
    pub out_dir: String,
}

/// What an xschem import produced beyond its writes, for the command line's summary.
pub struct XschemImported {
    /// The import report.
    pub report: Report,
    /// `drc` procs recognised in the `xschemrc` files.
    pub drc_procs: usize,
    /// One scan per [`XschemImport::models`] entry.
    pub scans: Vec<ModelScan>,
    /// Files under a converted directory that could not be read (left out).
    pub unread: Vec<String>,
}

/// Splits a `--models` value `<file>[:<section>]`: the part after the last `:` is a section
/// unless it is empty or holds a `/`.
pub fn models_arg(v: &str) -> (String, Option<String>) {
    match v.rsplit_once(':') {
        Some((p, s)) if !s.contains('/') && !s.is_empty() => (p.to_owned(), Some(s.to_owned())),
        _ => (v.to_owned(), None),
    }
}

/// What the driver must do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Read these paths and [`Job::supply`] each (bytes, or `None` when absent).
    Read(Vec<String>),
    /// List these directories and [`Job::supply`] each listing as the host's `fs_list`
    /// answers it (one path relative to the directory per line, recursive, each directory's
    /// entries by name, hidden entries and `target` left out), or `None` when absent.
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
    rounds: u32,
    imported: Option<XschemImported>,
}

impl Job {
    /// Starts `command`.
    pub fn new(command: Command) -> Job {
        Job {
            command,
            files: Files::new(),
            absent: BTreeSet::new(),
            asked: BTreeSet::new(),
            rounds: 0,
            imported: None,
        }
    }

    /// The finished xschem import's report and check, once [`Step::Done`].
    pub fn imported(&self) -> Option<&XschemImported> {
        self.imported.as_ref()
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

    /// Paths asked for and not yet supplied.
    pub fn pending(&self) -> usize {
        self.asked.len()
    }

    /// Advances the job. Call again after supplying every path of a [`Step::Read`].
    ///
    /// # Panics
    ///
    /// When reads are still pending.
    pub fn step(&mut self) -> Step {
        assert!(
            self.asked.is_empty(),
            "step with {} reads pending",
            self.asked.len()
        );
        self.rounds += 1;
        if self.rounds > ROUNDS_MAX {
            return Step::Failed(vec![format!("more than {ROUNDS_MAX} read rounds")]);
        }
        let command = self.command.clone();
        match command {
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
                            let path = if dir.is_empty() {
                                format!("ocean/{p}")
                            } else {
                                format!("{dir}/ocean/{p}")
                            };
                            writes.push((path, t.into_bytes()));
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
                    Ok(imp) => {
                        let mut writes = imp.files;
                        let base = sfyx.strip_suffix(".sfyx").unwrap_or(&sfyx).to_owned();
                        writes.push((
                            format!("{base}.report.md"),
                            imp.notes.markdown("Virtuoso import").into_bytes(),
                        ));
                        writes.push((format!("{base}.report.txt"), imp.notes.text().into_bytes()));
                        writes.sort_by(|a, b| a.0.cmp(&b.0));
                        let n = writes.len() - 2;
                        Step::Done {
                            writes,
                            summary: format!(
                                "Virtuoso import: {n} files, {} report rows ({base}.report.md)",
                                imp.notes.rows.len()
                            ),
                        }
                    }
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
                            .map(|(p, b)| {
                                (
                                    if dir.is_empty() {
                                        p
                                    } else {
                                        format!("{dir}/{p}")
                                    },
                                    b,
                                )
                            })
                            .collect();
                        finish(
                            writes,
                            &format!("{dir}/xschem-export"),
                            &x.notes,
                            "xschem export",
                        )
                    }
                    Err(p) => Step::Failed(p),
                }
            }
            Command::XschemImport(x) => self.xschem_import(&x),
        }
    }

    /// Lists the converted directories, reads their `.sym`/`.sch`/`.cir` files, the
    /// `xschemrc`s and the model cards with their includes (one round per include level),
    /// then translates. The import oracle ([`crate::check_output`]) is the command line's:
    /// the guest leaves it out of the module (160 KB).
    fn xschem_import(&mut self, x: &XschemImport) -> Step {
        let converted: Vec<&str> = x
            .dirs
            .iter()
            .filter(|d| d.1)
            .map(|d| d.0.as_str())
            .collect();
        let need: Vec<String> = converted
            .iter()
            .filter(|d| !self.known(d))
            .map(|d| d.to_string())
            .collect();
        if !need.is_empty() {
            self.asked.extend(need.iter().cloned());
            return Step::List(need);
        }
        let mut sources: Vec<String> = Vec::new();
        let mut seen = BTreeSet::new();
        for d in &converted {
            let Some(list) = self.files.get(*d) else {
                return Step::Failed(vec![format!("{d}: cannot open")]);
            };
            for rel in String::from_utf8_lossy(list).lines() {
                let p = format!("{d}/{rel}");
                if [".sym", ".sch", ".cir"].iter().any(|e| rel.ends_with(e))
                    && seen.insert(p.clone())
                {
                    sources.push(p);
                }
            }
        }
        let need: Vec<String> = sources.iter().chain(&x.rcs).cloned().collect();
        if need.iter().any(|p| !self.known(p)) {
            return self.ask(need);
        }
        let mut vars: Vec<(String, String)> = Vec::new();
        let mut procs = Vec::new();
        for rc in &x.rcs {
            let Some(text) = self.files.get(rc).and_then(|b| std::str::from_utf8(b).ok()) else {
                return Step::Failed(vec![format!("{rc}: cannot read")]);
            };
            for v in tcl::pdk_vars(text) {
                if !vars.iter().any(|(n, _)| *n == v.0) {
                    vars.push(v);
                }
            }
            procs.extend(tcl::drc_procs(text));
        }
        // A model card's includes are known only once it is read: scan with what is here,
        // ask for every include not yet known, and scan again.
        let mut missing = BTreeSet::new();
        let scans: Vec<ModelScan> = x
            .models
            .iter()
            .map(|(p, section)| {
                let opts = spice::ScanOptions {
                    section: section.as_deref(),
                    search_dirs: &x.search_dirs,
                };
                spice::scan(Path::new(p), &opts, |q| {
                    let k = q.to_string_lossy().replace('\\', "/");
                    match self.files.get(&k) {
                        Some(b) => String::from_utf8(b.clone()).ok(),
                        None if self.absent.contains(&k) => None,
                        None => {
                            missing.insert(k);
                            None
                        }
                    }
                })
            })
            .collect();
        if !missing.is_empty() {
            return self.ask(missing.into_iter().collect());
        }
        let roots: Vec<String> = x.dirs.iter().map(|d| d.0.clone()).collect();
        let opts = Options {
            roots: &roots,
            pdk_vars: &vars,
            map_core: x.core,
            models: &scans,
            drc: &procs,
            grid_pm: x.grid_pm,
            cdf: x.cdf.as_ref(),
        };
        let unread: Vec<String> = sources
            .iter()
            .filter(|p| self.absent.contains(*p))
            .cloned()
            .collect();
        let inputs: Vec<(&str, &[u8])> = sources
            .iter()
            .filter_map(|p| Some((p.as_str(), self.files.get(p)?.as_slice())))
            .collect();
        let (mut outputs, mut report) = translate_library(&inputs, &opts);
        // The rcs' netlister settings are not applied: one row each, in the written report.
        let mut settings = false;
        for rc in &x.rcs {
            let text = String::from_utf8_lossy(&self.files[rc]);
            for (line, set) in tcl::netlist_settings(&text) {
                if !report.files.contains(rc) {
                    report.files.push(rc.clone());
                    report.records_in.push(0);
                    report.records_out.push(0);
                }
                let file = report.files.iter().position(|f| f == rc).expect("pushed") as u32;
                report.items.push(crate::report::Item {
                    file,
                    line,
                    class: crate::report::Class::Review(crate::template::Reason::Unsupported),
                    records: 0,
                    detail: format!("{set}: an xschem netlister setting the import does not apply"),
                });
                settings = true;
            }
        }
        if settings {
            for (p, b) in &mut outputs {
                match p.as_str() {
                    "import-report.md" => *b = report.markdown().into_bytes(),
                    "import-report.txt" => *b = report.text().into_bytes(),
                    _ => {}
                }
            }
        }
        let dir = x.out_dir.trim_end_matches('/');
        let join = |p: &str| {
            if dir.is_empty() {
                p.to_owned()
            } else {
                format!("{dir}/{p}")
            }
        };
        let mut summary = format!(
            "xschem import: {} files, {} report rows ({})",
            outputs.len() - 2,
            report.items.len(),
            join("import-report.md")
        );
        if !unread.is_empty() {
            summary.push_str(&format!("; unreadable, left out: {}", unread.join(", ")));
        }
        let writes = outputs.into_iter().map(|(p, b)| (join(&p), b)).collect();
        self.imported = Some(XschemImported {
            report,
            drc_procs: procs.len(),
            scans,
            unread,
        });
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

/// Adds the report files beside `base` and builds the summary.
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

    /// The import lists the converted directories, reads only their xschem files (and the
    /// model includes, a round per level), and fails on a directory that is not there.
    #[test]
    fn xschem_import_lists_then_reads() {
        let x = |dirs: &[&str]| {
            Job::new(Command::XschemImport(Box::new(XschemImport {
                dirs: dirs.iter().map(|d| (d.to_string(), true)).collect(),
                rcs: Vec::new(),
                models: vec![("m/top.lib".into(), None)],
                search_dirs: Vec::new(),
                grid_pm: None,
                cdf: None,
                core: true,
                out_dir: "imp".into(),
            })))
        };
        let mut j = x(&["lib"]);
        assert_eq!(j.step(), Step::List(vec!["lib".into()]));
        j.supply("lib", Some(b"a.sym\nnotes.txt\n".to_vec()));
        assert_eq!(j.step(), Step::Read(vec!["lib/a.sym".into()]));
        j.supply(
            "lib/a.sym",
            Some(b"v {xschem version=3.4.4 file_version=1.2}\n".to_vec()),
        );
        assert_eq!(j.step(), Step::Read(vec!["m/top.lib".into()]));
        j.supply("m/top.lib", Some(b".include inc.lib\n".to_vec()));
        assert_eq!(j.step(), Step::Read(vec!["m/inc.lib".into()]));
        j.supply("m/inc.lib", None);
        let Step::Done { writes, .. } = j.step() else {
            panic!()
        };
        assert!(writes.iter().any(|w| w.0 == "imp/schemify.project"));
        assert!(j.imported().is_some());
        // An xschemrc netlister setting is a report row (inv_xschem row 18).
        let mut j = x(&["lib"]);
        if let Command::XschemImport(c) = &mut j.command {
            c.rcs.push("rc".into());
            c.models.clear();
        }
        j.step();
        j.supply("lib", Some(b"a.sym\n".to_vec()));
        j.step();
        j.supply(
            "lib/a.sym",
            Some(b"v {xschem version=3.4.4 file_version=1.2}\n".to_vec()),
        );
        j.supply("rc", Some(b"set PDK sky130A\nset top_subckt 1\n".to_vec()));
        let Step::Done { writes, .. } = j.step() else {
            panic!()
        };
        let txt = &writes
            .iter()
            .find(|w| w.0 == "imp/import-report.txt")
            .unwrap()
            .1;
        assert!(
            String::from_utf8_lossy(txt).contains(
                "needs_review:unsupported\trc\t2\t0\tset top_subckt 1: an xschem netlister setting"
            ),
            "{}",
            String::from_utf8_lossy(txt)
        );
        let mut j = x(&["gone"]);
        assert!(matches!(j.step(), Step::List(_)));
        j.supply("gone", None);
        assert_eq!(j.step(), Step::Failed(vec!["gone: cannot open".into()]));
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
