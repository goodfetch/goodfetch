//! The Datasets a crawl declares (ADRs 0082 and 0103), and the check of a
//! Record pushed into one. A declaration names the Dataset, may give it a
//! JSON Schema and a key: a second Dataset of one name is refused with
//! `dataset_duplicate`, a key on a field that is no top-level field its
//! schema requires with `dataset_key_invalid`, and a schema that is no JSON
//! Schema with `invalid_params`, all before the Run begins. The
//! declarations are kept in the run file with each schema's hash, the
//! SHA-256 of its JSON with keys in order, which a resume compares.
//!
//! A pushed Record names a declared Dataset, else `dataset_unknown`; it is
//! a JSON object that fits its Dataset's schema and, for a Dataset with a
//! key and no schema, holds every field of the key, else `record_invalid`
//! names each issue by its path, a JSON Pointer into the Record.

use data_encoding::HEXLOWER;
use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::{
    DATASET_DUPLICATE, DATASET_KEY_INVALID, DATASET_UNKNOWN, DatasetDeclaration,
    DatasetDuplicateDetails, DatasetKeyInvalidDetails, DatasetUnknownDetails, KeyRefused,
    RECORD_INVALID, RecordInvalidDetails, RecordIssue,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::records::Checked;
use crate::rejections::{Rejection, Why};

/// A pushed Record the check refused: the error `crawl_request.push`
/// answers, and what to stage in the Record's place, its rejection when
/// the crawl declares its Dataset, else nothing.
#[derive(Debug)]
pub struct Refused {
    /// The error.
    pub error: CallError,
    /// What to stage instead.
    pub rejected: Checked,
}

/// The Datasets a crawl declares, in the order declared, each checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Datasets(Vec<DatasetDeclaration>);

impl Datasets {
    /// The Datasets `declarations` declare.
    ///
    /// # Errors
    ///
    /// `dataset_duplicate` for a second Dataset of one name;
    /// `dataset_key_invalid` for a key's field that is no top-level field
    /// its schema requires; `invalid_params` for a schema that is no JSON
    /// Schema.
    pub fn declare(declarations: &[DatasetDeclaration]) -> Result<Self, CallError> {
        for (position, declaration) in declarations.iter().enumerate() {
            let name = &declaration.name;
            if declarations[..position]
                .iter()
                .any(|earlier| &earlier.name == name)
            {
                return Err(CallError::new(
                    &DATASET_DUPLICATE,
                    format!(
                        "The crawl declares the Dataset `{name}` twice, so it was not started: give each Dataset its own name."
                    ),
                    &DatasetDuplicateDetails {
                        dataset: name.clone(),
                    },
                ));
            }
            let Some(schema) = &declaration.schema else {
                continue;
            };
            if let Err(error) = jsonschema::validator_for(schema) {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("The Dataset `{name}`'s schema is no JSON Schema: {error}"),
                ));
            }
            let required = required(schema);
            if let Some(field) = declaration
                .key
                .iter()
                .find(|field| !required.contains(&field.as_str()))
            {
                return Err(CallError::new(
                    &DATASET_KEY_INVALID,
                    format!(
                        "The Dataset `{name}`'s key names `{field}`, which is no top-level field its schema requires, so a Record could lack it and the crawl was not started: key the Dataset only by fields its schema requires."
                    ),
                    &DatasetKeyInvalidDetails {
                        dataset: name.clone(),
                        refused: KeyRefused::NotRequired {
                            field: field.clone(),
                        },
                    },
                ));
            }
        }
        Ok(Self(declarations.to_vec()))
    }

    /// Records the declarations in the run file `connection` holds, as the
    /// Run begins.
    ///
    /// # Errors
    ///
    /// The run file's, when a row cannot be written.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut insert = connection.prepare(
            "INSERT INTO datasets (name, position, schema, schema_hash, key)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO UPDATE SET position = excluded.position,
                 schema = excluded.schema, schema_hash = excluded.schema_hash, key = excluded.key",
        )?;
        for (position, declaration) in self.0.iter().enumerate() {
            insert.execute((
                &declaration.name,
                i64::try_from(position).unwrap_or(i64::MAX),
                declaration.schema.as_ref().map(Value::to_string),
                declaration.schema.as_ref().map(hash),
                Value::from(declaration.key.clone()).to_string(),
            ))?;
        }
        Ok(())
    }

    /// The declarations, in the order declared.
    pub fn declarations(&self) -> &[DatasetDeclaration] {
        &self.0
    }

    /// The declarations the run file `connection` holds; a row that no
    /// longer reads is left out.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let mut select =
            connection.prepare("SELECT name, schema, key FROM datasets ORDER BY position")?;
        let rows = select.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut declared = Vec::new();
        for row in rows {
            let (name, schema, key) = row?;
            let schema = schema
                .map(|schema| serde_json::from_str(&schema))
                .transpose();
            match (schema, serde_json::from_str(&key)) {
                (Ok(schema), Ok(key)) => declared.push(DatasetDeclaration { name, schema, key }),
                _ => {
                    tracing::warn!(dataset = %name, "a declared Dataset in the run file no longer reads");
                }
            }
        }
        Ok(Self(declared))
    }

    /// `records`, once each holds for the Dataset it names.
    ///
    /// # Errors
    ///
    /// The first failing Record's: `dataset_unknown` when the crawl does not
    /// declare its Dataset; `record_invalid`, naming each issue's path, when
    /// it is no JSON object, does not fit its Dataset's schema, or lacks a
    /// field of the key of a Dataset with no schema.
    pub fn check(&self, records: Vec<PushedRecord>) -> Result<Checked, CallError> {
        for record in &records {
            self.check_one(record)?;
        }
        Ok(Checked::new(records))
    }

    /// `record`, once it holds for the Dataset it names, as
    /// `crawl_request.push` stages it.
    ///
    /// # Errors
    ///
    /// [`check`](Self::check)'s, with the rejection to stage in its place
    /// when the crawl declares its Dataset: the Record, the error's
    /// message as the reason, and each issue (ADR 0103).
    pub fn check_pushed(&self, record: PushedRecord) -> Result<Checked, Refused> {
        let Err(error) = self.check_one(&record) else {
            return Ok(Checked::new(vec![record]));
        };
        let rejected = match self.declared(&record.dataset) {
            Ok(declaration) => Checked::rejected(Rejection {
                why: Why {
                    reason: error.message().to_owned(),
                    issues: issues(declaration, &record.data),
                    hook: None,
                },
                dataset: record.dataset,
                data: record.data,
            }),
            Err(_) => Checked::default(),
        };
        Err(Refused { error, rejected })
    }

    fn declared(&self, name: &str) -> Result<&DatasetDeclaration, CallError> {
        self.0
            .iter()
            .find(|declared| declared.name == name)
            .ok_or_else(|| {
                self.unknown(
                    name,
                    format!(
                        "The crawl declares no Dataset `{name}`, so the Record was not staged: push it into one the crawl declares."
                    ),
                )
            })
    }

    /// Answers whether the crawl declares the Dataset `name`, for a reader
    /// of its Records.
    ///
    /// # Errors
    ///
    /// `dataset_unknown`, naming the Datasets the crawl declares, when it
    /// declares none of that name.
    pub fn readable(&self, name: &str) -> Result<(), CallError> {
        if self.0.iter().any(|declared| declared.name == name) {
            return Ok(());
        }
        Err(self.unknown(
            name,
            format!(
                "The crawl declares no Dataset `{name}`, so no Record was read: read one the crawl declares."
            ),
        ))
    }

    /// `dataset_unknown` for `name`, saying `message`.
    fn unknown(&self, name: &str, message: String) -> CallError {
        CallError::new(
            &DATASET_UNKNOWN,
            message,
            &DatasetUnknownDetails {
                dataset: name.to_owned(),
                declared: self
                    .0
                    .iter()
                    .map(|declared| declared.name.clone())
                    .collect(),
            },
        )
    }

    fn check_one(&self, record: &PushedRecord) -> Result<(), CallError> {
        let name = &record.dataset;
        let declaration = self.declared(name)?;
        let issues = issues(declaration, &record.data);
        if issues.is_empty() {
            return Ok(());
        }
        Err(CallError::new(
            &RECORD_INVALID,
            format!(
                "A Record of the Dataset `{name}` does not hold, so it was not staged: {}",
                described(&issues)
            ),
            &RecordInvalidDetails {
                dataset: name.clone(),
                issues,
            },
        ))
    }
}

/// The top-level fields `schema` requires.
fn required(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|fields| fields.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Each way `data` does not hold for `declaration`.
fn issues(declaration: &DatasetDeclaration, data: &Value) -> Vec<RecordIssue> {
    let Some(object) = data.as_object() else {
        return vec![RecordIssue {
            path: String::new(),
            message: "is no JSON object".to_owned(),
        }];
    };
    match &declaration.schema {
        Some(schema) => {
            let Ok(validator) = jsonschema::validator_for(schema) else {
                return Vec::new();
            };
            validator
                .iter_errors(data)
                .map(|error| RecordIssue {
                    path: error.instance_path().to_string(),
                    message: error.to_string(),
                })
                .collect()
        }
        None => declaration
            .key
            .iter()
            .filter(|field| !object.contains_key(field.as_str()))
            .map(|field| RecordIssue {
                path: format!("/{}", field.replace('~', "~0").replace('/', "~1")),
                message: format!("the key's field `{field}` is missing"),
            })
            .collect(),
    }
}

fn described(issues: &[RecordIssue]) -> String {
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

/// The hash of `schema`: the SHA-256 of its JSON, its keys in order.
fn hash(schema: &Value) -> String {
    HEXLOWER.encode(&Sha256::digest(schema.to_string().as_bytes()))
}
