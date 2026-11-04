//! The SKILL file the plugin ships (spec `interop.md` §5), assembled from its parts in
//! `skill/` so each part can be reviewed and tested on its own.

/// The parts of `schemify.il`, in load order: (file name, text).
pub const PARTS: &[(&str, &str)] = &[
    ("sfy_io.il", include_str!("../skill/sfy_io.il")),
    ("sfy_cells.il", include_str!("../skill/sfy_cells.il")),
    ("sfy_ade.il", include_str!("../skill/sfy_ade.il")),
    // Lane parts, empty until their lanes fill them (README "Development").
    ("sfy_cfg.il", include_str!("../skill/sfy_cfg.il")),
    ("sfy_results.il", include_str!("../skill/sfy_results.il")),
    ("sfy_adel.il", include_str!("../skill/sfy_adel.il")),
    ("sfy_layout.il", include_str!("../skill/sfy_layout.il")),
    ("sfy_cdfcb.il", include_str!("../skill/sfy_cdfcb.il")),
    ("sfy_char.il", include_str!("../skill/sfy_char.il")),
    ("sfy_selftest.il", include_str!("../skill/sfy_selftest.il")),
];

/// The probe a user runs once on a real Virtuoso (§5.5); not part of `schemify.il`.
pub const PROBE: &str = include_str!("../skill/sfy_probe.il");

/// `schemify.il`: every part, each preceded by a comment naming it; a part of comments only
/// (an empty lane part) is left out.
pub fn schemify_il() -> String {
    let mut out = String::from(
        ";; schemify.il: Schemify <-> Virtuoso bridge (interop plugin). Load with\n;; (load \"/absolute/path/schemify.il\"); entry points: SfyExport SfyExportLib SfyImport SfyCharacterize SfySelfTest.\n",
    );
    for (name, text) in PARTS {
        // An empty lane part ships nothing, so adding one changes no byte of schemify.il
        // (nor of the guest that embeds it) until its lane writes code there.
        if text
            .lines()
            .all(|l| l.trim().is_empty() || l.trim_start().starts_with(';'))
        {
            continue;
        }
        out.push_str(&format!("\n;; ---- {name}\n"));
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push('\n');
        }
    }
    assert!(out.is_ascii(), "schemify.il must be ASCII");
    out
}
