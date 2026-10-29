//! What a CLI command prints under `--json` when it fails (ADR 0069): one
//! JSON value, `{ "error": … }`, holding the Error code, its group, whether
//! it is retryable, the message, the operation and the code's own details.
//! A command that succeeds prints its operation's result, the catalog's
//! type. It also holds ADR 0069's exit-code table, which the catalog file
//! lists as `exit_codes`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::ErrorGroup;
use crate::entries::MODULES;

/// What a command prints under `--json` when it fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandFailed {
    /// The failure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: CommandError,
}

/// A failed command's error: what a Client's error object holds, with the
/// operation the command called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    /// The Error code, or the name of the protocol failure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// The group of the Error code, named for its remedy (ADR 0079).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group: ErrorGroup,
    /// Whether the same call may succeed when made again.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub retryable: bool,
    /// What went wrong, for people; never matched on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The operation the command called, by its canonical name, such as
    /// `run.finish`; absent when the command failed before it could call
    /// one, as `npx @goodfetch/collie` does when it finds no executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: Option<String>,
    /// The Error code's own details, such as `fix`.
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

/// A row of ADR 0069's exit-code table: the exit code of a CLI command and
/// what it means. An Error code's own row is its definition's `exit_code`
/// and a group's is [`ErrorGroup::exit_code`]; the [`OUTCOMES`] are the
/// command's other endings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCodeRow {
    /// Its `snake_case` name: the outcome's, the group's or the Error
    /// code's.
    pub name: &'static str,
    /// The exit code.
    pub code: u8,
    /// What it means.
    pub description: &'static str,
}

/// A command that did what it was asked.
pub const SUCCESS: ExitCodeRow = ExitCodeRow {
    name: "success",
    code: 0,
    description: "The command did what it was asked.",
};

/// A command line the executable cannot read, refused before any engine
/// call.
pub const USAGE: ExitCodeRow = ExitCodeRow {
    name: "usage",
    code: 2,
    description: "The command line is wrong: an unknown command or flag, or a bad value. Nothing reached the engine.",
};

/// A command whose Run ended `stopped`, which a resume by its Run ID goes
/// on with.
pub const RUN_STOPPED: ExitCodeRow = ExitCodeRow {
    name: "run_stopped",
    code: 20,
    description: "The Run ended `stopped`: a Limit, an abort, its timeout, a pause or the loss of its engine stopped it. Resume it by its Run ID.",
};

/// A command given `--require-verified` whose Run, or whose Agent's Success
/// check, ended with a Verdict other than `verified`.
pub const VERDICT_NOT_VERIFIED: ExitCodeRow = ExitCodeRow {
    name: "verdict_not_verified",
    code: 21,
    description: "`--require-verified` was given and the Run, or the Agent's Success check, ended with a Verdict other than `verified`: `rejected` or `needs_review`.",
};

/// A command interrupted by `SIGINT` (Ctrl-C), or `CTRL_BREAK_EVENT` on
/// Windows, once its call was cancelled: the shell's `128 + 2`.
pub const INTERRUPTED: ExitCodeRow = ExitCodeRow {
    name: "interrupted",
    code: 130,
    description: "The command was interrupted (`SIGINT`, or `CTRL_BREAK_EVENT` on Windows): its call was cancelled, and the Run it started checkpointed and stopped, resumable by its Run ID.",
};

/// A command ended by `SIGTERM`, once its call was cancelled: the shell's
/// `128 + 15`.
pub const TERMINATED: ExitCodeRow = ExitCodeRow {
    name: "terminated",
    code: 143,
    description: "The command was sent `SIGTERM`: its call was cancelled, and the Run it started checkpointed and stopped, resumable by its Run ID.",
};

/// `audit verify`, whose log or export holds an entry that does not
/// verify, like `sha256sum -c` meeting a mismatch (ADR 0069).
pub const AUDIT_LOG_NOT_INTACT: ExitCodeRow = ExitCodeRow {
    name: "audit_log_not_intact",
    code: 22,
    description: "`audit verify` found an entry that does not verify: the Audit log, or the export it was given, was edited, deleted from, reordered or truncated at the sequence its output names.",
};

/// The rows that are no Error code's or group's.
pub const OUTCOMES: [ExitCodeRow; 7] = [
    SUCCESS,
    USAGE,
    RUN_STOPPED,
    VERDICT_NOT_VERIFIED,
    AUDIT_LOG_NOT_INTACT,
    INTERRUPTED,
    TERMINATED,
];

/// The one failure that exits by another Error code's row (ADR 0069's
/// #1183 note): a Daemon's `not_in_product` refusal of `hello` while it
/// cannot hand over exits by `protocol_version_unsupported`'s row, since
/// the same `daemon restart` in its `fix` ends both. That row says so.
pub const REFUSED_AT_HELLO: &str = "The Daemon refused the command's `hello`: by protocol version (`protocol_version_unsupported`), or, while it could not hand over, by Product (`not_in_product`, whose own row is for a call refused after `hello`). Run the `daemon restart` the refusal's `fix` names, or upgrade the Client.";

/// The row whose description [`REFUSED_AT_HELLO`] gives.
const ROW_OF_REFUSED_AT_HELLO: &str = "protocol_version_unsupported";

/// ADR 0069's whole exit-code table, by exit code: the [`OUTCOMES`], each
/// group's row and each Error code's own row. An Error code without a row
/// of its own exits with its group's.
pub fn exit_code_table() -> Vec<ExitCodeRow> {
    let groups = ErrorGroup::ALL.into_iter().map(|group| ExitCodeRow {
        name: group.as_str(),
        code: group.exit_code(),
        description: group.description(),
    });
    let codes = MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .filter_map(|definition| {
            Some(ExitCodeRow {
                name: definition.name,
                code: definition.exit_code?,
                description: if definition.name == ROW_OF_REFUSED_AT_HELLO {
                    REFUSED_AT_HELLO
                } else {
                    definition.description
                },
            })
        });
    let mut table: Vec<ExitCodeRow> = OUTCOMES.into_iter().chain(groups).chain(codes).collect();
    table.sort_by_key(|row| (row.code, row.name));
    table
}
