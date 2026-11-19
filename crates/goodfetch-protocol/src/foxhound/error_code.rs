//! Foxhound's Error codes, each a free constant of the open
//! [`ErrorCode`](crate::error_code::ErrorCode) Collie's codes are
//! associated constants of (ADR 0099), generated from the catalog by
//! `xtask codegen`. Foxhound's crate re-exports this module.

use crate::error_code::ErrorCode;

include!("../../generated/foxhound_error_codes.rs");
