//! Detaching a Run (ADRs 0078 and 0102): a `detached` start's Run outlives
//! the Connection that starts it, so only a Run with no handler in the
//! program, started on a Daemon, detaches. A Run whose start gives it a
//! handler in the program would wait on a Connection gone, and an Attached
//! or linked engine ends with its Connection; either start fails with
//! `detach_unsupported`, which names what kept the Run from detaching.

use goodfetch_protocol::run::{DETACH_UNSUPPORTED, DetachUnsupportedDetails};

use crate::call_error::CallError;
use crate::composition::CallContext;

/// What gives a Run a handler in the program: the key of its start that
/// chose it, and how to start the Run with none.
#[derive(Debug)]
pub struct InProgram {
    /// The key.
    pub key: &'static str,
    /// How to start the Run with no handler in the program.
    pub fix: String,
}

/// Checks that the Run a `detached` start asks for may outlive its
/// Connection, `in_program` naming what gives it a handler in the program,
/// if anything does.
///
/// # Errors
///
/// `detach_unsupported` naming `in_program`'s key when it is some, else
/// `engine` when the Connection's engine is no Daemon.
pub fn detachable(in_program: Option<InProgram>, context: &CallContext) -> Result<(), CallError> {
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
            "Connect to the Daemon, `engine: daemon`, to start the Run detached, or start it without `detached`.".to_owned(),
        ));
    }
    Ok(())
}

fn refused(message: String, key: &str, fix: String) -> CallError {
    CallError::new(
        &DETACH_UNSUPPORTED,
        message,
        &DetachUnsupportedDetails {
            key: key.to_owned(),
            fix,
        },
    )
}
