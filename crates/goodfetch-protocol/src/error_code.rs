//! An Error code as an open value type (ADR 0099): a newtype holding the
//! code's `snake_case` string inline, at most 63 bytes, so a code read from
//! the wire equals its constant with `==` and in a `match`, whichever crate
//! defines the constant, and a code the program does not know stays a value.
//! Each Collie code is an associated constant of [`ErrorCode`] and each
//! Foxhound code a free constant of `foxhound::error_code`, both generated
//! from the catalog by `xtask codegen`. An Error code's typed details name
//! their code through [`ErrorDetails`].

use serde::de::DeserializeOwned;

use crate::inline_value::open_value_type;

open_value_type!(
    /// An Error code, such as `target_not_unique`: one of the catalog's
    /// constants, or a code a newer engine sent that this program does not
    /// know.
    ErrorCode,
    "an Error code"
);

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
