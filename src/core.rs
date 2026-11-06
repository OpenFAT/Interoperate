//! The table that maps xschem's `devices/<name>.sym` onto the bundled `core` library
//! (spec `import.md` §6), and that library itself ([`core_library`], the files in `core/`).
//!
//! The core cells' pin *positions* follow xschem's devices, because imported schematics
//! connect by geometry. The map below is data: one row per mapped symbol, with the pin and
//! parameter renames between the xschem symbol and the core cell. The translator
//! applies a row only when the xschem symbol it finds has exactly the row's pins at
//! the core cell's positions; otherwise the symbol is converted like any other.

/// One xschem `devices/` symbol mapped to a core cell.
#[derive(Clone, Copy, Debug)]
pub struct DeviceMap {
    /// xschem symbol file stem in the `devices` library (`res` for `devices/res.sym`).
    pub symbol: &'static str,
    /// Core cell name.
    pub cell: &'static str,
    /// `(xschem pin, core pin)` renames; pins not listed keep their name.
    pub pins: &'static [(&'static str, &'static str)],
    /// `(xschem param, core param)` renames; params not listed keep their name.
    /// xschem parameters the core cell lacks (`footprint`, `device`, …) only feed
    /// other netlist formats and are dropped (counted in the report).
    pub params: &'static [(&'static str, &'static str)],
}

const PM: &[(&str, &str)] = &[("m", "n")];
const PM_UP: &[(&str, &str)] = &[("P", "p"), ("M", "n")];
const CTRL: &[(&str, &str)] = &[("m", "n"), ("cm", "cn")];
const BJT: &[(&str, &str)] = &[("C", "c"), ("B", "b"), ("E", "e")];

/// xschem `devices/*.sym` → core cell (spec §6). Coverage on the corpus is measured by
/// the corpus tests and reported in `docs/spec/import.md`.
pub const DEVICE_MAP: &[DeviceMap] = &[
    DeviceMap {
        symbol: "ammeter",
        cell: "ammeter",
        pins: &[("plus", "p"), ("minus", "n")],
        params: &[],
    },
    DeviceMap {
        symbol: "bsource",
        cell: "bsource",
        pins: PM,
        params: &[("VAR", "src"), ("FUNC", "func")],
    },
    DeviceMap {
        symbol: "capa",
        cell: "cap",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "capa-2",
        cell: "cap",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "cccs",
        cell: "cccs",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "ccvs",
        cell: "ccvs",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "delay_line",
        cell: "tline",
        pins: &[("nap", "p1"), ("nam", "n1"), ("nbp", "p2"), ("nbm", "n2")],
        params: &[],
    },
    DeviceMap {
        symbol: "diode",
        cell: "diode",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "ind",
        cell: "ind",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "isource",
        cell: "isource",
        pins: PM,
        params: &[],
    },
    DeviceMap {
        symbol: "nmos4",
        cell: "nmos4",
        pins: &[],
        params: &[],
    },
    DeviceMap {
        symbol: "npn",
        cell: "npn",
        pins: BJT,
        params: &[],
    },
    DeviceMap {
        symbol: "parax_cap",
        cell: "parax_cap",
        pins: &[],
        params: &[],
    },
    DeviceMap {
        symbol: "pmos4",
        cell: "pmos4",
        pins: &[],
        params: &[],
    },
    DeviceMap {
        symbol: "pnp",
        cell: "pnp",
        pins: BJT,
        params: &[],
    },
    DeviceMap {
        symbol: "res",
        cell: "res",
        pins: PM_UP,
        params: &[],
    },
    DeviceMap {
        symbol: "spice_probe",
        cell: "probe",
        pins: &[],
        params: &[],
    },
    DeviceMap {
        symbol: "switch_ngspice",
        cell: "vswitch",
        pins: &[("P", "p"), ("M", "n"), ("CP", "cp"), ("CM", "cn")],
        params: &[],
    },
    DeviceMap {
        symbol: "vccs",
        cell: "vccs",
        pins: CTRL,
        params: &[],
    },
    DeviceMap {
        symbol: "vcvs",
        cell: "vcvs",
        pins: CTRL,
        params: &[],
    },
    DeviceMap {
        symbol: "vsource",
        cell: "vsource",
        pins: PM,
        params: &[],
    },
];

/// Returns the map row for xschem device `symbol` (file stem).
pub fn device_map(symbol: &str) -> Option<&'static DeviceMap> {
    DEVICE_MAP.iter().find(|m| m.symbol == symbol)
}

impl DeviceMap {
    /// Returns the core name of xschem pin `pin`.
    pub fn pin<'a>(&self, pin: &'a str) -> &'a str {
        self.pins
            .iter()
            .find(|(x, _)| *x == pin)
            .map_or(pin, |(_, c)| c)
    }

    /// Returns the core name of xschem parameter `param`.
    pub fn param<'a>(&self, param: &'a str) -> &'a str {
        self.params
            .iter()
            .find(|(x, _)| *x == param)
            .map_or(param, |(_, c)| c)
    }
}

macro_rules! core_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/core/", $name)))),*]
    };
}

/// The core library's files, sorted by name.
static CORE_FILES: &[(&str, &[u8])] = core_files![
    "ammeter.shprim",
    "bsource.shprim",
    "cap.shprim",
    "cccs.shprim",
    "ccvs.shprim",
    "diode.shprim",
    "gnd.shprim",
    "ind.shprim",
    "isource.shprim",
    "nmos4.shprim",
    "npn.shprim",
    "parax_cap.shprim",
    "pmos4.shprim",
    "pnp.shprim",
    "probe.shprim",
    "res.shprim",
    "switch.shprim",
    "tline.shprim",
    "vccs.shprim",
    "vcvs.shprim",
    "vdd.shprim",
    "vsource.shprim",
    "vswitch.shprim",
];

/// Returns Schemify's built-in `core` library (`lib core builtin`) as `(file name, bytes)`,
/// sorted by file name.
pub fn core_library() -> &'static [(&'static str, &'static [u8])] {
    CORE_FILES
}

/// Returns the bytes of core cell `cell` (a stem: `res` for `res.shprim`).
pub fn core_file(cell: &str) -> Option<&'static [u8]> {
    CORE_FILES
        .iter()
        .find(|(n, _)| n.strip_suffix(".shprim") == Some(cell))
        .map(|(_, b)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapped_cell_exists() {
        for m in DEVICE_MAP {
            assert!(core_file(m.cell).is_some(), "{}", m.cell);
        }
        assert!(CORE_FILES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(DEVICE_MAP.windows(2).all(|w| w[0].symbol < w[1].symbol));
        assert_eq!(device_map("res").map(|m| m.pin("M")), Some("n"));
        assert!(device_map("lab_pin").is_none());
    }
}
