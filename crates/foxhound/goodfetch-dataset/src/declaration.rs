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
//! names it by its position among the Records pushed together, or the
//! first by its position in the answer, and each issue by its path, a JSON
//! Pointer into the Record
//! (ADR 0147). Each
//! schema's validator is built once, with the Datasets, as the crawl starts
//! or resumes, and a Record's check builds none (ADR 0138); a reader of its
//! Records takes the declarations alone, building none, and a run file no
//! crawl wrote declares none.

use std::sync::Arc;

use data_encoding::HEXLOWER;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::{
    DATASET_DUPLICATE, DATASET_KEY_INVALID, DATASET_UNKNOWN, DatasetDeclaration,
    DatasetDuplicateDetails, DatasetKeyInvalidDetails, DatasetUnknownDetails, InvalidRecord,
    KeyRefused, RECORD_INVALID, RecordData, RecordInvalidDetails, RecordIssue,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::records::Checked;
use crate::rejections::{Rejection, Why};
use crate::run_tables::DATASETS_TABLE;

/// A push the check refused: the error `crawl_request.push` answers, and
/// what to stage: the Records that hold and, in the place of each that
/// does not, its rejection; nothing when the crawl does not declare their
/// Dataset.
#[derive(Debug)]
pub struct Refused {
    /// The error.
    pub error: CallError,
    /// What to stage instead.
    pub staged: Checked,
}

/// The Datasets a crawl declares, in the order declared, each checked, with
/// the validator of each schema, built once with them, and whether the
/// crawl keeps Records in them, its `dataset.records` (ADR 0138). A clone
/// shares them.
#[derive(Clone, Debug, Default)]
pub struct Datasets(Arc<Declared>, Keeping);

/// Whether a crawl keeps the Records its attempts give.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Keeping {
    /// Each is checked against its Dataset and committed.
    #[default]
    Records,
    /// None is checked or committed: `dataset.records` is off.
    Nothing,
}

/// The declarations, and the validator of each one's schema, at the same
/// position: none for a Dataset with no schema, or for one whose schema the
/// run file holds but no longer builds, under which a Record holds as it is.
/// Beside them, at the same position, the hash of each one's schema as the
/// run file keeps it, which each of its Records takes: none for a Dataset
/// with no schema.
#[derive(Debug, Default)]
struct Declared {
    declarations: Vec<DatasetDeclaration>,
    validators: Vec<Option<jsonschema::Validator>>,
    schema_hashes: Vec<Option<Arc<str>>>,
}

impl Declared {
    /// The declarations `kept` holds, each beside its schema's hash, with
    /// the validator of each schema `built` gives.
    fn of(
        kept: Vec<(DatasetDeclaration, Option<String>)>,
        built: impl Fn(&DatasetDeclaration, &Value) -> Option<jsonschema::Validator>,
    ) -> Self {
        let (declarations, schema_hashes): (Vec<_>, Vec<_>) = kept
            .into_iter()
            .map(|(declaration, hash)| (declaration, hash.map(Arc::from)))
            .unzip();
        let validators = declarations
            .iter()
            .map(|declaration| {
                let schema = declaration.schema.as_ref()?;
                built(declaration, schema)
            })
            .collect();
        Self {
            declarations,
            validators,
            schema_hashes,
        }
    }
}

/// A declared Dataset, as a Record's check finds it.
struct Found<'a> {
    declaration: &'a DatasetDeclaration,
    validator: Option<&'a jsonschema::Validator>,
    schema_hash: Option<&'a Arc<str>>,
}

/// Two Datasets are equal when they declare the same, their validators
/// following from their schemas.
impl PartialEq for Datasets {
    fn eq(&self, other: &Self) -> bool {
        self.0.declarations == other.0.declarations
    }
}

impl Eq for Datasets {}

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
        let mut validators = Vec::with_capacity(declarations.len());
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
                validators.push(None);
                continue;
            };
            let validator = built(schema).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("The Dataset `{name}`'s schema is no JSON Schema: {error}"),
                )
            })?;
            validators.push(Some(validator));
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
        Ok(Self(
            Arc::new(Declared {
                declarations: declarations.to_vec(),
                validators,
                schema_hashes: declarations
                    .iter()
                    .map(|declaration| {
                        declaration
                            .schema
                            .as_ref()
                            .map(|schema| hash(schema).into())
                    })
                    .collect(),
            }),
            Keeping::Records,
        ))
    }

    /// Records the declarations in the run file `connection` holds, as the
    /// Run begins.
    ///
    /// # Errors
    ///
    /// The run file's, when a row cannot be written.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut insert = connection.prepare_cached(
            "INSERT INTO datasets (name, position, schema, schema_hash, key)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO UPDATE SET position = excluded.position,
                 schema = excluded.schema, schema_hash = excluded.schema_hash, key = excluded.key",
        )?;
        let Declared {
            declarations,
            schema_hashes,
            ..
        } = &*self.0;
        for (position, (declaration, schema_hash)) in
            declarations.iter().zip(schema_hashes).enumerate()
        {
            insert.execute((
                &declaration.name,
                i64::try_from(position).unwrap_or(i64::MAX),
                declaration.schema.as_ref().map(Value::to_string),
                schema_hash.as_deref(),
                Value::from(declaration.key.clone()).to_string(),
            ))?;
        }
        Ok(())
    }

    /// The same Datasets, keeping the Records the crawl's attempts give
    /// when `keeps`, its `dataset.records`, and else none: a Record is then
    /// neither validated, encoded nor committed, though one named for a
    /// Dataset the crawl does not declare is still refused (ADR 0138).
    #[must_use]
    pub fn keeping_records(self, keeps: bool) -> Self {
        let keeping = if keeps {
            Keeping::Records
        } else {
            Keeping::Nothing
        };
        Self(self.0, keeping)
    }

    /// Whether the crawl keeps the Records its attempts give.
    pub fn keeps_records(&self) -> bool {
        self.1 == Keeping::Records
    }

    /// The declarations, in the order declared.
    pub fn declarations(&self) -> &[DatasetDeclaration] {
        &self.0.declarations
    }

    /// The declarations the run file `connection` holds, each schema's
    /// validator built and its hash as the run file keeps it, as a resuming
    /// crawl takes them; a row that no longer reads is left out, and a
    /// schema that no longer builds checks nothing.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        Ok(Self(
            Arc::new(Declared::of(kept(connection)?, |declaration, schema| {
                built(schema)
                .inspect_err(|error| {
                    tracing::warn!(dataset = %declaration.name, %error, "a declared Dataset's schema in the run file no longer builds; its Records are not checked against it");
                })
                .ok()
            })),
            Keeping::Records,
        ))
    }

    /// `records`, once each holds for the Dataset it names.
    ///
    /// # Errors
    ///
    /// The first failing Record's: `dataset_unknown` when the crawl does not
    /// declare its Dataset; `record_invalid`, naming its position in
    /// `records` and each issue's path, when it is no JSON object, does not
    /// fit its Dataset's schema, or lacks a field of the key of a Dataset
    /// with no schema.
    pub fn check(
        &self,
        records: Vec<PushedRecord>,
        work: &CountedWork,
    ) -> Result<Checked, CallError> {
        if !self.keeps_records() {
            for record in &records {
                self.declared(&record.dataset)?;
            }
            return Ok(Checked::default());
        }
        let mut schema_hashes = Vec::with_capacity(records.len());
        for (position, record) in records.iter().enumerate() {
            let Found {
                declaration,
                validator,
                schema_hash,
            } = self.declared(&record.dataset)?;
            let issues = issues(declaration, validator, &record.data, work);
            if !issues.is_empty() {
                let message = if records.len() == 1 {
                    record_message(&record.dataset, &issues)
                } else {
                    format!(
                        "The first Record of the handler's answer that does not hold, at position {position}, of the Dataset `{}`, was not staged: {}",
                        record.dataset,
                        described(&issues)
                    )
                };
                return Err(invalid(
                    &record.dataset,
                    vec![InvalidRecord {
                        position: position_of(position),
                        issues,
                    }],
                    message,
                ));
            }
            schema_hashes.push(schema_hash.cloned());
        }
        Ok(Checked::new(
            records.into_iter().zip(schema_hashes).collect(),
        ))
    }

    /// `records`, pushed together into the Dataset `dataset`, once each
    /// holds for it, as `crawl_request.push` stages them (ADR 0147).
    ///
    /// # Errors
    ///
    /// `dataset_unknown` when the crawl does not declare `dataset`, with
    /// nothing to stage; `record_invalid` when any Record does not hold, as
    /// [`check`](Self::check) finds it, naming each such Record by its
    /// position in `records` and each of its issues by its path, with the
    /// Records that hold to stage and, in the place of each that does not,
    /// its rejection: the Record, its own message as the reason, and each
    /// issue (ADR 0103).
    pub fn check_pushed(
        &self,
        dataset: &str,
        records: Vec<RecordData>,
        work: &CountedWork,
    ) -> Result<Checked, Refused> {
        let found = self.declared(dataset).map_err(|error| Refused {
            error,
            staged: Checked::default(),
        })?;
        if !self.keeps_records() {
            return Ok(Checked::default());
        }
        let together = records.len();
        let mut holding = Vec::with_capacity(together);
        let mut rejected = Vec::new();
        let mut invalid_records = Vec::new();
        for (position, data) in records.into_iter().enumerate() {
            let issues = issues(found.declaration, found.validator, &data, work);
            let dataset = dataset.to_owned();
            if issues.is_empty() {
                holding.push((PushedRecord { dataset, data }, found.schema_hash.cloned()));
                continue;
            }
            rejected.push(Rejection {
                why: Why {
                    reason: record_message(&dataset, &issues),
                    issues: issues.clone(),
                    hook: None,
                },
                dataset,
                data,
            });
            invalid_records.push(InvalidRecord {
                position: position_of(position),
                issues,
            });
        }
        let message = match (together, rejected.as_slice()) {
            (_, []) => None,
            (1, [alone]) => Some(alone.why.reason.clone()),
            _ => Some(several_message(dataset, together, &invalid_records)),
        };
        let staged = Checked::with_rejected(holding, rejected);
        match message {
            None => Ok(staged),
            Some(message) => Err(Refused {
                error: invalid(dataset, invalid_records, message),
                staged,
            }),
        }
    }

    /// The declaration of the Dataset `name`, with its schema's validator
    /// and hash.
    fn declared(&self, name: &str) -> Result<Found<'_>, CallError> {
        let Declared {
            declarations,
            validators,
            schema_hashes,
        } = &*self.0;
        declarations
            .iter()
            .zip(validators)
            .zip(schema_hashes)
            .find(|((declared, _), _)| declared.name == name)
            .map(|((declaration, validator), schema_hash)| Found {
                declaration,
                validator: validator.as_ref(),
                schema_hash: schema_hash.as_ref(),
            })
            .ok_or_else(|| {
                unknown(
                    declarations,
                    name,
                    format!(
                        "The crawl declares no Dataset `{name}`, so the Record was not staged: push it into one the crawl declares."
                    ),
                )
            })
    }
}

/// `position`, an index into a list held in memory, as the wire's number.
fn position_of(position: usize) -> u64 {
    u64::try_from(position).expect("invariant: a list's index fits in 64 bits")
}

/// The Datasets a crawl declares, as a reader of its Records takes them:
/// their declarations alone, in the order declared, with no validator built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declarations(Vec<DatasetDeclaration>);

impl Declarations {
    /// The declarations the run file `connection` holds; a row that no
    /// longer reads is left out, and a run file no crawl wrote, which holds
    /// no Dataset's table, declares none.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        Ok(Self(
            kept(connection)?
                .into_iter()
                .map(|(declaration, _)| declaration)
                .collect(),
        ))
    }

    /// The declarations, in the order declared.
    pub fn declarations(&self) -> &[DatasetDeclaration] {
        &self.0
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
        Err(unknown(
            &self.0,
            name,
            format!(
                "The crawl declares no Dataset `{name}`, so no Record was read: read one the crawl declares."
            ),
        ))
    }
}

/// The declarations the run file `connection` holds, in the order
/// declared, each beside its schema's hash as the file keeps it; a row that
/// no longer reads is left out, and a run file no crawl wrote, which holds
/// no Dataset's table, declares none.
fn kept(connection: &Connection) -> rusqlite::Result<Vec<(DatasetDeclaration, Option<String>)>> {
    if !DATASETS_TABLE.held_by(connection)? {
        return Ok(Vec::new());
    }
    let mut select = connection
        .prepare_cached("SELECT name, schema, key, schema_hash FROM datasets ORDER BY position")?;
    let rows = select.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut declared = Vec::new();
    for row in rows {
        let (name, schema, key, schema_hash) = row?;
        let schema = schema
            .map(|schema| serde_json::from_str(&schema))
            .transpose();
        match (schema, serde_json::from_str(&key)) {
            (Ok(schema), Ok(key)) => {
                declared.push((DatasetDeclaration { name, schema, key }, schema_hash));
            }
            _ => {
                tracing::warn!(dataset = %name, "a declared Dataset in the run file no longer reads");
            }
        }
    }
    Ok(declared)
}

/// `dataset_unknown` for `name` among `declarations`, saying `message`.
fn unknown(declarations: &[DatasetDeclaration], name: &str, message: String) -> CallError {
    CallError::new(
        &DATASET_UNKNOWN,
        message,
        &DatasetUnknownDetails {
            dataset: name.to_owned(),
            declared: declarations
                .iter()
                .map(|declared| declared.name.clone())
                .collect(),
        },
    )
}

/// `record_invalid` for the Records `records` of the Dataset `name`, at
/// least one, said by `message`.
fn invalid(name: &str, records: Vec<InvalidRecord>, message: String) -> CallError {
    CallError::new(
        &RECORD_INVALID,
        message,
        &RecordInvalidDetails {
            dataset: name.to_owned(),
            records,
        },
    )
}

/// Why `records` of the Dataset `name`, among the `together` pushed
/// together, were not staged, naming each by its position.
fn several_message(name: &str, together: usize, records: &[InvalidRecord]) -> String {
    format!(
        "Of the {together} Records pushed together, {} of the Dataset `{name}` {}: {}",
        records.len(),
        if records.len() == 1 {
            "does not hold, so it was not staged"
        } else {
            "do not hold, so they were not staged"
        },
        records
            .iter()
            .map(|record| format!(
                "the one at position {}: {}",
                record.position,
                described(&record.issues)
            ))
            .collect::<Vec<_>>()
            .join("; ")
    )
}

/// Why one Record of the Dataset `name` with `issues` was not staged, the
/// reason its rejection keeps.
fn record_message(name: &str, issues: &[RecordIssue]) -> String {
    format!(
        "A Record of the Dataset `{name}` does not hold, so it was not staged: {}",
        described(issues)
    )
}

/// The top-level fields `schema` requires.
fn required(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|fields| fields.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Each way `data` does not hold for `declaration`, under its schema's
/// `validator`, `record` read from its text.
fn issues(
    declaration: &DatasetDeclaration,
    validator: Option<&jsonschema::Validator>,
    record: &RecordData,
    work: &CountedWork,
) -> Vec<RecordIssue> {
    let data = record.value();
    let Some(object) = data.as_object() else {
        return vec![RecordIssue {
            path: String::new(),
            message: "is no JSON object".to_owned(),
        }];
    };
    match &declaration.schema {
        Some(_) => {
            let Some(validator) = validator else {
                return Vec::new();
            };
            work.count(Row::RecordValidations);
            validator
                .iter_errors(&data)
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

/// The validator of `schema`, built once per Dataset as the crawl starts or
/// resumes, on no Record: no row's work (ADR 0138).
fn built(schema: &Value) -> Result<jsonschema::Validator, jsonschema::ValidationError<'_>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "each schema's one validator build, per Run and on no Record (ADR 0138)"
    )]
    jsonschema::validator_for(schema)
}

/// The validator of `schema`, counted on `F4.validator_builds`: the one
/// function a validator built per Record would go through, which a
/// Record's check, using the validators built with the Datasets, never
/// calls; the counts' plants build through it (ADR 0140).
///
/// # Errors
///
/// The validator's, when `schema` is no JSON Schema.
pub fn counted_validator<'schema>(
    work: &CountedWork,
    schema: &'schema Value,
) -> Result<jsonschema::Validator, jsonschema::ValidationError<'schema>> {
    work.count(Row::RecordValidatorBuilds);
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a Record's validator build (ADR 0140)"
    )]
    jsonschema::validator_for(schema)
}

#[cfg(test)]
mod tests;
