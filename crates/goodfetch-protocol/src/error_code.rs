//! An Error code as an open value type (ADR 0099): a newtype holding the
//! code's `snake_case` string inline, at most 63 bytes, so a code read from
//! the wire equals its constant with `==` and in a `match`, whichever crate
//! defines the constant, and a code the program does not know stays a value.
//! Each Collie code is an associated constant of [`ErrorCode`] and each
//! Foxhound code a free constant of `foxhound::error_code`, both generated
//! from the catalog by `xtask codegen`. An Error code's typed details name
//! their code through [`ErrorDetails`].

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An Error code, such as `target_not_unique`: one of the catalog's
/// constants, or a code a newer engine sent that this program does not know.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorCode {
    /// How many of `bytes` the code takes; the rest are zero, so two equal
    /// codes are equal byte for byte.
    len: u8,
    bytes: [u8; ErrorCode::MAX_LEN],
}

impl ErrorCode {
    /// The longest code it holds, in bytes, which codegen checks on every
    /// code of the catalog (ADR 0099).
    pub const MAX_LEN: usize = 63;

    /// The code `code`, for a constant.
    ///
    /// # Panics
    ///
    /// When `code` is longer than [`ErrorCode::MAX_LEN`] bytes, which in a
    /// constant fails the build.
    #[must_use]
    pub const fn from_static(code: &'static str) -> Self {
        match Self::of(code) {
            Some(code) => code,
            None => panic!("an Error code is at most 63 bytes"),
        }
    }

    /// The code `code`, or none when it is longer than
    /// [`ErrorCode::MAX_LEN`] bytes, which no catalog code is.
    #[must_use]
    pub fn new(code: &str) -> Option<Self> {
        Self::of(code)
    }

    const fn of(code: &str) -> Option<Self> {
        let source = code.as_bytes();
        if source.len() > Self::MAX_LEN {
            return None;
        }
        let mut bytes = [0; Self::MAX_LEN];
        let mut at = 0;
        while at < source.len() {
            bytes[at] = source[at];
            at += 1;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the length is at most MAX_LEN, 63, checked above"
        )]
        let len = source.len() as u8;
        Some(Self { len, bytes })
    }

    /// The code's `snake_case` string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        let bytes = &self.bytes[..usize::from(self.len)];
        std::str::from_utf8(bytes).expect("invariant: an ErrorCode holds the bytes of a str")
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ErrorCode")
            .field(&self.as_str())
            .finish()
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = std::borrow::Cow::<str>::deserialize(deserializer)?;
        Self::new(&code).ok_or_else(|| {
            serde::de::Error::custom(format_args!(
                "an Error code is at most {} bytes",
                Self::MAX_LEN
            ))
        })
    }
}

/// The typed details of an Error code, which an SDK's error reads by type
/// (ADR 0099): `D::CODES` are the codes whose details `D` holds, most often
/// one.
pub trait ErrorDetails: DeserializeOwned {
    /// The codes whose details these are.
    const CODES: &'static [ErrorCode];
}

include!("../generated/error_codes.rs");

#[cfg(test)]
mod tests;
