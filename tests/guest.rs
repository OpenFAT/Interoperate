//! The built guest (`interop.wasm`, refreshed by `build.sh`) end to
//! end through the real host (`schemify_plugin`, wasmi, default fuel): the File menu and
//! commands compose without faults; a command without arguments opens the path panel; the
//! Virtuoso export, xschem export and SKILL install read through inverted I/O and write
//! byte-identically to the native job (`interop::job`, what the CLI runs); the xschem import
//! writes what `schemify-interop xschem-import` writes, and refuses a path no grant covers
//! without reading it. Skips when the module has not been built.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use interop::job::{Command, Job, Step};
use interop::virtuoso::ExportOptions;
use schemify_plugin::abi::{Code, Event, FUEL_DEFAULT, Kind};
use schemify_plugin::{IoOp, PluginEvents, PluginId, Plugins, RequestQueues, Snapshots, deliver};

/// The capabilities the guest runs with here, and what `schemify.project` requests: the
/// `read:<NAME>` roots are where `interop.xschem_import` reads designs, xschem's libraries
/// and a PDK from outside the project (README "xschem import in the editor").
const GRANTS: [&str; 5] = [
    "read_project",
    "write_project",
    "read:ANALOG_ROOT",
    "read:XSCHEM_LIBRARY",
    "read:PDK_ROOT",
];

struct World {
    plugins: Plugins,
    events: PluginEvents,
    snap: Snapshots,
    queues: RequestQueues,
    id: PluginId,
    /// Log lines of the last [`World::drive`].
    logs: Vec<String>,
    /// Paths of the I/O requests the host queued in the last [`World::drive`].
    requested: Vec<String>,
}

impl World {
    fn new() -> Option<World> {
        let wasm =
            std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("interop.wasm")).ok()?;
        let mut plugins = Plugins::new();
        let id = plugins
            .load("interop", &wasm, &GRANTS, FUEL_DEFAULT)
            .expect("loads");
        let mut w = World {
            plugins,
            events: PluginEvents::new(),
            snap: Snapshots::new(),
            queues: RequestQueues::default(),
            id,
            logs: Vec::new(),
            requested: Vec::new(),
        };
        w.frame();
        Some(w)
    }

    fn frame(&mut self) {
        deliver(
            &mut self.plugins,
            &self.events,
            &mut self.snap,
            &mut self.queues,
        );
        self.events.clear();
    }

    /// Runs `command` with `args`, serving reads from `files`; returns the writes. Asserts
    /// that the guest faulted nowhere.
    fn run(
        &mut self,
        command: &str,
        args: &str,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> BTreeMap<String, Vec<u8>> {
        let written = self.drive(command, args, files);
        let faults: Vec<String> = self
            .plugins
            .health()
            .faults()
            .iter()
            .map(|f| format!("{f:?}"))
            .collect();
        assert!(faults.is_empty(), "{faults:?}");
        written
    }

    /// [`World::run`] without the fault check. A listing answers every file of `files`
    /// under the directory, as the app's `fs_list` does.
    fn drive(
        &mut self,
        command: &str,
        args: &str,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> BTreeMap<String, Vec<u8>> {
        std::mem::take(&mut self.queues);
        self.logs.clear();
        self.requested.clear();
        self.events
            .push(
                Some(self.id),
                &Event::Command {
                    name: command,
                    args,
                },
            )
            .expect("queued");
        self.frame();
        let mut written = BTreeMap::new();
        for _ in 0..10_000 {
            let q = std::mem::take(&mut self.queues);
            self.logs.extend(q.logs.iter().map(|l| l.text.clone()));
            if q.io.is_empty() {
                break;
            }
            for r in &q.io {
                let path = r.args[0].clone();
                self.requested.push(path.clone());
                let (status, bytes): (i32, Vec<u8>) = match r.op {
                    IoOp::FsRead => match files.get(&path) {
                        Some(b) => (0, b.clone()),
                        None => (Code::Gone.neg(), Vec::new()),
                    },
                    IoOp::FsList => {
                        let under = format!("{path}/");
                        let listing: String = files
                            .keys()
                            .filter_map(|k| Some(format!("{}\n", k.strip_prefix(&under)?)))
                            .collect();
                        if listing.is_empty() {
                            (Code::Gone.neg(), Vec::new())
                        } else {
                            (0, listing.into_bytes())
                        }
                    }
                    IoOp::FsWrite => {
                        written.insert(path, r.data.clone());
                        (0, Vec::new())
                    }
                    op => panic!("unexpected {op:?}"),
                };
                self.events
                    .push(
                        Some(self.id),
                        &Event::IoDone {
                            handle: r.handle,
                            status,
                            bytes: &bytes,
                        },
                    )
                    .expect("queued");
            }
            self.frame();
        }
        written
    }
}

fn fixture(rel: &str) -> BTreeMap<String, Vec<u8>> {
    tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/schemify")
            .join(rel),
        "",
    )
}

/// Every file under `dir`, keyed `<prefix><path relative to dir>`.
fn tree(dir: &Path, prefix: &str) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, d: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut e: Vec<PathBuf> = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        e.sort();
        for p in e {
            if p.is_dir() {
                walk(base, &p, prefix, out);
            } else {
                let r = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(format!("{prefix}{r}"), std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, prefix, &mut out);
    out
}

/// The native job over the same files: what the CLI writes.
fn native(cmd: Command, files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    let mut j = Job::new(cmd);
    loop {
        match j.step() {
            Step::Read(ps) => {
                for p in ps {
                    let b = files.get(&p).cloned();
                    j.supply(&p, b);
                }
            }
            Step::Done { writes, .. } => return writes.into_iter().collect(),
            Step::List(p) | Step::Failed(p) => panic!("{p:?}"),
        }
    }
}

#[test]
fn contributes_the_file_menu_and_opens_the_path_panel() {
    let Some(mut w) = World::new() else {
        eprintln!("interop.wasm not built (./build.sh); skipped");
        return;
    };
    let c = w.plugins.composed();
    for cmd in [
        "interop.virtuoso_export",
        "interop.virtuoso_import",
        "interop.xschem_export",
        "interop.xschem_import",
        "interop.install_skill",
    ] {
        assert!(
            c.of(Kind::Ui).iter().any(|r| r.text.contains(cmd)),
            "{cmd} in the menu"
        );
        assert!(
            c.of(Kind::Commands).iter().any(|r| r.text.contains(cmd)),
            "{cmd} registered"
        );
    }
    w.events
        .push(
            Some(w.id),
            &Event::Command {
                name: "interop.virtuoso_export",
                args: "",
            },
        )
        .unwrap();
    w.frame();
    assert!(
        w.plugins
            .composed()
            .of(Kind::Ui)
            .iter()
            .any(|r| r.text.contains("text_input"))
    );
}

#[test]
fn guest_conversions_equal_the_native_job_byte_for_byte() {
    let Some(mut w) = World::new() else {
        eprintln!("interop.wasm not built (./build.sh); skipped");
        return;
    };
    let files = fixture("crates/app/tests/fixtures/inv");
    let got = w.run("interop.virtuoso_export", "top.shcomp out/inv.sfyx", &files);
    let want = native(
        Command::VirtuosoExport {
            tops: vec!["top.shcomp".into()],
            out: "out/inv.sfyx".into(),
            opts: ExportOptions {
                producer: "schemify interop plugin".into(),
                ..ExportOptions::default()
            },
        },
        &files,
    );
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>()
    );
    assert_eq!(got, want, "Virtuoso export: wasm ≠ native");
    let got = w.run("interop.xschem_export", "top.shcomp xs", &files);
    let want = native(
        Command::XschemExport {
            tops: vec!["top.shcomp".into()],
            out_dir: "xs".into(),
        },
        &files,
    );
    assert_eq!(got, want, "xschem export: wasm ≠ native");
    let got = w.run("interop.install_skill", "il", &files);
    assert_eq!(
        got["il/schemify.il"],
        interop::skill::schemify_il().into_bytes()
    );
    // A Virtuoso import of the export, into the same project.
    let mut with = files.clone();
    with.insert("out/inv.sfyx".into(), want_sfyx(&files));
    let got = w.run("interop.virtuoso_import", "out/inv.sfyx", &with);
    let want = native(
        Command::VirtuosoImport {
            sfyx: "out/inv.sfyx".into(),
            opts: Default::default(),
        },
        &with,
    );
    assert_eq!(got, want, "Virtuoso import: wasm ≠ native");
}

/// `interop.xschem_import` over `${XSCHEM_LIBRARY}` (the determinism library), a project
/// `xschemrc` and model cards under `${PDK_ROOT}` (read include by include) writes exactly
/// the files `schemify-interop xschem-import` writes for the same inputs.
#[cfg(feature = "cli")]
#[test]
fn guest_xschem_import_equals_the_command_line() {
    let Some(mut w) = World::new() else {
        eprintln!("interop.wasm not built (./build.sh); skipped");
        return;
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (lib, models) = (
        repo.join("determinism/xschem"),
        repo.join("tests/fixtures/pdk_scan/kit/models"),
    );
    let rc = "set PDK_ROOT /pdk\nproc drc_w {instance symbol model w l} {\n set res {}\n return $res\n}\n";
    let mut files = tree(&lib, "${XSCHEM_LIBRARY}/");
    files.extend(tree(&models, "${PDK_ROOT}/models/"));
    files.insert("xschemrc".into(), rc.as_bytes().to_vec());
    let got = w.run(
        "interop.xschem_import",
        "${XSCHEM_LIBRARY}/devices ${XSCHEM_LIBRARY}/lib --root ${XSCHEM_LIBRARY} \
         --xschemrc xschemrc --models ${PDK_ROOT}/models/top.lib:tt --out imp",
        &files,
    );
    let tmp = std::env::temp_dir().join(format!("interop-guest-xsimport-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("xschemrc"), rc).unwrap();
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let args = [
        "xschem-import".into(),
        s(lib.join("devices")),
        s(lib.join("lib")),
        "--root".into(),
        s(lib.clone()),
        "--xschemrc".into(),
        s(tmp.join("xschemrc")),
        "--models".into(),
        format!("{}:tt", s(models.join("top.lib"))),
        "-o".into(),
        s(tmp.join("out")),
    ];
    assert_eq!(interop::cli::run(&args), 0);
    let want = tree(&tmp.join("out"), "imp/");
    std::fs::remove_dir_all(&tmp).unwrap();
    assert!(
        want.contains_key("imp/schemify.project"),
        "{:?}",
        want.keys()
    );
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>()
    );
    assert_eq!(got, want, "xschem import: wasm ≠ command line");
    assert!(
        w.requested
            .contains(&"${PDK_ROOT}/models/mos.inc".to_owned()),
        "model includes are read: {:?}",
        w.requested
    );
    assert!(
        w.logs.iter().any(|l| l.starts_with("xschem import: ")),
        "{:?}",
        w.logs
    );
}

/// A path outside the project and every granted `read:<NAME>` root (`${HOME}` is not
/// granted, `/etc` is absolute) is refused by the host with a message naming the grant,
/// never read, and nothing is written.
#[test]
fn guest_xschem_import_refuses_ungranted_paths() {
    let Some(mut w) = World::new() else {
        eprintln!("interop.wasm not built (./build.sh); skipped");
        return;
    };
    let files = BTreeMap::from([("${HOME}/lib/a.sym".to_owned(), b"v {}\n".to_vec())]);
    for dir in ["${HOME}/lib", "/etc"] {
        let got = w.drive("interop.xschem_import", &format!("{dir} --out imp"), &files);
        assert!(got.is_empty(), "{got:?}");
        assert!(w.requested.is_empty(), "never read: {:?}", w.requested);
        let refused = format!("interop: {dir}: refused: outside the project");
        assert!(
            w.logs.iter().any(|l| l.starts_with(&refused)),
            "{:?}",
            w.logs
        );
        assert!(
            w.logs
                .iter()
                .any(|l| l == &format!("interop: {dir}: cannot open"))
        );
    }
    let faults = w.plugins.health().faults();
    assert_eq!(faults.len(), 2, "{faults:?}");
    assert!(faults.iter().all(|f| f.code == Code::Denied), "{faults:?}");
}

fn want_sfyx(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    native(
        Command::VirtuosoExport {
            tops: vec!["top.shcomp".into()],
            out: "out/inv.sfyx".into(),
            opts: ExportOptions {
                producer: "schemify interop plugin".into(),
                ..ExportOptions::default()
            },
        },
        files,
    )["out/inv.sfyx"]
        .clone()
}

/// The package manifest (`schemify.project`, read by `schemify pack`) is canonical, names the
/// committed module, requests exactly the grants these tests run the guest with, and suggests
/// the fuel ceiling (README: a real conversion needs it).
#[test]
fn package_manifest_requests_the_tested_grants() {
    let text = include_str!("../schemify.project");
    let doc = schemify_format::parse(text.as_bytes()).expect("parses");
    assert_eq!(schemify_format::write(&doc), text, "canonical");
    let mut lines = text.lines().skip(2);
    assert_eq!(lines.next(), Some("plugin interop path interop.wasm"));
    let rest: Vec<&str> = lines.collect();
    let grants: Vec<&str> = rest
        .iter()
        .filter_map(|l| l.strip_prefix("  grant "))
        .collect();
    assert_eq!(grants, GRANTS);
    assert!(rest.contains(&"  fuel 100000000000"), "{rest:?}");
}
