//! Save-format compatibility suite (SPEC §9: "Old saves must load forever").
//!
//! `fixtures/v1_seed7_fixture50_tick1000.embersave` is a byte-frozen v1
//! save, committed at Phase 0: seed 7, 50 initial fixture entities, run
//! 1,000 ticks. Every future change to the codec configuration, `SaveBody`
//! layout, canonical store form, component names, RNG serialization, seed
//! derivation, or hash algorithm must keep this file loading — and loading
//! to exactly the golden state — or ship a `format_version` bump plus a
//! migration that makes it pass again. Regenerating the fixture to make a
//! red test green is a save-format break and is forbidden without that
//! migration (SPEC §9, ADR 0002 §§1-3, §6, §8).

use core_types::{Seed, Ticks, WorldHash};
use headless::runner;

/// The committed v1 fixture bytes.
const FIXTURE: &[u8] = include_bytes!("../fixtures/v1_seed7_fixture50_tick1000.embersave");

/// Golden state hash of the fixture as saved (tick 1,000), recorded at
/// Phase 0 from the run that produced the file.
const GOLDEN_HASH_AT_SAVE: u64 = 0x147b_2178_465a_09ad;

/// Golden state hash after resuming the fixture for 100 further ticks —
/// freezes not just loading but continued deterministic evolution.
const GOLDEN_HASH_AFTER_100: u64 = 0x61c1_fe74_25d2_ea8e;

#[test]
fn v1_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(FIXTURE, runner::register_components)
        .expect("committed v1 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(1000));
    assert_eq!(sim.seed(), Seed::new(7));
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(GOLDEN_HASH_AT_SAVE),
        "loaded state differs from the state that was saved at Phase 0"
    );
}

#[test]
fn v1_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(FIXTURE, runner::register_components)
        .expect("committed v1 save no longer loads");
    let mut schedule = runner::build_schedule(&runner::WorldSpec::fixture(sim.seed(), 0));
    sim.run_ticks(&mut schedule, 100).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(GOLDEN_HASH_AFTER_100),
        "resumed evolution diverged from the Phase 0 recording"
    );
}
