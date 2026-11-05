//! A resume's check of a crawl's Datasets (ADR 0103): a start under the ID
//! of a Run compares the Datasets the crawl declares with those the Run
//! began with, as its run file holds them, before any work. A Dataset that
//! holds Records but is no longer declared, a renamed one included, fails
//! with `dataset_unknown`, so no Record is orphaned; one whose key changed
//! with `dataset_key_invalid`, so a Sink's upsert never mixes two
//! identities. A rejected Record counts as a Record here, so a Dataset whose
//! every Record failed its schema keeps its rejections readable under its
//! name (#964). A Dataset that holds no Record may be dropped or keyed anew,
//! and a changed schema is allowed: once the Run resumes, the declarations
//! are recorded again, so the Records committed from then on are checked
//! against the new schema and keep its hash.

use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::dataset::{
    DATASET_KEY_INVALID, DATASET_UNKNOWN, DatasetKeyInvalidDetails, DatasetUnknownDetails,
    KeyRefused,
};
use rusqlite::Connection;

use crate::declaration::Datasets;

/// The Datasets a Run began with, as its run file holds them: each one's
/// name, its key, none when it no longer reads, and whether it holds
/// Records, committed or rejected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Began(Vec<BeganWith>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct BeganWith {
    name: String,
    key: Option<Vec<String>>,
    holds_records: bool,
}

impl Began {
    /// The Datasets the run file `connection` holds.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let mut select = connection.prepare_cached(
            "SELECT name, key,
                 EXISTS (SELECT 1 FROM records WHERE records.dataset = datasets.name)
                 OR EXISTS (SELECT 1 FROM rejected_records
                     WHERE rejected_records.dataset = datasets.name)
             FROM datasets ORDER BY position",
        )?;
        let rows = select.query_map([], |row| {
            Ok(BeganWith {
                name: row.get(0)?,
                key: serde_json::from_str(&row.get::<_, String>(1)?).ok(),
                holds_records: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map(Self)
    }
}

impl Datasets {
    /// Whether the Run `run_id`, which began with `began`, may go on under
    /// these declarations.
    ///
    /// # Errors
    ///
    /// `dataset_unknown` for the first Dataset that holds Records, committed
    /// or rejected, and is no longer declared; `dataset_key_invalid` for the
    /// first that holds them and whose key changed.
    pub fn resumes(&self, began: &Began, run_id: &str) -> Result<(), CallError> {
        let declarations = self.declarations();
        for dataset in began.0.iter().filter(|dataset| dataset.holds_records) {
            let name = &dataset.name;
            let Some(declared) = declarations.iter().find(|declared| &declared.name == name) else {
                return Err(CallError::new(
                    &DATASET_UNKNOWN,
                    format!(
                        "The Run `{run_id}` holds Records, committed or rejected, of the Dataset `{name}`, which the crawl no longer declares, so the Run did no work: declare `{name}` again, or start a new Run."
                    ),
                    &DatasetUnknownDetails {
                        dataset: name.clone(),
                        declared: declarations
                            .iter()
                            .map(|declared| declared.name.clone())
                            .collect(),
                    },
                ));
            };
            match &dataset.key {
                Some(began_with) if began_with != &declared.key => {
                    return Err(CallError::new(
                        &DATASET_KEY_INVALID,
                        format!(
                            "The Run `{run_id}` holds Records of the Dataset `{name}` keyed by {began_with:?}, and the crawl now keys it by {:?}, so the Run did no work: keep its key, or start a new Run.",
                            declared.key
                        ),
                        &DatasetKeyInvalidDetails {
                            dataset: name.clone(),
                            refused: KeyRefused::Changed {
                                key: declared.key.clone(),
                                began_with: began_with.clone(),
                            },
                        },
                    ));
                }
                Some(_) => {}
                None => {
                    tracing::warn!(dataset = %name, "the key a resumed Run's Dataset began with no longer reads");
                }
            }
        }
        Ok(())
    }

    /// Records these declarations again in the run file `connection` holds,
    /// as a Run that [`Datasets::resumes`] allowed resumes: each declared
    /// Dataset takes its new position, schema, schema hash and key, and one
    /// no longer declared that holds no Record, committed or rejected, is
    /// dropped.
    ///
    /// # Errors
    ///
    /// The run file's, when a row cannot be written.
    pub fn record_again(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection
            .prepare_cached(
                "DELETE FROM datasets
             WHERE NOT EXISTS (SELECT 1 FROM records WHERE records.dataset = datasets.name)
               AND NOT EXISTS (SELECT 1 FROM rejected_records
                   WHERE rejected_records.dataset = datasets.name)",
            )
            .and_then(|mut cached| cached.execute([]))?;
        self.record(connection)
    }
}
