//! The writer (ADR 0069): it turns a case log and the suite's declared case
//! list into a `conformance` result document, through `goodfetch-protocol`'s
//! constructor, so no runner sets an outcome. A declared case the log does
//! not hold leaves its capability `unproved`; the log's entries for a case
//! become that case's entries, one per time it ran.

use std::collections::BTreeMap;

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::result_document::{
    CapabilityFacts, CaseEntry, CaseFacts, ConformanceDocument, ConformanceHeader,
};

use crate::case_list::CaseList;

/// Reads a case log: one JSON entry per line, blank lines skipped.
///
/// # Errors
///
/// [`CaseLogError::Entry`] for the first line that is not a case log entry,
/// one that sets an outcome among them.
pub fn read_case_log(text: &str) -> Result<Vec<CaseLogEntry>, CaseLogError> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|source| CaseLogError::Entry {
                line: index + 1,
                source,
            })
        })
        .collect()
}

/// Why a case log could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaseLogError {
    /// A line is not a case log entry.
    #[error("line {line} of the case log is not a case log entry: {source}")]
    Entry {
        /// Its number, from 1.
        line: usize,
        /// Why.
        source: serde_json::Error,
    },
}

/// The `conformance` document of a case log, each capability and case in
/// the case list's order.
///
/// # Errors
///
/// When the header names another suite than the case list, or an entry
/// names a case the list does not declare, another capability than its
/// case's, or another platform or runtime than the header's.
pub fn write_document(
    header: ConformanceHeader,
    case_list: &CaseList,
    log: &[CaseLogEntry],
) -> Result<ConformanceDocument, WriteError> {
    if header.suite != case_list.suite {
        return Err(WriteError::OtherSuite);
    }
    let capability_of: BTreeMap<&str, &str> = case_list
        .capabilities
        .iter()
        .flat_map(|capability| {
            capability
                .cases
                .iter()
                .map(|case| (case.id.as_str(), capability.name.as_str()))
        })
        .collect();
    let mut entries: BTreeMap<&str, Vec<CaseEntry>> = BTreeMap::new();
    for entry in log {
        let Some(&capability) = capability_of.get(entry.case.as_str()) else {
            return Err(WriteError::UndeclaredCase {
                case: entry.case.clone(),
            });
        };
        if capability != entry.capability {
            return Err(WriteError::OtherCapability {
                case: entry.case.clone(),
                declared: capability.to_owned(),
                logged: entry.capability.clone(),
            });
        }
        if entry.platform != header.platform || entry.runtime != header.runtime {
            return Err(WriteError::OtherHost {
                case: entry.case.clone(),
            });
        }
        entries
            .entry(entry.case.as_str())
            .or_default()
            .push(CaseEntry {
                mode: entry.mode.clone(),
                blocking: entry.blocking,
                unreachable: entry.unreachable.clone(),
                checks: entry.checks.clone(),
                evidence: entry.evidence.clone(),
            });
    }
    let capabilities = case_list
        .capabilities
        .iter()
        .map(|capability| CapabilityFacts {
            name: capability.name.clone(),
            cases: capability
                .cases
                .iter()
                .map(|case| CaseFacts {
                    id: case.id.clone(),
                    covers: case.covers.clone(),
                    entries: entries.remove(case.id.as_str()).unwrap_or_default(),
                })
                .collect(),
        })
        .collect();
    Ok(ConformanceDocument::new(header, capabilities))
}

/// Why a case log could not become a document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// The header's suite is not the case list's.
    #[error("the header names another suite or version than the case list")]
    OtherSuite,
    /// An entry names a case the list does not declare.
    #[error("the case log has an entry for `{case}`, which the case list does not declare")]
    UndeclaredCase {
        /// The case's ID.
        case: String,
    },
    /// An entry names another capability than its case's.
    #[error(
        "the case log files `{case}` under `{logged}`, but the case list declares it under `{declared}`"
    )]
    OtherCapability {
        /// The case's ID.
        case: String,
        /// The capability the case list gives it.
        declared: String,
        /// The capability the entry names.
        logged: String,
    },
    /// An entry ran on another platform or runtime than the document's.
    #[error(
        "the case log's entry for `{case}` ran on another platform or runtime than the document's"
    )]
    OtherHost {
        /// The case's ID.
        case: String,
    },
}
