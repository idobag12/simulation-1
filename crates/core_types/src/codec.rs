//! The single canonical binary encoding for saves and state hashing.
//!
//! Invariant: every structure that is persisted or hashed anywhere in
//! Embervale is encoded through [`to_bytes`]/[`from_bytes`] — there is
//! exactly one bincode configuration in the codebase (ADR 0002 §1). Changing
//! the configuration is a save-format break and requires a `format_version`
//! bump plus a migration.

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Error produced by the canonical codec.
#[derive(Debug, Clone, Error)]
pub enum CodecError {
    /// Encoding failed (should be unreachable for well-formed in-memory
    /// state; surfaced rather than panicking per SPEC §3).
    #[error("canonical encode failed: {0}")]
    Encode(String),
    /// Decoding failed: corrupt, truncated, or format-mismatched bytes.
    #[error("canonical decode failed: {0}")]
    Decode(String),
    /// Decoding succeeded but did not consume the whole input — the blob
    /// does not represent exactly one value of the target type.
    #[error("canonical decode left {trailing} trailing bytes")]
    TrailingBytes {
        /// Number of unconsumed bytes.
        trailing: usize,
    },
}

/// The one bincode configuration (little-endian, varint lengths). Not
/// exported: callers must go through this module's functions.
fn config() -> impl bincode::config::Config {
    bincode::config::standard()
}

/// Encodes a value in the canonical binary form.
pub fn to_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    bincode::serde::encode_to_vec(value, config()).map_err(|e| CodecError::Encode(e.to_string()))
}

/// Decodes a value from the canonical binary form.
///
/// Invariant: the entire input must be consumed; trailing bytes are an
/// error, never silently ignored.
pub fn from_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    let (value, consumed) = bincode::serde::decode_from_slice(bytes, config())
        .map_err(|e| CodecError::Decode(e.to_string()))?;
    if consumed != bytes.len() {
        return Err(CodecError::TrailingBytes {
            trailing: bytes.len() - consumed,
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_value() {
        let value: (u64, String, Vec<i64>) = (42, "embervale".into(), vec![-1, 0, 1]);
        let bytes = to_bytes(&value).unwrap();
        let back: (u64, String, Vec<i64>) = from_bytes(&bytes).unwrap();
        assert_eq!(value, back);
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = to_bytes(&7u32).unwrap();
        bytes.push(0);
        let result: Result<u32, _> = from_bytes(&bytes);
        assert!(matches!(
            result,
            Err(CodecError::TrailingBytes { trailing: 1 })
        ));
    }

    #[test]
    fn corrupt_input_is_a_typed_error() {
        // A varint length claiming more data than exists.
        let result: Result<String, _> = from_bytes(&[0xff, 0xff]);
        assert!(matches!(result, Err(CodecError::Decode(_))));
    }
}
