//! Guest glue (`wasm32-unknown-unknown` only): File menu items, a path panel and the driver
//! that runs a [`Job`] through the host's inverted I/O (plugin ABI §5). Every decision is in
//! `job` and below, which the native tests exercise; this file only moves bytes.
//!
//! ```text
//! command (path args) → Job::new → step → Read(paths): fs_read each → io_done → supply …
//!                                        | List(dirs): fs_list each → io_done → supply_list …
//!                                        → step … → Done: fs_write each → io_done … → log summary
//! ```
//!
//! A new command abandons a job in flight (its late completions are ignored), so a trap or a
//! fuel abort mid-job never wedges the plugin.

use std::cell::RefCell;

use schemify_abi::guest::{self, Host};
use schemify_abi::{Event, Kind, Level, Out, text};

use crate::job::{Command, Job, Step};
use crate::virtuoso::{ExportOptions, ImportOptions};

guest::export!(on_event);

const UI: &str = "node file.interop_vexport item
  parent menu.file
  label \"Export to Virtuoso (.sfyx)…\"
  command interop.virtuoso_export
node file.interop_vimport item
  parent menu.file
  label \"Import from Virtuoso (.sfyx)…\"
  command interop.virtuoso_import
node file.interop_xexport item
  parent menu.file
  label \"Export to xschem…\"
  command interop.xschem_export
node file.interop_skill item
  parent menu.file
  label \"Write Virtuoso SKILL bridge (schemify.il)\"
  command interop.install_skill
";

const COMMANDS: &str = "command interop.virtuoso_export \"Export cells to Virtuoso (.sfyx)\"
command interop.virtuoso_import \"Import a Virtuoso dump (.sfyx)\"
command interop.xschem_export \"Export cells to xschem\"
command interop.install_skill \"Write schemify.il for Virtuoso\"
command interop.xschem_import \"Import xschem libraries\"
command interop.close \"Close the interop panel\"
";

/// I/O requests kept in flight (the host queue holds 256 per frame).
const WINDOW: usize = 64;

struct Running {
    job: Job,
    /// (handle, path) of reads in flight.
    reads: Vec<(u32, String)>,
    /// (handle, directory) of listings in flight.
    lists: Vec<(u32, String)>,
    /// Listings not issued yet.
    list_queue: Vec<String>,
    /// Reads not issued yet.
    queue: Vec<String>,
    /// Writes not issued yet, then the summary once all are done.
    writes: Vec<(String, Vec<u8>)>,
    writing: Vec<(u32, String)>,
    failed: Vec<String>,
    summary: String,
}

thread_local! {
    static RUN: RefCell<Option<Running>> = const { RefCell::new(None) };
}

fn contribute(out: &mut Out, panel: Option<&str>) {
    let mut ui = UI.to_owned();
    if let Some(cmd) = panel {
        ui.push_str(&format!(
            "node interop.panel panel\n  parent window\nnode interop.prompt label\n  parent interop.panel\n  label \"{}\"\nnode interop.path text_input\n  parent interop.panel\n  command {cmd}\nnode interop.close button\n  parent interop.panel\n  label Close\n  command interop.close\n",
            prompt(cmd)
        ));
    }
    out.contribute(Kind::Ui, &ui);
    out.contribute(Kind::Commands, COMMANDS);
}

fn prompt(cmd: &str) -> &'static str {
    match cmd {
        "interop.virtuoso_export" => {
            "Cell file(s) to export, then the .sfyx path (e.g. lib/amp.shcomp out/amp.sfyx); flag --core-cells. Enter to run:"
        }
        "interop.virtuoso_import" => "The .sfyx dump to import (project-relative). Enter to run:",
        "interop.xschem_import" => {
            "xschem library dirs, then -o and the output dir (e.g. xschem --root ${PDK_ROOT}/sky130A/libs.tech/xschem -o lib). Enter to run:"
        }
        "interop.xschem_export" => {
            "Cell file(s) to export, then the output directory. Enter to run:"
        }
        _ => "Output directory for schemify.il (Enter for interop/):",
    }
}

fn on_event(ev: Event<'_>, host: &Host, out: &mut Out) {
    match ev {
        Event::Load => contribute(out, None),
        Event::Command {
            name: "interop.close",
            ..
        } => contribute(out, None),
        Event::Command { name, args } if name.starts_with("interop.") => {
            let mut words = Vec::new();
            let mut values = text::values(args);
            while let Ok(v) = values.need() {
                words.extend(v.split_whitespace().map(str::to_owned));
            }
            let Some(cmd) = command(name, &words) else {
                contribute(out, Some(name));
                return;
            };
            contribute(out, None);
            let mut run = Running {
                job: Job::new(cmd),
                reads: Vec::new(),
                lists: Vec::new(),
                list_queue: Vec::new(),
                queue: Vec::new(),
                writes: Vec::new(),
                writing: Vec::new(),
                failed: Vec::new(),
                summary: String::new(),
            };
            advance(&mut run, host, out);
            if !over(&run, out) {
                RUN.set(Some(run));
            }
        }
        Event::IoDone {
            handle,
            status,
            bytes,
        } => {
            let Some(mut run) = RUN.take() else { return };
            if let Some(k) = run.reads.iter().position(|r| r.0 == handle) {
                let (_, path) = run.reads.swap_remove(k);
                run.job.supply(&path, (status >= 0).then(|| bytes.to_vec()));
                pump_reads(&mut run, host, out);
                if waiting(&run) == 0 {
                    advance(&mut run, host, out);
                }
            } else if let Some(k) = run.lists.iter().position(|l| l.0 == handle) {
                let (_, dir) = run.lists.swap_remove(k);
                let listing = (status >= 0).then(|| {
                    String::from_utf8_lossy(bytes)
                        .lines()
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned)
                        .collect()
                });
                run.job.supply_list(&dir, listing);
                pump_lists(&mut run, host);
                if waiting(&run) == 0 {
                    advance(&mut run, host, out);
                }
            } else if let Some(k) = run.writing.iter().position(|w| w.0 == handle) {
                let (_, path) = run.writing.swap_remove(k);
                if status < 0 {
                    run.failed.push(path);
                }
                pump_writes(&mut run, host, out);
            }
            if !over(&run, out) {
                RUN.set(Some(run));
            }
        }
        Event::Rejected { code, record } => {
            out.log_at(
                Level::Warn,
                &format!(
                    "interop: output rejected ({code}): {}",
                    String::from_utf8_lossy(record)
                ),
            );
        }
        _ => {}
    }
}

/// Parses command arguments; `None` shows the path panel.
fn command(name: &str, w: &[String]) -> Option<Command> {
    let flags: Vec<&str> = w
        .iter()
        .filter(|x| x.starts_with("--"))
        .map(String::as_str)
        .collect();
    let paths: Vec<String> = w.iter().filter(|x| !x.starts_with("--")).cloned().collect();
    match name {
        "interop.virtuoso_export" if !paths.is_empty() => {
            let (tops, out) = match paths.last() {
                Some(l) if l.ends_with(".sfyx") && paths.len() > 1 => {
                    (paths[..paths.len() - 1].to_vec(), l.clone())
                }
                _ => {
                    let stem = paths[0]
                        .rsplit_once('.')
                        .map_or(paths[0].as_str(), |(s, _)| s);
                    (paths.clone(), format!("{stem}.sfyx"))
                }
            };
            let opts = ExportOptions {
                // analogLib masters unless `--core-cells` (§6.5), as the command line.
                analoglib: !flags.contains(&"--core-cells"),
                producer: "schemify interop plugin".into(),
                ..ExportOptions::default()
            };
            Some(Command::VirtuosoExport { tops, out, opts })
        }
        "interop.virtuoso_import" if paths.len() == 1 => Some(Command::VirtuosoImport {
            sfyx: paths[0].clone(),
            opts: ImportOptions::default(),
        }),
        "interop.xschem_export" if !paths.is_empty() => {
            let (tops, out_dir) = if paths.len() > 1 && !paths.last()?.contains(".sh") {
                (paths[..paths.len() - 1].to_vec(), paths.last()?.clone())
            } else {
                (paths.clone(), "xschem_export".to_owned())
            };
            Some(Command::XschemExport { tops, out_dir })
        }
        "interop.xschem_import" => xschem_import(w),
        "interop.install_skill" => Some(Command::InstallSkill {
            dir: paths.first().cloned().unwrap_or_else(|| "interop".into()),
        }),
        _ => None,
    }
}

/// `interop.xschem_import <lib-dir>... [--root <dir>]... [--xschemrc <file>]... [--no-core]
/// -o <out-dir>`, the command line's `xschem-import` with plugin paths: project-relative, or
/// `${VAR}/…` under a variable the user granted (`read:<VAR>`). `None` when malformed.
fn xschem_import(w: &[String]) -> Option<Command> {
    let mut x = crate::job::XschemImport {
        map_core: true,
        ..crate::job::XschemImport::default()
    };
    let mut it = w.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => x.out_dir = it.next()?.clone(),
            "--root" => x.roots.push(it.next()?.clone()),
            "--xschemrc" => x.rcs.push(it.next()?.clone()),
            "--no-core" => x.map_core = false,
            s if s.starts_with('-') => return None,
            _ => x.convert.push(a.clone()),
        }
    }
    (!x.convert.is_empty() && !x.out_dir.is_empty()).then(|| Command::XschemImport(Box::new(x)))
}

/// Reads and listings in flight or queued.
fn waiting(run: &Running) -> usize {
    run.reads.len() + run.queue.len() + run.lists.len() + run.list_queue.len()
}

/// Steps the job until it waits for reads in flight, writes, or ends. Bounded by the job's
/// own round limit.
fn advance(run: &mut Running, host: &Host, out: &mut Out) {
    loop {
        match run.job.step() {
            Step::Read(paths) => {
                run.queue = paths;
                pump_reads(run, host, out);
                if waiting(run) > 0 {
                    return; // wait for io_done
                }
            }
            Step::List(dirs) => {
                run.list_queue = dirs;
                pump_lists(run, host);
                if waiting(run) > 0 {
                    return; // wait for io_done
                }
            }
            Step::Done { writes, summary } => {
                run.writes = writes;
                run.summary = summary;
                pump_writes(run, host, out);
                return;
            }
            Step::Failed(problems) => {
                for p in problems.iter().take(50) {
                    out.log_at(Level::Error, &format!("interop: {p}"));
                }
                run.summary = String::from("\u{0}failed");
                run.writes.clear();
                return;
            }
        }
    }
}

/// True (and the summary logged) when the job has nothing left in flight.
fn over(run: &Running, out: &mut Out) -> bool {
    let idle = waiting(run) == 0 && run.writes.is_empty() && run.writing.is_empty();
    if !idle || run.summary.is_empty() {
        return false;
    }
    if run.summary != "\u{0}failed" {
        finish(run, out);
    }
    true
}

fn pump_reads(run: &mut Running, host: &Host, out: &mut Out) {
    while run.reads.len() < WINDOW {
        let Some(path) = run.queue.pop() else { break };
        match host.fs_read(&path) {
            Ok(h) => run.reads.push((h, path)),
            Err(_) => run.job.supply(&path, None),
        }
    }
    let _ = out;
}

fn pump_lists(run: &mut Running, host: &Host) {
    while run.lists.len() < WINDOW {
        let Some(dir) = run.list_queue.pop() else {
            break;
        };
        match host.fs_list(&dir) {
            Ok(h) => run.lists.push((h, dir)),
            Err(_) => run.job.supply_list(&dir, None),
        }
    }
}

fn pump_writes(run: &mut Running, host: &Host, out: &mut Out) {
    while run.writing.len() < WINDOW {
        let Some((path, data)) = run.writes.pop() else {
            break;
        };
        match host.fs_write(&path, &data) {
            Ok(h) => run.writing.push((h, path)),
            Err(code) => {
                out.log_at(
                    Level::Error,
                    &format!("interop: cannot write {path} ({code})"),
                );
                run.failed.push(path);
            }
        }
    }
}

fn finish(run: &Running, out: &mut Out) {
    if run.failed.is_empty() {
        let lines: Vec<&str> = run.summary.lines().collect();
        let (last, rest) = lines.split_last().unwrap_or((&"", &[]));
        for l in rest {
            out.log_at(Level::Info, l);
        }
        // The import's output check: problems fail the command (the files are written).
        let level = if run.job.problems() > 0 {
            Level::Error
        } else {
            Level::Info
        };
        out.log_at(level, last);
    } else {
        out.log_at(
            Level::Error,
            &format!(
                "interop: {} (failed to write: {})",
                run.summary,
                run.failed.join(", ")
            ),
        );
    }
}
