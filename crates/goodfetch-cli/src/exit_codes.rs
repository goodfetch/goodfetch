//! The CLI's exit codes (ADR 0069's table, whose rows `goodfetch-protocol`
//! holds, [`exit_code_table`](goodfetch_protocol::cli::exit_code_table)):
//! `0` on success, `2` for a usage error before any engine call, for a
//! failed call the row of its Error code when the catalog gives it one,
//! else its group's, for a Run report how its Run ended, and `130` or `143`
//! for a command interrupted by `SIGINT` or ended by `SIGTERM`.

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::cli;
use goodfetch_protocol::connection::PROTOCOL_VERSION_UNSUPPORTED;
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::run::{Lifecycle, RunReport, Verdict};

/// A command that did what it was asked.
pub const SUCCESS: u8 = cli::SUCCESS.code;

/// A command interrupted by `SIGINT` (Ctrl-C), or `CTRL_BREAK_EVENT` on
/// Windows, its call cancelled when it could be: the shell's `128 + 2`.
pub const INTERRUPTED: u8 = cli::INTERRUPTED.code;

/// A command ended by `SIGTERM`, its call cancelled when it could be: the
/// shell's `128 + 15`.
pub const TERMINATED: u8 = cli::TERMINATED.code;

/// The exit code of a command a Daemon refused at `hello` by Product or by
/// protocol version, while it could not hand over, under `--daemon` or any
/// other Client that never falls back to an Attached engine: the
/// `protocol_version_unsupported` row, `11`, for both, since the same
/// `daemon restart` in the refusal's `fix` ends both (ticket #1183).
pub const DAEMON_REFUSED: u8 = PROTOCOL_VERSION_UNSUPPORTED
    .exit_code
    .expect("invariant: protocol_version_unsupported has its own row");

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

/// The exit code of a failure that names the Error code `code` without its
/// group, such as a Daemon's Console that did not start: the code's own
/// row, else its declared group's; a code no entry declares, `internal`'s.
pub fn of_code(code: &str) -> u8 {
    MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .find(|definition| definition.name == code)
        .map_or_else(unanswered, |definition| of_error(code, definition.group))
}

/// The exit code of a command that answered `report`: `20` when its Run
/// ended `stopped`, which a resume by its Run ID goes on with; else, when
/// `require_verified` (`--require-verified`), `21` for a Verdict other than
/// `verified`; else success. A stopped Run's Verdict is not its last, so a
/// stop wins.
pub fn of_report(report: &RunReport, require_verified: bool) -> u8 {
    if report.lifecycle == Lifecycle::Stopped {
        cli::RUN_STOPPED.code
    } else if require_verified && report.verdict != Verdict::Verified {
        cli::VERDICT_NOT_VERIFIED.code
    } else {
        SUCCESS
    }
}

/// The exit code of a command whose call got no answer: a bug in
/// Goodfetch, the `internal` group's row.
pub fn unanswered() -> u8 {
    ErrorGroup::Internal.exit_code()
}

#[cfg(test)]
mod tests;
