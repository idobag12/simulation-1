//! Versioned save/load (SPEC §9).
//!
//! Save format (ADR 0002 §8, ADR 0004 §9):
//! `[8-byte magic "EMBRSAV1"][u32 LE format_version][zstd(codec(SaveBody))]`
//! (the magic identifies the file family and never changes; the version
//! field routes migrations).
//!
//! Invariants:
//! - The version header sits *outside* the compressed payload so the
//!   migration pipeline can route before decoding.
//! - RNG streams, the entity allocator, every component store, and the
//!   full event state (bus queues, log ring, scheduler queue) are saved
//!   and restored exactly: loading a save and running N ticks equals
//!   running the original world those same N ticks (permanent CI test).
//! - Loading is strict: unknown magic, unknown version, codec errors, or a
//!   component/event registration mismatch are typed errors — never a
//!   silent default.
//! - Old saves must load forever: a format change bumps `FORMAT_VERSION`
//!   and adds a pure migration in [`migrations`], proven against the
//!   committed golden fixture of every previous version.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod migrations;

use core_ecs::{EcsError, World};
use core_types::codec::{self, CodecError};
use core_types::{Seed, Ticks};
use serde::{Deserialize, Serialize};
use sim_time::{Calendar, Simulation};
use thiserror::Error;

/// Current save format version. Bumping this requires a migration in
/// [`migrations`] and a compatibility test that loads the previous version.
/// History: v1 = Phase 0 (no event state); v2 = Phase 1 (+ event state);
/// v3 = Phase 2 (+ people registrations — ADR 0005 §9: appending
/// registrations is a format bump with a list-extension migration);
/// v4 = Phase 3 (+ world/AI registrations, ADR 0006 §8);
/// v5 = Phase 4 (+ economy registrations and events, ADR 0007 §9);
/// v6 = Phase 5 (+ labor registrations and events, ADR 0008 §8);
/// v7 = Phase 6 (+ money registrations and events, ADR 0009 §6);
/// v8 = Phase 7 (+ social registrations and events, ADR 0010 §6);
/// v9 = Phase 8 (+ LOD registrations and the tier-change event,
/// ADR 0011 §6).
pub const FORMAT_VERSION: u32 = 9;

/// 8-byte file magic identifying an Embervale save.
pub const MAGIC: &[u8; 8] = b"EMBRSAV1";

// zstd compression level. Affects file size only, never simulation
// behavior — not a balance tunable (ADR 0002 §8).
const ZSTD_LEVEL: i32 = 3;

/// Errors produced by saving and loading.
#[derive(Debug, Error)]
pub enum PersistError {
    /// The blob is too short to contain the header.
    #[error("save data truncated: {0} bytes is too short for the header")]
    Truncated(usize),
    /// The magic bytes do not identify an Embervale save.
    #[error("bad magic: not an Embervale save")]
    BadMagic,
    /// The save's format version has no migration path (newer than this
    /// build, or an unknown value).
    #[error("unsupported save format version {0} (current {FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    /// Compression or decompression of the payload failed (corrupt or
    /// truncated compressed data). Distinct from [`PersistError::Io`]:
    /// matching this variant means the *bytes* are bad, not the filesystem.
    #[error("zstd: {0}")]
    Zstd(std::io::Error),
    /// Reading or writing the save file failed (missing file, permissions).
    #[error("io on `{path}`: {source}")]
    Io {
        /// The file that could not be read or written.
        path: std::path::PathBuf,
        /// The underlying filesystem error.
        source: std::io::Error,
    },
    /// Canonical encoding/decoding failed.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// World reconstruction failed (registration mismatch, blob mismatch).
    #[error(transparent)]
    Ecs(#[from] EcsError),
}

/// The full serialized world state (SPEC §9), current format (v9).
/// Everything a running simulation is, minus the schedule and calendar,
/// which the application reconstructs exactly as it reconstructs
/// registrations.
///
/// Invariant: this struct is the CURRENT format. When the format changes,
/// it is copied verbatim into [`migrations`] under its version name and
/// frozen there forever (ADR 0004 §9).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SaveBody {
    pub(crate) seed: Seed,
    pub(crate) tick: Ticks,
    pub(crate) entities: Vec<u8>,
    pub(crate) rng: Vec<u8>,
    pub(crate) components: Vec<(String, Vec<u8>)>,
    pub(crate) events: Vec<u8>,
}

/// The world-assembly configuration a load needs: the same values the
/// application used to build the fresh world (from `data/`), because a
/// loaded world is reconstructed through the same path.
#[derive(Debug, Clone, Copy)]
pub struct LoadConfig {
    /// The calendar (from `data/balance/calendar.ron`).
    pub calendar: Calendar,
    /// Event log ring capacity (from `data/balance/engine.ron`).
    pub event_log_capacity: usize,
}

/// Serializes a simulation to the versioned, compressed save format.
pub fn save_to_bytes(sim: &Simulation) -> Result<Vec<u8>, PersistError> {
    let body = SaveBody {
        seed: sim.seed(),
        tick: sim.tick(),
        entities: sim.world().entities_to_bytes()?,
        rng: sim.world().rng_to_bytes()?,
        components: sim.world().component_blobs()?,
        events: sim.world().events_to_bytes()?,
    };
    let raw = codec::to_bytes(&body)?;
    let compressed =
        zstd::stream::encode_all(raw.as_slice(), ZSTD_LEVEL).map_err(PersistError::Zstd)?;

    let mut out = Vec::with_capacity(MAGIC.len() + 4 + compressed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Deserializes a simulation from the save format (any supported version;
/// older versions migrate).
///
/// `register` must register exactly the component AND event sets (in
/// exactly the order) the application registers for a fresh world; any
/// mismatch with the save is a typed error (ADR 0002 §6, ADR 0004 §4).
pub fn load_from_bytes(
    bytes: &[u8],
    config: LoadConfig,
    register: impl FnOnce(&mut World) -> Result<(), EcsError>,
) -> Result<Simulation, PersistError> {
    let header_len = MAGIC.len() + 4;
    if bytes.len() < header_len {
        return Err(PersistError::Truncated(bytes.len()));
    }
    let (magic, rest) = bytes.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err(PersistError::BadMagic);
    }
    let (version_bytes, payload) = rest.split_at(4);
    let mut version_arr = [0u8; 4];
    version_arr.copy_from_slice(version_bytes);
    let version = u32::from_le_bytes(version_arr);

    let raw = zstd::stream::decode_all(payload).map_err(PersistError::Zstd)?;
    let body = migrations::migrate_to_current(version, raw)?;

    let mut world = World::new(body.seed, config.event_log_capacity);
    register(&mut world)?;
    world.restore_entities(&body.entities)?;
    world.load_component_blobs(&body.components)?;
    world.restore_rng(&body.rng)?;
    // A zero-length events blob is the migration marker for "saved before
    // the event system existed" (migrations::v1_to_v2): the fresh, empty
    // event system with live registration IS that world's event state.
    if !body.events.is_empty() {
        world.restore_events(&body.events)?;
    }
    Ok(Simulation::from_parts(world, config.calendar, body.tick))
}

/// Saves to a file. Tooling convenience over [`save_to_bytes`].
pub fn save_to_file(sim: &Simulation, path: &std::path::Path) -> Result<(), PersistError> {
    let bytes = save_to_bytes(sim)?;
    std::fs::write(path, bytes).map_err(|source| PersistError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Loads from a file. Tooling convenience over [`load_from_bytes`].
pub fn load_from_file(
    path: &std::path::Path,
    config: LoadConfig,
    register: impl FnOnce(&mut World) -> Result<(), EcsError>,
) -> Result<Simulation, PersistError> {
    let bytes = std::fs::read(path).map_err(|source| PersistError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    load_from_bytes(&bytes, config, register)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ecs::{Component, StorageKind};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Counter(u64);
    impl Component for Counter {
        const NAME: &'static str = "test.counter";
        const STORAGE: StorageKind = StorageKind::Dense;
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Beep(u64);
    impl core_ecs::Event for Beep {
        const NAME: &'static str = "test.beep";
    }

    fn register(world: &mut World) -> Result<(), EcsError> {
        world.register::<Counter>()?;
        world.register_event::<Beep>()
    }

    // Test fixture parameters: 30-day seasons, event log capacity 16.
    fn config() -> LoadConfig {
        LoadConfig {
            calendar: Calendar::new(30).expect("static test config"),
            event_log_capacity: 16,
        }
    }

    fn sample_sim() -> Simulation {
        let mut sim = Simulation::new(
            World::new(Seed::new(99), config().event_log_capacity),
            config().calendar,
        );
        register(sim.world_mut()).unwrap();
        let world = sim.world_mut();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Counter(1)).unwrap();
        world.insert(b, Counter(2)).unwrap();
        world.despawn(a).unwrap();
        use core_rng::RngCore;
        let _ = world.rng("test.stream").next_u64();
        world.emit(&Beep(1)).unwrap();
        world
            .schedule_event(core_types::Ticks::new(500), &Beep(2))
            .unwrap();
        sim
    }

    #[test]
    fn round_trip_preserves_hash_and_save_bytes() {
        let sim = sample_sim();
        let bytes = save_to_bytes(&sim).unwrap();
        let loaded = load_from_bytes(&bytes, config(), register).unwrap();
        assert_eq!(sim.state_hash().unwrap(), loaded.state_hash().unwrap());
        assert_eq!(sim.tick(), loaded.tick());
        assert_eq!(sim.seed(), loaded.seed());
        // Saving the loaded sim reproduces identical bytes.
        assert_eq!(bytes, save_to_bytes(&loaded).unwrap());
    }

    #[test]
    fn bad_magic_and_truncation_are_typed_errors() {
        let sim = sample_sim();
        let mut bytes = save_to_bytes(&sim).unwrap();
        assert!(matches!(
            load_from_bytes(&bytes[..6], config(), register),
            Err(PersistError::Truncated(6))
        ));
        bytes[0] = b'X';
        assert!(matches!(
            load_from_bytes(&bytes, config(), register),
            Err(PersistError::BadMagic)
        ));
    }

    #[test]
    fn missing_file_is_io_not_zstd() {
        let result = load_from_file(
            std::path::Path::new("/nonexistent/dir/embervale.sav"),
            config(),
            register,
        );
        assert!(matches!(result, Err(PersistError::Io { .. })));
    }

    #[test]
    fn corrupt_payload_is_zstd_not_io() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(b"this is not a zstd frame");
        assert!(matches!(
            load_from_bytes(&bytes, config(), register),
            Err(PersistError::Zstd(_))
        ));
    }

    #[test]
    fn unknown_version_is_a_typed_error() {
        let sim = sample_sim();
        let mut bytes = save_to_bytes(&sim).unwrap();
        // Corrupt the version field (little-endian u32 after the magic).
        bytes[8] = 0xff;
        assert!(matches!(
            load_from_bytes(&bytes, config(), register),
            Err(PersistError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn registration_mismatch_is_a_typed_error() {
        let sim = sample_sim();
        let bytes = save_to_bytes(&sim).unwrap();
        let result = load_from_bytes(&bytes, config(), |_| Ok(()));
        assert!(matches!(
            result,
            Err(PersistError::Ecs(EcsError::ComponentBlobMismatch(_)))
        ));
    }
}
