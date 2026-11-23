//! What every start from a Client is checked for before its Run begins
//! (ADRs 0036, 0078 and 0102): its tags, and whether a `detached` start's
//! Run may outlive the Connection that starts it. Only a Run with no
//! handler in the program, started on a Daemon, detaches. A Run whose
//! start gives it a handler in the program would wait on a Connection
//! gone, and an Attached or linked engine ends with its Connection; either
//! start fails with `detach_unsupported`, which names what kept the Run
//! from detaching.

use goodfetch_protocol::run::{DETACH_UNSUPPORTED, DetachUnsupportedDetails, Tags};

use crate::call_error::CallError;
use crate::composition::CallContext;

/// What gives a Run a handler in the program: the key of its start that
/// chose it, and how to start the Run with none.
#[derive(Clone, Copy, Debug)]
pub struct InProgram {
    /// The key.
    pub key: &'static str,
    /// How to start the Run with no handler in the program.
    pub fix: &'static str,
}

/// Checks a start from a Client before its Run begins: its `tags`, then,
/// when `detached`, that its Run may outlive its Connection, `in_program`
/// naming what gives it a handler in the program, if anything does.
///
/// # Errors
///
/// `invalid_params` naming a tag only the Daemon's scheduler sets;
/// `detach_unsupported` naming `in_program`'s key when it is some, else
/// `engine` when the Connection's engine is no Daemon.
pub fn client_start(
    tags: &Tags,
    detached: bool,
    in_program: Option<InProgram>,
    context: &CallContext,
) -> Result<(), CallError> {
    super::client_tags(tags)?;
    if !detached {
        return Ok(());
    }
    if let Some(InProgram { key, fix }) = in_program {
        return Err(refused(
            format!(
                "`{key}` gives the Run a handler in the program, which would wait on a Connection gone, so it cannot be detached."
            ),
            key,
            fix,
        ));
    }
    if context.daemon().is_none() {
        return Err(refused(
            "This engine ends with its Connection, so no Run it runs can be detached.".to_owned(),
            "engine",
            "Connect to the Daemon, `engine: daemon`, to start the Run detached, or start it without `detached`.",
        ));
    }
    Ok(())
}

fn refused(message: String, key: &str, fix: &str) -> CallError {
    CallError::new(
        &DETACH_UNSUPPORTED,
        message,
        &DetachUnsupportedDetails {
            key: key.to_owned(),
            fix: fix.to_owned(),
        },
    )
}
