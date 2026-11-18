//! The Limits a program gives a Run at its start (ADRs 0041, 0080 and
//! 0102): each a Client's row on the Run's own `run` scope, with no
//! period, and the values given when the Run began, kept in a table of
//! their own since a person's raise rewrites the rows. A start that names
//! the Run again lowers its Limits and never raises them: a value equal to
//! the one it began with changes nothing, so a person's raise stands; one
//! at or below the Limit as it stands adds a row beside it, the tightest
//! holding; one above it is refused, and nothing is written.

use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{Limit, LimitRaiseRefusedDetails, RUN, RunLimit};
use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension as _};

use super::{Ledger, LedgerError, TARGET, amount, insert_limit};
use crate::sqlite_store::immediate;

/// What a start's Limits do to its Run's: refused when one is above the
/// Limit as it stands.
pub type Given = Result<(), LimitRaiseRefusedDetails>;

impl Ledger {
    /// Whether `given`, a start's Limits on the Run `run_id`, raise none of
    /// its Limits as they stand, writing nothing. A ledger no engine
    /// created holds none, and stays uncreated.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    pub fn check_run_limits(&self, run_id: &str, given: &[RunLimit]) -> Result<Given, LedgerError> {
        if given.is_empty() {
            return Ok(Ok(()));
        }
        let checked = self.on(&mut None, false, |connection| {
            Ok(added(connection, run_id, given, false)?.map(drop))
        })?;
        Ok(checked.unwrap_or(Ok(())))
    }

    /// Gives the Run `run_id` the Limits `given` at its start, in one
    /// transaction: when the start `began` the Run, each as a Client's
    /// Limit on its `run` scope and as the value it began with; else each
    /// as ADR 0102's resume rule says, the whole start refused, with
    /// nothing written, when one is above the Limit as it stands.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn give_run_limits(
        &self,
        run_id: &str,
        given: &[RunLimit],
        began: bool,
        now: Timestamp,
    ) -> Result<Given, LedgerError> {
        if given.is_empty() {
            return Ok(Ok(()));
        }
        let written = self.on_created(&mut None, |connection| {
            immediate(connection, |transaction| {
                let adding = match added(transaction, run_id, given, began)? {
                    Ok(adding) => adding,
                    Err(refused) => return Ok(Err(refused)),
                };
                if began {
                    transaction
                        .prepare_cached("DELETE FROM run_limits WHERE run_id = ?1")?
                        .execute([run_id])?;
                    for limit in given {
                        transaction
                            .prepare_cached(
                                "INSERT INTO run_limits (run_id, usage_kind, currency, amount)
                                 VALUES (?1, ?2, ?3, ?4)",
                            )?
                            .execute((
                                run_id,
                                &limit.usage_kind,
                                &limit.currency,
                                amount(limit.amount),
                            ))?;
                    }
                }
                for limit in &adding {
                    insert_limit(transaction, &run_limit(run_id, limit), None, now)?;
                }
                Ok(Ok(adding.len()))
            })
        })?;
        Ok(written.map(|added| {
            if added > 0 {
                self.changed();
                tracing::trace!(target: TARGET, transition = "set_limit", run_id, added, "a start gave its Run Limits");
            }
        }))
    }
}

/// The Limits of `given` that add a row on the Run `run_id`'s scope: all
/// when the start `began` the Run; else each that differs from the value
/// the Run began with and is below the Limit as it stands, or that no
/// Limit stands for. The first above the Limit as it stands refuses them
/// all.
fn added<'a>(
    connection: &Connection,
    run_id: &str,
    given: &'a [RunLimit],
    began: bool,
) -> rusqlite::Result<Result<Vec<&'a RunLimit>, LimitRaiseRefusedDetails>> {
    if began {
        return Ok(Ok(given.iter().collect()));
    }
    let mut adding = Vec::new();
    for limit in given {
        let began_with: Option<i64> = connection
            .prepare_cached(
                "SELECT amount FROM run_limits WHERE run_id = ?1 AND usage_kind = ?2 AND currency IS ?3",
            )?
            .query_row((run_id, &limit.usage_kind, &limit.currency), |row| {
                row.get(0)
            })
            .optional()?;
        if began_with == Some(amount(limit.amount)) {
            continue;
        }
        let standing: Option<i64> = connection
            .prepare_cached(
                "SELECT MIN(amount) FROM limits
                 WHERE scope_kind = ?1 AND scope_id = ?2 AND usage_kind = ?3 AND currency IS ?4
                   AND period IS NULL",
            )?
            .query_row((RUN, run_id, &limit.usage_kind, &limit.currency), |row| {
                row.get(0)
            })?;
        let given_amount = amount(limit.amount);
        match standing {
            Some(standing) if given_amount > standing => {
                return Ok(Err(LimitRaiseRefusedDetails {
                    usage_kind: limit.usage_kind.clone(),
                    currency: limit.currency.clone(),
                    given: limit.amount,
                    standing: u64::try_from(standing).unwrap_or_default(),
                }));
            }
            Some(standing) if given_amount == standing => {}
            Some(_) | None => adding.push(limit),
        }
    }
    Ok(Ok(adding))
}

/// The Client's Limit `given` names on the Run `run_id`'s scope.
fn run_limit(run_id: &str, given: &RunLimit) -> Limit {
    Limit {
        scope: LimitScope {
            kind: RUN.to_owned(),
            id: run_id.to_owned(),
        },
        usage_kind: given.usage_kind.clone(),
        currency: given.currency.clone(),
        period: None,
        amount: given.amount,
        must_understand: true,
        set_by: LimitSetter::Client,
    }
}
