//! The save-format migration pipeline (SPEC §9).
//!
//! Invariants:
//! - Old saves must load forever: every historical `format_version` has a
//!   pure migration path to the current version, chained
//!   `v1 → v2 → … → current`, each step a pure function on the decompressed
//!   body bytes.
//! - A version without a path (typically: newer than this build) is a typed
//!   error, never a guess.
//!
//! Phase 0 note: v1 is the only version that has ever existed, so the
//! pipeline is a single pass-through arm. The intermediate dynamic
//! representation mandated by SPEC §9 for real migrations is introduced
//! together with the first real migration (v2); building it against a
//! single version would be speculative complexity (SPEC §16.3, ADR 0002 §8).

use crate::{FORMAT_VERSION, PersistError};

/// Migrates a decompressed save body from `version` to [`FORMAT_VERSION`].
pub(crate) fn migrate_to_current(version: u32, body: Vec<u8>) -> Result<Vec<u8>, PersistError> {
    match version {
        FORMAT_VERSION => Ok(body),
        other => Err(PersistError::UnsupportedVersion(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_passes_through_unchanged() {
        let body = vec![1, 2, 3];
        assert_eq!(
            migrate_to_current(FORMAT_VERSION, body.clone()).unwrap(),
            body
        );
    }

    #[test]
    fn unknown_versions_error() {
        assert!(matches!(
            migrate_to_current(0, vec![]),
            Err(PersistError::UnsupportedVersion(0))
        ));
        assert!(matches!(
            migrate_to_current(FORMAT_VERSION + 1, vec![]),
            Err(PersistError::UnsupportedVersion(_))
        ));
    }
}
