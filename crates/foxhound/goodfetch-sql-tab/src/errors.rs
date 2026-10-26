//! DuckDB's translation point (ADR 0127): the one place that reads
//! DuckDB's failures. A Parser, Binder or Catalog error, and a statement
//! that is not one `SELECT` or `WITH`, are an invalid query; a Permission
//! error is a read of a file or URL, which the locked connection never
//! reaches, and so an invalid query too; Out of Memory is the memory
//! limit; an Interrupt is the engine's, at the query's deadline; any other
//! failure is unrecognized, logged at `warn` with DuckDB's text, which no
//! other code reads.

use goodfetch_protocol::foxhound::sql_tab::QueryRefusal;
use serde_json::Value;

/// How a query failed, as the engine answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// `query_invalid`, with why and DuckDB's explanation for the person
    /// who wrote the statement.
    Invalid {
        /// Why.
        reason: QueryRefusal,
        /// DuckDB's explanation.
        explanation: String,
    },
    /// `query_memory_exceeded`.
    MemoryExceeded,
    /// The engine interrupted it, at its deadline.
    Interrupted,
    /// `query_failed`: nothing the engine recognises.
    Unrecognized,
}

/// The class of a DuckDB error, the words before ` Error: ` in its text.
fn class(text: &str) -> Option<&str> {
    text.split_once(" Error: ").map(|(class, _)| class)
}

/// What a failure of DuckDB's means.
pub(crate) fn translate(error: &duckdb::Error) -> Failure {
    let text = error.to_string();
    let invalid = |reason| Failure::Invalid {
        reason,
        explanation: text.clone(),
    };
    match class(&text) {
        Some("Parser") => invalid(QueryRefusal::Syntax),
        Some("Binder" | "Catalog") => invalid(QueryRefusal::UnknownName),
        Some("Permission") => invalid(QueryRefusal::OutsideRead),
        Some("Out of Memory") => Failure::MemoryExceeded,
        Some("INTERRUPT") => Failure::Interrupted,
        _ => {
            tracing::warn!(error = %text, "DuckDB failed in a way the SQL tab does not recognise");
            Failure::Unrecognized
        }
    }
}

/// Whether `serialized`, what `json_serialize_sql` answered for a
/// statement, is one read, `SELECT` or `WITH`: it serializes nothing else.
pub(crate) fn one_read(serialized: &str) -> Result<(), Failure> {
    let parsed: Value = serde_json::from_str(serialized).map_err(|_| Failure::Unrecognized)?;
    if parsed["error"] == Value::Bool(true) {
        let explanation = parsed["error_message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let reason = if parsed["error_type"] == "parser" {
            QueryRefusal::Syntax
        } else {
            QueryRefusal::NotARead
        };
        return Err(Failure::Invalid {
            reason,
            explanation,
        });
    }
    match parsed["statements"].as_array().map(Vec::len) {
        Some(1) => Ok(()),
        Some(_) => Err(Failure::Invalid {
            reason: QueryRefusal::NotARead,
            explanation: "the SQL tab runs one statement at a time".to_owned(),
        }),
        None => Err(Failure::Unrecognized),
    }
}

#[cfg(test)]
mod tests;
