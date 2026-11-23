//! The Effect journal's tables in the run file, and the statements that
//! read and change its entries (ADR 0106): each declaration, sent mark,
//! judgment and person's answer goes through [`apply`], in the writer's
//! transaction, so an entry left by a body no engine runs is judged before
//! the journal answers about it.

use std::time::SystemTime;

use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::run::{RunReport, UncertainEffect};
use hkdf::Hkdf;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension as _, ToSql as _};
use sha2::Sha256;

use super::Reconciliation;
use super::lifecycle::{EffectEvent, EffectState, InvalidEffectTransition, Next, apply};
use crate::run_state::RunStateTables;
use crate::runs::ReportPart;
use crate::stored_time::StoredTime;

/// The Effect journal's tables in every run file: an entry per Effect key
/// of a namespace, the crawl request of a handler's attempt or `""` for
/// the Run, and the Run's secret its idempotency keys derive from.
pub const TABLES: RunStateTables = RunStateTables {
    name: "effect-journal",
    statements: &[
        "CREATE TABLE IF NOT EXISTS effect_journal (entry INTEGER PRIMARY KEY AUTOINCREMENT, \
         namespace TEXT NOT NULL, key TEXT NOT NULL, repeatable INTEGER NOT NULL, \
         state TEXT NOT NULL CHECK (state IN ('open', 'sent', 'done', 'uncertain', 'reconciled')), \
         output TEXT, reason TEXT, first_command TEXT, since TEXT, performed_at TEXT, \
         reconciled_by TEXT, reconciled_at TEXT, \
         UNIQUE (namespace, key), \
         CHECK (state NOT IN ('sent', 'uncertain') OR since IS NOT NULL), \
         CHECK (state <> 'done' OR performed_at IS NOT NULL), \
         CHECK (state <> 'uncertain' OR reason IS NOT NULL), \
         CHECK (state <> 'reconciled' OR (reconciled_by IS NOT NULL AND reconciled_at IS NOT NULL))) STRICT",
        "CREATE TABLE IF NOT EXISTS effect_secret (id INTEGER PRIMARY KEY CHECK (id = 1), secret BLOB NOT NULL) STRICT",
    ],
};

/// The bytes of the Run's secret and of an idempotency key.
pub(super) const SECRET_BYTES: usize = 32;

/// The Run report's uncertain Effects (ADR 0106): each entry left
/// `uncertain`, and each left `sent` by a body that is not repeatable and
/// whose engine is gone, which its next declaration judges uncertain.
pub struct Report;

impl ReportPart for Report {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        let mut statement = connection.prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal \
             WHERE state = 'uncertain' OR (state = 'sent' AND repeatable = 0) ORDER BY entry"
        ))?;
        let found = statement
            .query_map([], Row::read)?
            .map(|row| row.map(|row| row.uncertain()))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !found.is_empty() {
            report
                .uncertain_effects
                .get_or_insert_with(Vec::new)
                .extend(found);
        }
        Ok(())
    }
}

/// What a declaration's write found.
pub(super) enum Declared {
    /// The key's entry was written: run the body.
    Perform {
        entry: i64,
        /// The Run's secret.
        secret: Vec<u8>,
    },
    /// A body returned this output, at this point in time.
    Journal {
        output: Option<String>,
        performed_at: SystemTime,
    },
    /// A person answered "it happened".
    Reconciled { by: String, at: SystemTime },
    /// The Effect is uncertain.
    Uncertain(UncertainEffect),
    /// The transition table refused the declaration.
    Refused(InvalidEffectTransition),
}

/// What a person's answer found.
pub(super) enum Answering {
    Answered(UncertainEffect),
    NotUncertain,
    Refused(InvalidEffectTransition),
}

/// The columns [`Row::read`] reads.
const ROW: &str = "entry, namespace, key, repeatable, state, output, reason, first_command, since, \
                   performed_at, reconciled_by, reconciled_at";

/// An entry of the Effect journal.
pub(super) struct Row {
    pub(super) entry: i64,
    pub(super) namespace: String,
    pub(super) key: String,
    pub(super) repeatable: bool,
    pub(super) state: String,
    pub(super) output: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) first_command: Option<String>,
    pub(super) since: Option<SystemTime>,
    pub(super) performed_at: Option<SystemTime>,
    pub(super) reconciled_by: Option<String>,
    pub(super) reconciled_at: Option<SystemTime>,
}

impl Row {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            entry: row.get(0)?,
            namespace: row.get(1)?,
            key: row.get(2)?,
            repeatable: row.get(3)?,
            state: row.get(4)?,
            output: row.get(5)?,
            reason: row.get(6)?,
            first_command: row.get(7)?,
            since: row.get::<_, Option<StoredTime>>(8)?.map(|at| at.0),
            performed_at: row.get::<_, Option<StoredTime>>(9)?.map(|at| at.0),
            reconciled_by: row.get(10)?,
            reconciled_at: row.get::<_, Option<StoredTime>>(11)?.map(|at| at.0),
        })
    }

    /// Its state in the transition table.
    fn effect_state(&self) -> rusqlite::Result<EffectState> {
        EffectState::of(&self.state, self.repeatable).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                Type::Text,
                format!("`{}` is no state of the Effect journal", self.state).into(),
            )
        })
    }

    /// The uncertain Effect it is, as the Run report lists it: a `sent`
    /// entry is one whose body was cut short.
    pub(super) fn uncertain(&self) -> UncertainEffect {
        UncertainEffect {
            entry: self.entry.to_string(),
            key: Some(self.key.clone()),
            instruction: None,
            crawl_request: (!self.namespace.is_empty()).then(|| self.namespace.clone()),
            reason: self
                .reason
                .clone()
                .unwrap_or_else(|| UncertainReason::Interrupted.as_str().to_owned()),
            first_command: self.first_command.clone(),
            since: self
                .since
                .expect("invariant: the journal's CHECK keeps a sent or uncertain entry's `since`"),
        }
    }
}

/// The entry `entry`, if the journal holds it.
pub(super) fn row(connection: &Connection, entry: i64) -> rusqlite::Result<Option<Row>> {
    connection
        .prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal WHERE entry = ?1"
        ))?
        .query_row([entry], Row::read)
        .optional()
}

/// The entry of `key` in `namespace`, if the journal holds it.
fn keyed(connection: &Connection, namespace: &str, key: &str) -> rusqlite::Result<Option<Row>> {
    connection
        .prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal WHERE namespace = ?1 AND key = ?2"
        ))?
        .query_row((namespace, key), Row::read)
        .optional()
}

/// Judges `row`, left `open` or `sent` by a body no engine runs, as cut
/// short, and answers its state after: the entry `uncertain`, or absent.
/// Any other entry's state is answered unchanged.
fn settled(connection: &Connection, row: &Row) -> rusqlite::Result<EffectState> {
    let state = row.effect_state()?;
    if !matches!(
        state,
        EffectState::Open
            | EffectState::OpenRepeatable
            | EffectState::Sent
            | EffectState::SentRepeatable
    ) {
        return Ok(state);
    }
    match apply(state, EffectEvent::Interrupt) {
        Ok(Next::To(EffectState::Uncertain)) => {
            turn_uncertain(connection, row.entry, UncertainReason::Interrupted)?;
            Ok(EffectState::Uncertain)
        }
        Ok(Next::Dropped) => {
            drop_entry(connection, row.entry)?;
            Ok(EffectState::Absent)
        }
        Ok(Next::To(other)) => Ok(other),
        Ok(Next::Unchanged) | Err(_) => Ok(state),
    }
}

/// Judges each of `entries`, whose bodies the engine cut short at its Run's
/// end, as [`settled`] judges one no engine runs.
pub(super) fn interrupted(connection: &Connection, entries: &[i64]) -> rusqlite::Result<()> {
    for &entry in entries {
        if let Some(found) = row(connection, entry)? {
            settled(connection, &found)?;
        }
    }
    Ok(())
}

/// Declares `key` in `namespace`: judges an entry left by a body no engine
/// runs, then writes the entry, or answers the journal's answer.
pub(super) fn declare(
    connection: &Connection,
    namespace: &str,
    key: &str,
    repeatable: bool,
    candidate: &[u8; SECRET_BYTES],
) -> rusqlite::Result<Declared> {
    let found = keyed(connection, namespace, key)?;
    let state = match &found {
        Some(row) => settled(connection, row)?,
        None => EffectState::Absent,
    };
    let event = if repeatable {
        EffectEvent::DeclareRepeatable
    } else {
        EffectEvent::Declare
    };
    match apply(state, event) {
        Ok(Next::To(_)) => {
            let entry = connection
                .prepare_cached(
                    "INSERT INTO effect_journal (namespace, key, repeatable, state) \
                     VALUES (?1, ?2, ?3, 'open') RETURNING entry",
                )?
                .query_row((namespace, key, repeatable), |row| row.get(0))?;
            connection
                .prepare_cached(
                    "INSERT INTO effect_secret (id, secret) VALUES (1, ?1) ON CONFLICT (id) DO NOTHING",
                )?
                .execute([candidate.as_slice()])?;
            let secret = connection
                .prepare_cached("SELECT secret FROM effect_secret WHERE id = 1")?
                .query_row([], |row| row.get(0))?;
            Ok(Declared::Perform { entry, secret })
        }
        Ok(Next::Unchanged | Next::Dropped) => {
            let row = found.expect("invariant: only an entry the journal holds answers");
            Ok(match state {
                EffectState::Reconciled => Declared::Reconciled {
                    by: row.reconciled_by.unwrap_or_default(),
                    at: row
                        .reconciled_at
                        .expect("invariant: the journal's CHECK keeps a reconciled entry's time"),
                },
                _ => Declared::Journal {
                    output: row.output,
                    performed_at: row
                        .performed_at
                        .expect("invariant: the journal's CHECK keeps a done entry's time"),
                },
            })
        }
        Err(_) if state == EffectState::Uncertain => {
            let row = keyed(connection, namespace, key)?
                .expect("invariant: an uncertain entry is in the journal");
            Ok(Declared::Uncertain(row.uncertain()))
        }
        Err(refused) => Ok(Declared::Refused(refused)),
    }
}

/// Marks each of `entries` sent, before `first_command` at `since`.
pub(super) fn mark_sent(
    connection: &Connection,
    entries: &[i64],
    first_command: &str,
    since: SystemTime,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(
        "UPDATE effect_journal SET state = 'sent', first_command = ?2, since = ?3 \
         WHERE entry = ?1 AND state = 'open'",
    )?;
    let since = StoredTime(since);
    let since = since.to_sql()?;
    for entry in entries {
        statement.execute((entry, first_command, &since))?;
    }
    Ok(())
}

/// Keeps `output` as what the body of `entry` returned, performed at
/// `performed_at`.
pub(super) fn done(
    connection: &Connection,
    entry: i64,
    output: Option<&str>,
    performed_at: SystemTime,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "UPDATE effect_journal SET state = 'done', output = ?2, performed_at = ?3 WHERE entry = ?1",
        )?
        .execute((entry, output, StoredTime(performed_at)))?;
    Ok(())
}

/// Leaves `entry` uncertain for `reason`.
pub(super) fn turn_uncertain(
    connection: &Connection,
    entry: i64,
    reason: UncertainReason,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "UPDATE effect_journal SET state = 'uncertain', reason = ?2 WHERE entry = ?1",
        )?
        .execute((entry, reason.as_str()))?;
    Ok(())
}

/// Takes `entry` out of the journal.
pub(super) fn drop_entry(connection: &Connection, entry: i64) -> rusqlite::Result<()> {
    connection
        .prepare_cached("DELETE FROM effect_journal WHERE entry = ?1")?
        .execute([entry])?;
    Ok(())
}

/// Applies a person's `answer` to `entry`, given `by` at `at`.
pub(super) fn answer_about(
    connection: &Connection,
    entry: i64,
    answer: Reconciliation,
    by: &str,
    at: SystemTime,
) -> rusqlite::Result<Answering> {
    let Some(found) = row(connection, entry)? else {
        return Ok(Answering::NotUncertain);
    };
    if settled(connection, &found)? != EffectState::Uncertain {
        return Ok(Answering::NotUncertain);
    }
    let found = row(connection, entry)?.expect("invariant: an uncertain entry is in the journal");
    let effect = found.uncertain();
    match apply(EffectState::Uncertain, answer.event()) {
        Ok(Next::To(EffectState::Reconciled)) => {
            connection
                .prepare_cached(
                    "UPDATE effect_journal SET state = 'reconciled', reconciled_by = ?2, reconciled_at = ?3 \
                     WHERE entry = ?1",
                )?
                .execute((entry, by, StoredTime(at)))?;
        }
        Ok(Next::Dropped) => drop_entry(connection, entry)?,
        Ok(Next::To(_) | Next::Unchanged) => {}
        Err(refused) => return Ok(Answering::Refused(refused)),
    }
    Ok(Answering::Answered(effect))
}

/// The idempotency key of `key` in `namespace`: HKDF-SHA256 of the Run's
/// secret, so it reveals neither the Run nor the key.
pub(super) fn idempotency_key(secret: &[u8], namespace: &str, key: &str) -> String {
    let mut info = b"goodfetch effect idempotency key\0".to_vec();
    info.extend_from_slice(namespace.as_bytes());
    info.push(0);
    info.extend_from_slice(key.as_bytes());
    let mut okm = [0_u8; SECRET_BYTES];
    Hkdf::<Sha256>::new(None, secret)
        .expand(&info, &mut okm)
        .expect("invariant: 32 bytes is a valid HKDF-SHA256 output length");
    data_encoding::BASE64URL_NOPAD.encode(&okm)
}
