//! The delivery ledger (ADR 0027): one row per event and channel in the
//! Run's own run file, so it moves with the Run and a resumed Run, on this
//! engine or another, finds what was sent. An attempt is claimed, in a
//! synced commit, before anything is sent; a claim of an earlier run epoch
//! that never settled was in flight when its Run stopped, and is settled
//! `uncertain`, never sent again. A delivered, failed or uncertain event
//! is never claimed again, so a status change notifies once.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::run_state::{RunState, RunStateError, RunStateTables};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::message::EventId;

#[cfg(test)]
mod tests;

/// The ledger's table, which every run file holds.
pub const TABLES: RunStateTables = RunStateTables {
    name: "notification",
    statements: &["CREATE TABLE IF NOT EXISTS notification_deliveries (
        event_id TEXT NOT NULL,
        channel TEXT NOT NULL,
        state TEXT NOT NULL,
        attempt INTEGER NOT NULL,
        run_epoch INTEGER NOT NULL,
        next_attempt_at INTEGER,
        failure TEXT,
        PRIMARY KEY (event_id, channel)
    ) STRICT"],
};

/// What the ledger holds of one event on one channel. An attempt is
/// counted from 0, the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `attempt` is being sent by the engine of `run_epoch`.
    Sending {
        /// The attempt.
        attempt: u32,
        /// The run epoch of the engine that claimed it.
        run_epoch: u64,
    },
    /// `attempt` failed for a while and is sent at `at`.
    Waiting {
        /// The next attempt.
        attempt: u32,
        /// When it is sent, to the millisecond.
        at: SystemTime,
    },
    /// `attempt` delivered it.
    Delivered {
        /// The attempt that delivered it.
        attempt: u32,
    },
    /// `attempt` failed for good, with the failure's name.
    Failed {
        /// The last attempt.
        attempt: u32,
        /// Its failure's name, such as `gone`.
        failure: String,
    },
    /// `attempt` was in flight when its Run stopped, so whether it arrived
    /// is unknown: it is never sent again.
    Uncertain {
        /// The attempt in flight.
        attempt: u32,
    },
}

/// What a claim of an attempt found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The attempt is this engine's to send.
    Claimed,
    /// The ledger already held the event: delivered, failed, uncertain, or
    /// an attempt in flight on this engine.
    Held(Entry),
}

/// How a claimed attempt ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// It delivered the event.
    Delivered,
    /// It failed for a while: `attempt` is sent at `at`.
    RetryAt {
        /// The next attempt.
        attempt: u32,
        /// When it is sent.
        at: SystemTime,
    },
    /// It failed for good, with the failure's name.
    Failed(&'static str),
}

/// The delivery ledger of one Run.
#[derive(Clone, Debug)]
pub struct Ledger {
    state: RunState,
}

impl Ledger {
    /// The ledger in `state`, the Run's run file, which holds [`TABLES`].
    pub fn new(state: RunState) -> Self {
        Self { state }
    }

    /// What it holds of `event` on `channel`.
    ///
    /// # Errors
    ///
    /// As [`RunState::read`].
    pub async fn entry(
        &self,
        event: &EventId,
        channel: &str,
    ) -> Result<Option<Entry>, RunStateError> {
        let key = (event.as_str().to_owned(), channel.to_owned());
        self.state
            .read(move |connection| read(connection, &key.0, &key.1))
            .await
    }

    /// Claims `attempt` of `event` on `channel` for this engine, in a
    /// synced commit: the first attempt of an event it holds nothing of, or
    /// the attempt it waits to send. A claim an earlier run epoch left
    /// unsettled is settled `uncertain` instead.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    pub async fn claim(
        &self,
        event: &EventId,
        channel: &str,
        attempt: u32,
    ) -> Result<Claim, RunStateError> {
        let key = (event.as_str().to_owned(), channel.to_owned());
        let epoch = self.state.epoch();
        self.state
            .write(move |connection| {
                let (event, channel) = (key.0.as_str(), key.1.as_str());
                let entry = read(connection, event, channel)?;
                let claimed = match &entry {
                    None => true,
                    Some(Entry::Waiting { attempt: next, .. }) => *next == attempt,
                    Some(Entry::Sending { attempt, run_epoch }) if *run_epoch < epoch => {
                        connection
                            .prepare_cached(
                                "UPDATE notification_deliveries SET state = 'uncertain'
                                 WHERE event_id = ?1 AND channel = ?2",
                            )?
                            .execute(params![event, channel])?;
                        return Ok(Claim::Held(Entry::Uncertain { attempt: *attempt }));
                    }
                    Some(_) => false,
                };
                if !claimed {
                    return Ok(Claim::Held(
                        entry.expect("invariant: only a held row is not claimed"),
                    ));
                }
                connection
                    .prepare_cached(
                        "INSERT INTO notification_deliveries
                             (event_id, channel, state, attempt, run_epoch)
                         VALUES (?1, ?2, 'sending', ?3, ?4)
                         ON CONFLICT (event_id, channel) DO UPDATE SET
                             state = 'sending', attempt = ?3, run_epoch = ?4,
                             next_attempt_at = NULL",
                    )?
                    .execute(params![event, channel, attempt, to_column(epoch)])?;
                Ok(Claim::Claimed)
            })
            .await
    }

    /// Settles the attempt of `event` on `channel` this engine claimed, as
    /// `settled`; an attempt it did not claim is left as it is.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    pub async fn settle(
        &self,
        event: &EventId,
        channel: &str,
        settled: Settled,
    ) -> Result<(), RunStateError> {
        let key = (event.as_str().to_owned(), channel.to_owned());
        let epoch = to_column(self.state.epoch());
        self.state
            .write(move |connection| {
                let (state, next, at, failure) = match settled {
                    Settled::Delivered => ("delivered", None, None, None),
                    Settled::RetryAt { attempt, at } => {
                        ("waiting", Some(attempt), Some(milliseconds(at)), None)
                    }
                    Settled::Failed(failure) => ("failed", None, None, Some(failure)),
                };
                connection
                    .prepare_cached(
                        "UPDATE notification_deliveries
                         SET state = ?3, attempt = coalesce(?4, attempt),
                             next_attempt_at = ?5, failure = ?6
                         WHERE event_id = ?1 AND channel = ?2
                           AND state = 'sending' AND run_epoch = ?7",
                    )?
                    .execute(params![key.0, key.1, state, next, at, failure, epoch])?;
                Ok(())
            })
            .await
    }
}

/// The row of `event` on `channel`.
fn read(connection: &Connection, event: &str, channel: &str) -> rusqlite::Result<Option<Entry>> {
    connection
        .prepare_cached(
            "SELECT state, attempt, run_epoch, next_attempt_at, failure
             FROM notification_deliveries WHERE event_id = ?1 AND channel = ?2",
        )?
        .query_row(params![event, channel], |row| {
            let state: String = row.get(0)?;
            let attempt: u32 = row.get(1)?;
            Ok(match state.as_str() {
                "sending" => Entry::Sending {
                    attempt,
                    run_epoch: u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                },
                "waiting" => Entry::Waiting {
                    attempt,
                    at: UNIX_EPOCH
                        + Duration::from_millis(
                            u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
                        ),
                },
                "delivered" => Entry::Delivered { attempt },
                "failed" => Entry::Failed {
                    attempt,
                    failure: row.get(4)?,
                },
                _ => Entry::Uncertain { attempt },
            })
        })
        .optional()
}

/// `epoch` as SQLite's signed integer.
fn to_column(epoch: u64) -> i64 {
    i64::try_from(epoch).unwrap_or(i64::MAX)
}

/// `at` in milliseconds since the Unix epoch, as the ledger keeps it.
fn milliseconds(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}
