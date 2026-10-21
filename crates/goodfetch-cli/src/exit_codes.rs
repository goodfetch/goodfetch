//! The CLI's exit codes (ADR 0069's table, whose rows `goodfetch-protocol`
//! holds): `0` on success, `2` for a usage error before any engine call,
//! and for a failed call the row of its Error code when the catalog gives
//! it one, else its group's.

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::entries::MODULES;

/// A command that did what it was asked.
pub const SUCCESS: u8 = 0;

/// The exit code of a command that failed with the Error code `code` of
/// `group`: the code's own row of the table, or its group's.
pub fn of_error(code: &str, group: ErrorGroup) -> u8 {
    MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .find(|definition| definition.name == code)
        .and_then(|definition| definition.exit_code)
        .unwrap_or_else(|| group.exit_code())
}

/// The exit code of a command whose call got no answer: a bug in
/// Goodfetch, the `internal` group's row.
pub fn unanswered() -> u8 {
    ErrorGroup::Internal.exit_code()
}
