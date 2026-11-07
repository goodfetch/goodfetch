//! The engine's side of the Usage ledger (ADR 0041): where each of its runs
//! stands among the Limits' scopes, what a run journals of its Slices, and
//! what a reclaimer finds of them. Each charge records, in the run's own
//! state and under its run epoch, what the run has used of each Slice,
//! before it uses it; an engine that opens the ledger tests each holder's
//! writer lock, and reads that journal of a run whose writer is gone or
//! holds a later epoch. Its calls block: callers run them under their
//! owner's blocking policy.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, RUN_STATE_DURABILITY, RunId};
use goodfetch_protocol::settings::WeakenedSetting;
use goodfetch_protocol::usage::{CONNECTION, DAEMON, INSTALL, PERSON};
use jiff::Timestamp;
use rusqlite::Connection;

use super::ConnectionId;
use super::folder::open_reader;
use super::writer::{Writer, test_writer};
use crate::clock::Clock;
use crate::engine_context::CALLER;
use crate::ledger::{Charging, Holder, Ledger, Placement, Probed, SliceId, SliceSizing};
use crate::tasks::TaskOwner;

/// What every run of one engine charges against: the user's Usage ledger,
/// and the scopes each run of this engine stands in.
#[derive(Clone, Debug)]
pub(crate) struct Limits {
    /// Where its ledger calls run, which the engine's Runs wait for.
    pub(crate) owner: Arc<TaskOwner>,
    /// The user's Usage ledger.
    pub(crate) ledger: Ledger,
    /// Where each Run's charges count their work (ADR 0140).
    pub(crate) work: CountedWork,
    /// The engine's clock, whose wall time places each charge in its period.
    pub(crate) clock: Clock,
    /// The engine's Product.
    pub(crate) product: Product,
    /// What this engine's Client connections' IDs start with, drawn once per
    /// engine so that no two engines' Connections share one.
    pub(crate) instance: String,
    /// The major's name of the Daemon this engine is, if it is one.
    pub(crate) daemon: Option<String>,
}

impl Limits {
    /// The wall time now.
    pub(crate) fn now(&self) -> Timestamp {
        Timestamp::try_from(self.clock.wall()).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    /// The charging of the Run `run_id` under its run epoch `epoch`, its
    /// state in `folder`, placed at `placement`.
    pub(crate) fn charging(
        &self,
        run_id: &RunId,
        folder: &Path,
        epoch: u64,
        placement: Placement,
    ) -> Charging {
        Charging::new(
            self.ledger.clone().counting(self.work.clone()),
            Holder {
                run_id: run_id.as_str().to_owned(),
                epoch,
                folder: folder.to_owned(),
            },
            placement,
            SliceSizing::default(),
        )
    }

    /// The scope of the Client connection `connection`: the one Connection
    /// of an Attached engine when none.
    pub(crate) fn connection(&self, connection: Option<ConnectionId>) -> LimitScope {
        LimitScope {
            kind: CONNECTION.to_owned(),
            id: format!("{}-{}", self.instance, connection.unwrap_or(0)),
        }
    }

    /// Where a run started on `connection` stands: the install, its person,
    /// its Client connection and, on a Daemon, the Daemon.
    pub(crate) fn placement(&self, connection: Option<ConnectionId>) -> Placement {
        let mut scopes = vec![
            LimitScope {
                kind: INSTALL.to_owned(),
                id: String::new(),
            },
            LimitScope {
                kind: PERSON.to_owned(),
                id: CALLER.to_owned(),
            },
            self.connection(connection),
        ];
        if let Some(major) = &self.daemon {
            scopes.push(LimitScope {
                kind: DAEMON.to_owned(),
                id: major.clone(),
            });
        }
        Placement {
            product: self.product,
            scopes,
        }
    }
}

/// Records, under `epoch`, what the run has now used of each Slice, and
/// that it is *uncapped* once it is.
pub(super) fn journal(
    connection: &Connection,
    epoch: u64,
    used: &[(SliceId, u64)],
    uncapped: bool,
) -> rusqlite::Result<()> {
    if uncapped {
        connection
            .prepare_cached("UPDATE run SET uncapped = 1")
            .and_then(|mut cached| cached.execute([]))?;
    }
    for (slice, used) in used {
        connection
            .prepare_cached(
                "INSERT INTO usage (slice_id, epoch, used) VALUES (?1, ?2, ?3)
             ON CONFLICT (slice_id, epoch) DO UPDATE SET used = excluded.used",
            )
            .and_then(|mut cached| {
                cached.execute((
                    slice,
                    i64::try_from(epoch).unwrap_or(i64::MAX),
                    i64::try_from(*used).unwrap_or(i64::MAX),
                ))
            })?;
    }
    Ok(())
}

/// What a reclaim of the Slices the Run held before run epoch `epoch` may
/// have charged past its journal, once one of them charged a Limit; none
/// when nothing could be over-charged.
///
/// # Errors
///
/// When the file cannot be read.
pub(super) fn reclaimed_beyond(
    connection: &Connection,
    epoch: u64,
) -> rusqlite::Result<Option<OverCharge>> {
    let charged: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM usage WHERE epoch < ?1)",
        [i64::try_from(epoch).unwrap_or(i64::MAX)],
        |row| row.get(0),
    )?;
    if !charged {
        return Ok(None);
    }
    let (declared, weakened): (i64, String) =
        connection.query_row("SELECT deferred_charges, weakened FROM run", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    Ok(match beyond(declared, &weakened) {
        0 => None,
        u64::MAX => Some(OverCharge::WholeSlices),
        bound => Some(OverCharge::UpTo(bound)),
    })
}

/// What a reclaim may have charged of each Slice past a Run's journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverCharge {
    /// The whole Slice, at a Durability level that syncs no commit.
    WholeSlices,
    /// Up to this much more: the bound the run file declares at `full`.
    UpTo(u64),
}

/// What a run file whose `deferred_charges` is `declared` and whose
/// `weakened` listing is `weakened` may have charged of each Slice past its
/// journal: every Slice whole when its Durability level is weakened, which
/// syncs no commit, else the bound it declares.
fn beyond(declared: i64, weakened: &str) -> u64 {
    let unsynced =
        serde_json::from_str::<Vec<WeakenedSetting>>(weakened).map_or(true, |weakened| {
            weakened
                .iter()
                .any(|setting| setting.key == RUN_STATE_DURABILITY.name)
        });
    if unsynced {
        u64::MAX
    } else {
        u64::try_from(declared).unwrap_or_default()
    }
}

/// What a reclaimer finds of the run `holder` names, its state in `file`
/// of its folder, opened through `vfs`.
pub(super) fn probe(holder: &Holder, file: &str, vfs: Option<&str>) -> Probed {
    let writer = test_writer(&holder.folder);
    let read = read(&holder.folder.join(file), vfs, holder.epoch);
    match (writer, read) {
        (Ok(Writer::Live), Ok(Read { now, used, beyond })) if now > holder.epoch => {
            Probed::Gone { used, beyond }
        }
        (Ok(Writer::Live) | Err(_), _) => Probed::Live,
        (Ok(Writer::None), Ok(Read { used, beyond, .. })) => Probed::Gone { used, beyond },
        (Ok(Writer::None), Err(())) => Probed::Unreadable,
    }
}

/// What a reclaimer reads of a run file.
struct Read {
    /// Its run epoch now.
    now: u64,
    /// What the run journaled of each Slice under the epoch asked for.
    used: BTreeMap<SliceId, u64>,
    /// What it may have charged of each Slice past that in writes a power
    /// cut took (ADR 0138's note of #1926): the bound its file declares
    /// for its deferred claims at `full`, and every Slice whole at a lower
    /// Durability level, which commits nothing synced.
    beyond: u64,
}

/// The run file's run epoch now, what the run journaled under `epoch` and
/// what it may have charged beyond it.
fn read(file: &Path, vfs: Option<&str>, epoch: u64) -> Result<Read, ()> {
    let read = || -> rusqlite::Result<Option<Read>> {
        let Some(connection) = open_reader(file, vfs)? else {
            return Ok(None);
        };
        let (now, declared, weakened): (i64, i64, String) = connection
            .prepare_cached("SELECT epoch, deferred_charges, weakened FROM run")
            .and_then(|mut cached| {
                cached.query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            })?;
        let beyond = beyond(declared, &weakened);
        let mut statement =
            connection.prepare_cached("SELECT slice_id, used FROM usage WHERE epoch = ?1")?;
        let used = statement
            .query_map([i64::try_from(epoch).unwrap_or(i64::MAX)], |row| {
                Ok((row.get::<_, SliceId>(0)?, row.get::<_, i64>(1)?))
            })?
            .map(|row| row.map(|(slice, used)| (slice, u64::try_from(used).unwrap_or_default())))
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(Read {
            now: u64::try_from(now).unwrap_or_default(),
            used,
            beyond,
        }))
    };
    read().ok().flatten().ok_or(())
}
