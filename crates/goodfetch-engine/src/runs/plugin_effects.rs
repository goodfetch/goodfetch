//! Run-backed [`PluginEffectJournal`] (ADRs 0010, 0033 and 0106; issue 845):
//! writes each Plugin Effect (a changing `page.*` action, a logged-in
//! navigation or approved `page_script`, or a non-`GET`/`HEAD` `net.fetch`
//! toward the run's sites) into the Run's `effect_journal` table in
//! `run.sqlite` (synced before the action goes out) and closes it `done` or
//! `uncertain`.

use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::INTERNAL;
use goodfetch_protocol::effect::UncertainReason;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::plugin_kinds::{PluginEffectEntry, PluginEffectJournal};
use crate::run_state::RunState;
use crate::stored_time::StoredTime;

/// Persists Plugin Effects into one live Run's `effect_journal` table.
#[derive(Clone)]
pub struct RunEffectJournal {
    state: RunState,
    clock: Clock,
    next: Arc<tokio::sync::Mutex<u64>>,
}

impl fmt::Debug for RunEffectJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunEffectJournal")
            .finish_non_exhaustive()
    }
}

impl RunEffectJournal {
    /// Creates an Effect journal writer backed by `state` and `clock`.
    #[must_use]
    pub fn new(state: RunState, clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            state,
            clock,
            next: Arc::new(tokio::sync::Mutex::new(1)),
        })
    }
}

impl PluginEffectJournal for RunEffectJournal {
    fn begin<'a>(
        &'a self,
        plugin: &'a str,
        method: &'a str,
        target: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PluginEffectEntry>, CallError>> {
        let now = StoredTime(std::time::SystemTime::from(self.clock.timestamp()));
        let state = self.state.clone();
        let clock = self.clock;
        let next = Arc::clone(&self.next);
        let command = format!("plugin:{plugin}:{method} {target}");
        let plugin = plugin.to_owned();
        let method = method.to_owned();
        Box::pin(async move {
            let seq = {
                let mut guard = next.lock().await;
                let seq = *guard;
                *guard = guard.saturating_add(1);
                seq
            };
            let key = format!("plugin:{plugin}:{seq}:{method}");
            let entry: i64 = state
                .write(move |connection| {
                    connection
                        .prepare_cached(
                            "INSERT INTO effect_journal \
                             (namespace, key, repeatable, state, first_command, since) \
                             VALUES ('', ?1, 0, 'sent', ?2, ?3) RETURNING entry",
                        )?
                        .query_row((key, command, now), |row| row.get(0))
                })
                .await
                .map_err(|error| {
                    CallError::new(
                        &INTERNAL,
                        format!("Plugin Effect journal write failed: {error}"),
                        &(),
                    )
                })?;
            Ok(Box::new(RunEffectEntry {
                entry,
                state,
                clock,
            }) as Box<dyn PluginEffectEntry>)
        })
    }
}

struct RunEffectEntry {
    entry: i64,
    state: RunState,
    clock: Clock,
}

impl PluginEffectEntry for RunEffectEntry {
    fn done(self: Box<Self>) -> BoxFuture<'static, Result<(), CallError>> {
        let entry = self.entry;
        let now = StoredTime(std::time::SystemTime::from(self.clock.timestamp()));
        let state = self.state;
        Box::pin(async move {
            state
                .write(move |connection| {
                    connection
                        .prepare_cached(
                            "UPDATE effect_journal SET state = 'done', performed_at = ?1 \
                             WHERE entry = ?2",
                        )?
                        .execute((now, entry))?;
                    Ok(())
                })
                .await
                .map_err(|error| {
                    CallError::new(
                        &INTERNAL,
                        format!("Plugin Effect journal close failed: {error}"),
                        &(),
                    )
                })
        })
    }

    fn uncertain(self: Box<Self>, reason: UncertainReason) -> BoxFuture<'static, ()> {
        let entry = self.entry;
        let reason = reason.as_str().to_owned();
        let state = self.state;
        Box::pin(async move {
            if let Err(error) = state
                .write(move |connection| {
                    connection
                        .prepare_cached(
                            "UPDATE effect_journal SET state = 'uncertain', reason = ?1 \
                             WHERE entry = ?2",
                        )?
                        .execute((reason, entry))?;
                    Ok(())
                })
                .await
            {
                tracing::warn!(
                    entry,
                    %error,
                    "could not mark Plugin Effect entry uncertain"
                );
            }
        })
    }
}
