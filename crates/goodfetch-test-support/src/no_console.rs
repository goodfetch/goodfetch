//! What a Product's executable shows of a Console, now that neither
//! carries one (ADR 0137): the same two observations for Collie's and
//! Foxhound's, which each executable's own test asserts. The executable
//! refuses a `console.*` setting as any key it does not know, and the
//! Daemon it starts publishes a runtime file that names its endpoint and no
//! second listener.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the runtime file outside any async worker (ADR 0062)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::cli::{CliError, cli};
use crate::daemon::{DAEMON_DEADLINE, Daemon, DaemonError};
use crate::env::TestEnv;
use crate::runtime_file::RuntimeFile;

/// The exit status and the Error code `executable` answers
/// `config explain --json` with when user configuration sets
/// `console.address`.
///
/// # Errors
///
/// The command could not run.
pub fn console_address_answer(executable: &Path) -> Result<(Option<i32>, Value), CliError> {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");
    let output = cli(&env, executable)
        .args(["config", "explain", "--json"])
        .run()?;
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);
    Ok((output.exit, printed["error"]["code"].clone()))
}

/// The members of the runtime file the Daemon of `executable` publishes
/// in a new test home: each listener a Client can find is named there
/// (ADR 0024).
///
/// # Errors
///
/// The Daemon did not start or stop, or its runtime file is no JSON
/// object.
pub fn runtime_file_members(executable: &Path) -> Result<BTreeSet<String>, DaemonError> {
    let env = TestEnv::new();
    let daemon = Daemon::start_program(&env, executable, DAEMON_DEADLINE)?;
    let path = RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"));
    let published = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    daemon.stop(&env)?;
    let Some(Value::Object(members)) = published else {
        return Err(DaemonError::Unpublished { path });
    };
    Ok(members.into_iter().map(|(name, _)| name).collect())
}
