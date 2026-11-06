//! The details window's Records view: the chosen Run's Records, a page of
//! [`RECORDS_PAGE`] at a time, each Dataset's in order, through
//! `dataset.query`.

use goodfetch_protocol::foxhound::sql_tab::DatasetQueryParams;
use goodfetch_protocol::run::RunId;
use serde_json::Value;

use super::{Details, NAME_MOST, RECORDS_PAGE, RecordsPanel, VALUE_MOST, name};
use crate::text;

impl Details {
    /// The Records view of the chosen Run.
    pub(super) fn records_panel(&self) -> RecordsPanel {
        let mut panel = RecordsPanel {
            title: "Records".to_owned(),
            note: None,
            columns: Vec::new(),
            rows: Vec::new(),
            range: String::new(),
            previous: false,
            next: false,
        };
        let Some(id) = &self.chosen else {
            panel.note = Some("Choose a Run in the Runs view to read its Records".to_owned());
            return panel;
        };
        let named = match self.row(id).map(name) {
            Some(name) if name != id.as_str() => format!("{name} ({id})"),
            Some(_) | None => id.as_str().to_owned(),
        };
        panel.title = text::shown(&format!("Records of {named}"), VALUE_MOST);
        match &self.records {
            None => panel.note = Some("Reading the Records…".to_owned()),
            Some(Err(failure)) => {
                panel.note = Some(text::shown(
                    &format!(
                        "The Daemon refused the query: {} ({})",
                        failure.message, failure.code
                    ),
                    VALUE_MOST,
                ));
            }
            Some(Ok(result)) => {
                panel.columns = result
                    .columns
                    .iter()
                    .map(|column| text::shown(&column.name, NAME_MOST))
                    .collect();
                panel.rows = result
                    .rows
                    .iter()
                    .map(|row| row.iter().map(cell).collect())
                    .collect();
                let shown = u64::try_from(panel.rows.len()).unwrap_or(u64::MAX);
                let total = self.records_from + result.total;
                if shown == 0 {
                    panel.note = Some(if self.records_from == 0 {
                        "The Run has no Record".to_owned()
                    } else {
                        "No Record on this page".to_owned()
                    });
                } else {
                    panel.range = format!(
                        "Records {} to {} of {total}",
                        self.records_from + 1,
                        self.records_from + shown
                    );
                }
                panel.previous = self.records_from > 0;
                panel.next = shown < result.total;
            }
        }
        panel
    }
}

/// The `dataset.query` of a page of `run`'s Records, from the Record
/// `from`: each Dataset's in order.
pub(super) fn records_query(run: &RunId, from: u64) -> DatasetQueryParams {
    // A quote in the Run ID is doubled, as SQL quotes it.
    let run_literal = run.as_str().replace('\'', "''");
    let offset = if from > 0 {
        format!(" OFFSET {from}")
    } else {
        String::new()
    };
    DatasetQueryParams {
        sql: format!(
            "SELECT dataset, number, url, committed_at, value FROM records('{run_literal}') ORDER BY dataset, number{offset}"
        ),
        runs: vec![run.clone()],
        limit: Some(RECORDS_PAGE),
    }
}

/// A Record's value as the Records view shows it: a text as written,
/// anything else as JSON.
pub(super) fn cell(value: &Value) -> String {
    let written = match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    text::shown(&written, VALUE_MOST)
}
