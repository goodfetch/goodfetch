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
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure};
use goodfetch_protocol::run::{EndingError, Lifecycle, RunReport, Verdict};

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

/// The exit code of a command that answered `report`: the row of the error
/// its ending raised when it carries one, as a call that ends a Run fails
/// with it, the `internal` row for a Run that ended `failed` without one;
/// else `20` when its Run ended `stopped`, which a resume by its Run ID
/// goes on with; else, when `require_verified` (`--require-verified`),
/// `21` for a Verdict other than `verified`; else success. A stopped Run's
/// Verdict is not its last, so a stop wins.
pub fn of_report(report: &RunReport, require_verified: bool) -> u8 {
    if let Some(error) = &report.error {
        of_code(&error.code)
    } else if report.lifecycle == Lifecycle::Failed {
        unanswered()
    } else if report.lifecycle == Lifecycle::Stopped {
        cli::RUN_STOPPED.code
    } else if require_verified && report.verdict != Verdict::Verified {
        cli::VERDICT_NOT_VERIFIED.code
    } else {
        SUCCESS
    }
}

/// The exit code of a command whose engine answered what no call answers,
/// or nothing at all: a bug in Goodfetch, the `internal` group's row.
pub fn unanswered() -> u8 {
    ErrorGroup::Internal.exit_code()
}

/// The group of `error`: its numeric class's, or for a failure of the
/// protocol itself, the one ADR 0079 gives it.
pub fn group(error: &ErrorObject) -> ErrorGroup {
    ErrorGroup::ALL
        .into_iter()
        .find(|group| group.json_rpc_class() == error.code)
        .or_else(|| {
            [
                ProtocolFailure::ParseError,
                ProtocolFailure::InvalidRequest,
                ProtocolFailure::MessageTooLarge,
                ProtocolFailure::MessageTooDeep,
                ProtocolFailure::MethodNotFound,
                ProtocolFailure::InvalidParams,
                ProtocolFailure::InternalError,
            ]
            .into_iter()
            .find(|failure| failure.name() == error.data.code)
            .map(ProtocolFailure::group)
        })
        .unwrap_or(ErrorGroup::Internal)
}

/// The failure a Run report's ending `error` raises, as a call that ends
/// the Run fails with it: its Error code's, or `internal_error` for a code
/// this executable does not know.
pub fn of_ending(error: &EndingError) -> ErrorObject {
    let definition = MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .find(|definition| definition.name == error.code);
    let (code, retryable) = definition.map_or(
        (ProtocolFailure::InternalError.code(), false),
        |definition| (definition.group.json_rpc_class(), definition.retryable),
    );
    ErrorObject {
        code,
        message: error.message.clone(),
        data: ErrorData {
            code: error.code.clone(),
            retryable,
            details: error.details.clone(),
        },
    }
}

/// The failure of a command whose engine answered what no call answers,
/// or ended its Connection unanswered, as `message` says: a bug in
/// Goodfetch, `internal_error`, which exits [`unanswered`]'s row.
pub fn internal(message: String) -> ErrorObject {
    let failure = ProtocolFailure::InternalError;
    ErrorObject {
        code: failure.code(),
        message,
        data: ErrorData {
            code: failure.name().to_owned(),
            retryable: false,
            details: serde_json::Map::new(),
        },
    }
}

#[cfg(test)]
mod tests;
