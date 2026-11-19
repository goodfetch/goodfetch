//! TESTING.md's table of time budgets (ADR 0061), read from the "Time
//! budgets" section of its file for "Suites, commands and time budgets": one
//! row per suite and place, each budget a phrase such as `each job ≤ 45 min`
//! or `≤ 60 min per group`.

/// What one budget of a row measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Measure {
    /// Each job of the tier (`job` or `each job`).
    Job,
    /// The run's "all green", from its first job's start.
    AllGreen,
    /// A whole run of the tier (`per group`).
    Group,
    /// Anything else, such as the Quick suite's `tests`, which the store
    /// holds no step timings for.
    Other,
}

/// One budget of a row.
struct Budget {
    measure: Measure,
    seconds: u64,
}

/// One suite and place, and its budgets.
pub(crate) struct Row {
    suite: String,
    place: String,
    budgets: Vec<Budget>,
}

/// Every row of the table.
pub(crate) struct Table {
    rows: Vec<Row>,
}

/// The section and header the table is found by.
const SECTION: &str = "### Time budgets";
const HEADER: &str = "| Suite | Where | Budget |";

impl Table {
    /// The table in that file's `text`; empty when the section or its table
    /// is missing.
    pub(crate) fn read(text: &str) -> Self {
        let rows = text
            .lines()
            .skip_while(|line| line.trim() != SECTION)
            .skip_while(|line| !line.starts_with(HEADER))
            // The header and its separator.
            .skip(2)
            .take_while(|line| line.starts_with('|'))
            .filter_map(row)
            .collect();
        Self { rows }
    }

    /// The row of `suite` and `place`, when the table has one.
    pub(crate) fn row(&self, suite: &str, place: &str) -> Option<&Row> {
        self.rows
            .iter()
            .find(|row| row.suite == suite && row.place == place)
    }
}

impl Row {
    /// The row's budget of `measure`, in seconds, when it has one.
    pub(crate) fn budget(&self, measure: Measure) -> Option<u64> {
        self.budgets
            .iter()
            .find(|budget| budget.measure == measure)
            .map(|budget| budget.seconds)
    }
}

fn row(line: &str) -> Option<Row> {
    let mut cells = crate::markdown::cells(line)?.into_iter();
    let suite = cells.next()?.to_owned();
    let place = cells.next()?.to_owned();
    let budgets = cells.next()?.split(", ").filter_map(budget).collect();
    Some(Row {
        suite,
        place,
        budgets,
    })
}

/// One phrase `<what> ≤ <n> <unit>[ <rest>]`.
fn budget(phrase: &str) -> Option<Budget> {
    let (what, limit) = phrase.split_once('≤')?;
    let mut words = limit.split_whitespace();
    let count: u64 = words.next()?.parse().ok()?;
    let unit = match words.next()? {
        "s" => 1,
        "min" => 60,
        "h" => 3_600,
        _ => return None,
    };
    let rest: Vec<&str> = words.collect();
    let measure = match (what.trim(), rest.as_slice()) {
        ("job" | "each job", []) => Measure::Job,
        ("\"all green\"", []) => Measure::AllGreen,
        ("", ["per", "group"]) => Measure::Group,
        _ => Measure::Other,
    };
    Some(Budget {
        measure,
        seconds: count * unit,
    })
}
