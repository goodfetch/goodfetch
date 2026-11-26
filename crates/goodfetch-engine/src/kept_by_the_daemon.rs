//! The guard of the calls a major's Daemon keeps, such as `schedule.*`
//! (spec 589): on an Attached engine that reaches its Daemon, each call is
//! sent to that Daemon, which the engine starts when none runs, and the
//! Daemon's answer is the call's; elsewhere the call's handler answers.

use std::sync::Arc;

use serde_json::Value;

use crate::composition::{Answered, Guard};

/// The guard a contribution whose calls the Daemon keeps adds to them: a
/// Daemon, and an engine that cannot start one, such as a Linked engine,
/// lets the handler answer.
#[must_use]
pub fn kept_by_the_daemon() -> Guard {
    Arc::new(|operation, params, _read, context| {
        let daemon = Arc::clone(context.engine().daemon()?);
        let read = params.read::<Value>();
        let tasks = Arc::clone(context.engine().owner());
        let context = context.clone();
        Some(Box::pin(async move {
            let params = read?;
            let result = daemon
                .forward(
                    tasks,
                    operation,
                    params,
                    (context.surface(), context.form()),
                )
                .await?;
            Answered::encode(&context, &result)
        }))
    })
}
