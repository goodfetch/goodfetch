//! A crawl request's Payload (ADR 0082): the data a program attaches to a
//! request for its handler, kept in the Frontier's own table, apart from
//! the engine's fields (its URL, Label, depth and attempts), and never part
//! of its identity. A Label the crawl declares with a JSON Schema types its
//! Payloads: one that does not fit, a missing one read as `null`, is
//! refused at its enqueue with `payload_invalid` naming each issue's path;
//! so is one whose JSON, written compactly, is longer than
//! `crawl.payload.max_size`, which names the setting, since the Payload
//! lives in the Frontier and in every Checkpoint. A Label declared without
//! a schema, or not declared, takes any Payload under the cap.
//!
//! Each declared Label is recorded in the run file when the Run begins,
//! with its schema and the schema's hash, the SHA-256 of its JSON with keys
//! in order, which a resume compares: a Label now declared with another
//! schema has every pending Payload checked against it before any work,
//! and one that fails refuses the resume with `payload_schema_changed`,
//! naming the Label, how many fail out of how many, and the first failure;
//! a Payload is never migrated. The cap is recorded when the Run begins and
//! again when it resumes under its new settings.

use std::collections::BTreeMap;

use data_encoding::HEXLOWER;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_PAYLOAD_MAX_SIZE, PAYLOAD_INVALID, PayloadInvalidDetails, PayloadIssue,
    PayloadSchemaChangedDetails,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Each crawl request's Payload, by the request's ID; each declared Label,
/// with its Payload's JSON Schema and that schema's hash, both none for a
/// Label without one; and the cap, one row.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-payload",
    statements: &[
        "CREATE TABLE IF NOT EXISTS payloads (
            request TEXT NOT NULL PRIMARY KEY,
            payload TEXT NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS payload_labels (
            label TEXT NOT NULL PRIMARY KEY,
            schema TEXT,
            hash TEXT
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS payload_cap (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_size INTEGER NOT NULL
        ) STRICT",
    ],
};

/// What a crawl's Payloads are checked against: the cap, and each declared
/// Label's JSON Schema, none for a Label declared without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadRules {
    /// `crawl.payload.max_size`, in bytes.
    max_size: u64,
    /// Each declared Label's Payload schema.
    labels: BTreeMap<String, Option<Value>>,
}

impl PayloadRules {
    /// The rules of `settings` and the crawl's declared `labels`.
    ///
    /// # Errors
    ///
    /// `invalid_params` naming a Label whose schema is no JSON Schema.
    #[expect(
        clippy::disallowed_methods,
        reason = "the declaration's one check of its schema, as the crawl starts (ADR 0140)"
    )]
    pub fn of(
        settings: &Resolved,
        labels: &BTreeMap<String, LabelDeclaration>,
    ) -> Result<Self, CallError> {
        for (label, declaration) in labels {
            if let Some(schema) = &declaration.payload
                && let Err(error) = jsonschema::validator_for(schema)
            {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("The Label `{label}`'s Payload schema is no JSON Schema: {error}"),
                ));
            }
        }
        Ok(Self {
            max_size: max_size(settings),
            labels: labels
                .iter()
                .map(|(label, declaration)| (label.clone(), declaration.payload.clone()))
                .collect(),
        })
    }

    /// Records the rules in the run file `connection` holds, as the Run
    /// begins.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        record_cap(connection, self.max_size)?;
        let mut insert = connection.prepare_cached(
            "INSERT INTO payload_labels (label, schema, hash) VALUES (?1, ?2, ?3)
             ON CONFLICT (label) DO UPDATE SET schema = excluded.schema, hash = excluded.hash",
        )?;
        for (label, schema) in &self.labels {
            insert.execute((
                label,
                schema.as_ref().map(Value::to_string),
                schema.as_ref().map(hash),
            ))?;
        }
        Ok(())
    }

    /// The rules the run file `connection` holds.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let max_size = connection
            .prepare_cached("SELECT max_size FROM payload_cap WHERE id = 1")
            .and_then(|mut cached| cached.query_row([], |row| row.get::<_, i64>(0)))
            .optional()?
            .map_or_else(built_in_max_size, |max_size| {
                u64::try_from(max_size).unwrap_or(0)
            });
        let labels = connection
            .prepare_cached("SELECT label, schema FROM payload_labels")?
            .query_map([], |row| {
                let schema: Option<String> = row.get(1)?;
                Ok((
                    row.get(0)?,
                    schema.and_then(|schema| serde_json::from_str(&schema).ok()),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self { max_size, labels })
    }

    /// Whether `payload`, enqueued under `label`, may be added.
    ///
    /// # Errors
    ///
    /// `payload_invalid` when its JSON is longer than the cap, or it does
    /// not fit its Label's schema.
    pub fn check(
        &self,
        label: &str,
        payload: Option<&Value>,
        work: &CountedWork,
    ) -> Result<(), CallError> {
        if let Some(payload) = payload {
            work.count(Row::PayloadSerializations);
            let size = u64::try_from(payload.to_string().len()).unwrap_or(u64::MAX);
            if size > self.max_size {
                return Err(CallError::new(
                    &PAYLOAD_INVALID,
                    format!(
                        "The Payload of a `{label}` request is {size} bytes of JSON, more than `{}`'s {} bytes, so it was not added: put large content in the Key-value store and carry its key in the Payload.",
                        CRAWL_PAYLOAD_MAX_SIZE.name, self.max_size
                    ),
                    &PayloadInvalidDetails {
                        label: label.to_owned(),
                        issues: Vec::new(),
                        limit: Some(CRAWL_PAYLOAD_MAX_SIZE.name.to_owned()),
                        size: Some(size),
                        max_size: Some(self.max_size),
                    },
                ));
            }
        }
        let Some(Some(schema)) = self.labels.get(label) else {
            return Ok(());
        };
        work.count(Row::PayloadCompilations);
        work.count(Row::PayloadValidations);
        let issues = issues(schema, payload);
        if issues.is_empty() {
            return Ok(());
        }
        Err(CallError::new(
            &PAYLOAD_INVALID,
            format!(
                "The Payload of a `{label}` request does not fit the Label's schema, so it was not added: {}",
                described(&issues)
            ),
            &PayloadInvalidDetails {
                label: label.to_owned(),
                issues,
                limit: None,
                size: None,
                max_size: None,
            },
        ))
    }

    /// The first Label whose schema these rules, a resume's, change from
    /// the one the run file `connection` holds, and whose pending requests'
    /// Payloads fail it: those `pending` lists for a Label, by ID, in
    /// order. `None` when every pending Payload fits.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn schema_changed(
        &self,
        connection: &Connection,
        mut pending: impl FnMut(&str) -> rusqlite::Result<Vec<String>>,
    ) -> rusqlite::Result<Option<PayloadSchemaChangedDetails>> {
        let mut recorded =
            connection.prepare_cached("SELECT hash FROM payload_labels WHERE label = ?1")?;
        for (label, schema) in &self.labels {
            let Some(schema) = schema else {
                continue;
            };
            let kept: Option<String> = recorded
                .query_row([label], |row| row.get(0))
                .optional()?
                .flatten();
            if kept.as_deref() == Some(hash(schema).as_str()) {
                continue;
            }
            let requests = pending(label)?;
            let mut failing = 0_u64;
            let mut first = None;
            for request in &requests {
                let payload = of_request(connection, request)?;
                let issues = issues(schema, payload.as_ref());
                if !issues.is_empty() {
                    failing += 1;
                    first.get_or_insert_with(|| (request.clone(), issues));
                }
            }
            if let Some((crawl_request_id, issues)) = first {
                return Ok(Some(PayloadSchemaChangedDetails {
                    label: label.clone(),
                    failing,
                    pending: u64::try_from(requests.len()).unwrap_or(u64::MAX),
                    crawl_request_id,
                    issues,
                }));
            }
        }
        Ok(None)
    }
}

/// Records `crawl.payload.max_size` of `settings` in the run file
/// `connection` holds, as the Run resumes under its new settings.
///
/// # Errors
///
/// When the run file cannot be written.
pub fn record_max_size(connection: &Connection, settings: &Resolved) -> rusqlite::Result<()> {
    record_cap(connection, max_size(settings))
}

fn record_cap(connection: &Connection, max_size: u64) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO payload_cap (id, max_size) VALUES (1, ?1)
         ON CONFLICT (id) DO UPDATE SET max_size = excluded.max_size",
        )
        .and_then(|mut cached| cached.execute([i64::try_from(max_size).unwrap_or(i64::MAX)]))?;
    Ok(())
}

fn max_size(settings: &Resolved) -> u64 {
    settings
        .count(CRAWL_PAYLOAD_MAX_SIZE.name)
        .unwrap_or_else(built_in_max_size)
}

fn built_in_max_size() -> u64 {
    (CRAWL_PAYLOAD_MAX_SIZE.default)()
        .as_u64()
        .expect("invariant: `crawl.payload.max_size`'s default is a count")
}

/// Keeps the Payload of the crawl request `request`, when it has one.
///
/// # Errors
///
/// When the run file cannot be written.
pub fn store(
    connection: &Connection,
    request: &str,
    payload: Option<&Value>,
    work: &CountedWork,
) -> rusqlite::Result<()> {
    if let Some(payload) = payload {
        work.count(Row::PayloadSerializations);
        connection
            .prepare_cached("INSERT INTO payloads (request, payload) VALUES (?1, ?2)")
            .and_then(|mut cached| cached.execute((request, payload.to_string())))?;
    }
    Ok(())
}

/// The Payload of the crawl request `request`, if it has one.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn of_request(connection: &Connection, request: &str) -> rusqlite::Result<Option<Value>> {
    let payload: Option<String> = connection
        .prepare_cached("SELECT payload FROM payloads WHERE request = ?1")?
        .query_row([request], |row| row.get(0))
        .optional()?;
    payload
        .map(|payload| {
            serde_json::from_str(&payload).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

/// The hash of `schema`: the SHA-256 of its JSON, its keys in order.
fn hash(schema: &Value) -> String {
    HEXLOWER.encode(&Sha256::digest(schema.to_string().as_bytes()))
}

/// Each way `payload`, a missing one read as `null`, does not fit
/// `schema`, a JSON Schema [`PayloadRules::of`] checked.
#[expect(
    clippy::disallowed_methods,
    reason = "a Payload's validator, which `F3.compilations` has yet to count (ADR 0140)"
)]
fn issues(schema: &Value, payload: Option<&Value>) -> Vec<PayloadIssue> {
    let Ok(validator) = jsonschema::validator_for(schema) else {
        return Vec::new();
    };
    let payload = payload.unwrap_or(&Value::Null);
    validator
        .iter_errors(payload)
        .map(|error| PayloadIssue {
            path: error.instance_path().to_string(),
            message: error.to_string(),
        })
        .collect()
}

fn described(issues: &[PayloadIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            if issue.path.is_empty() {
                issue.message.clone()
            } else {
                format!("at `{}`, {}", issue.path, issue.message)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests;
