//! Shared helpers for the cross-crate integration suites in `tests/tests/`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

/// Loads the FROZEN data snapshot committed beside the save fixtures
/// (`tests/fixtures/data_v7/`). Fixture-pinned suites (determinism,
/// events, save_compat) use this so live balance edits in `data/` never
/// silently shift their goldens; the demographic regression suite instead
/// loads the live `data/` directory on purpose (it validates shipped
/// balance).
pub fn pinned_defs() -> data_defs::DataDefs {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/data_v7"));
    data_defs::load(root).expect("committed data snapshot must load")
}

/// Loads the live `data/` directory (the shipped balance).
pub fn live_defs() -> data_defs::DataDefs {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../data"));
    data_defs::load(root).expect("live data directory must load")
}
