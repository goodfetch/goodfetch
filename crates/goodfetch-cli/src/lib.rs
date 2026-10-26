//! The CLI both Products' executables carry (ADRs 0069, 0090 and 0099): the
//! commands generated from the catalog, the engine their calls run on, the
//! one writer of stdout and stderr, and the exit codes. Its own crate so
//! that Foxhound's executable reaches Collie's command tree without Collie's
//! executable crate (STYLE.md, reason 2); it names no Foxhound type (ADR
//! 0087).

pub mod engine_choice;
pub mod exit_codes;
pub mod generated;
pub mod other_product;
pub mod output;
pub mod settings_flags;
