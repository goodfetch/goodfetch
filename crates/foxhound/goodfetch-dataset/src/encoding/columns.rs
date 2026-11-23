//! A table Export's columns (pitfall D07): the union of the fields seen in
//! any of its Records, in first-seen order, and each Record's cells under
//! them. A Record's fields are taken in the order its committed JSON text
//! holds them; under [`Nested::Flatten`] an object's fields are columns of
//! their own, named by their dotted path.

use std::collections::HashMap;

use serde_json::Value;

/// How a field whose value is an object is written, `export.csv.nested`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nested {
    /// As its JSON text, in the field's column.
    Json,
    /// Each of its fields in a column of its own, named by its dotted path,
    /// at any depth; an empty object is its JSON text.
    Flatten,
}

impl Nested {
    /// The way `choice`, one of `CSV_NESTED_VALUES`, names.
    pub(crate) fn of(choice: &str) -> Option<Self> {
        match choice {
            "json" => Some(Self::Json),
            "flatten" => Some(Self::Flatten),
            _ => None,
        }
    }
}

/// One cell of a table Export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    /// Nothing: `null`, or a field the Record lacks.
    Empty,
    /// A string's text.
    Text(String),
    /// A number as JSON writes it.
    Number(String),
    /// `true` or `false`.
    Boolean(bool),
    /// An object's or an array's JSON text.
    Json(String),
}

impl Cell {
    /// Its text in a text format: what a reader gives back.
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Empty => "",
            Self::Text(text) | Self::Number(text) | Self::Json(text) => text,
            Self::Boolean(true) => "true",
            Self::Boolean(false) => "false",
        }
    }
}

/// The type a typed format (Parquet, Avro, Arrow IPC) gives a column, from
/// every value seen in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// No value but `null` seen yet: a string column.
    #[default]
    Unseen,
    /// Only `true` and `false`.
    Boolean,
    /// Only whole numbers a 64-bit integer holds.
    Integer,
    /// Only numbers, some not such whole numbers.
    Float,
    /// Anything else: each cell's text.
    Text,
}

impl Kind {
    /// The kind of a column that held this kind's values and `cell`.
    fn with(self, cell: &Cell, value: &Value) -> Self {
        let kind = match (cell, value) {
            (Cell::Empty, _) => return self,
            (Cell::Boolean(_), _) => Self::Boolean,
            (Cell::Number(_), Value::Number(number)) if number.is_i64() => Self::Integer,
            (Cell::Number(_), Value::Number(number)) if number.is_f64() => Self::Float,
            _ => Self::Text,
        };
        match (self, kind) {
            (Self::Unseen, kind) => kind,
            (old, new) if old == new => old,
            (Self::Integer | Self::Float, Self::Integer | Self::Float) => Self::Float,
            _ => Self::Text,
        }
    }
}

/// The columns of a table Export, as its Records are seen.
#[derive(Clone, Debug)]
pub(crate) struct Columns {
    nested: Nested,
    names: Vec<String>,
    kinds: Vec<Kind>,
    index: HashMap<String, usize>,
    records: u64,
    longest: u64,
}

impl Columns {
    /// No column yet; objects written as `nested` says.
    #[must_use]
    pub(crate) fn new(nested: Nested) -> Self {
        Self {
            nested,
            names: Vec::new(),
            kinds: Vec::new(),
            index: HashMap::new(),
            records: 0,
            longest: 0,
        }
    }

    /// Adds the fields of the Record whose committed JSON text is `json`
    /// that no Record seen before had, after those.
    pub(crate) fn see(&mut self, json: &str) {
        self.records += 1;
        let record = parsed(json);
        for (name, value) in self.fields(&record) {
            let column = if let Some(&column) = self.index.get(&name) {
                column
            } else {
                self.measure(&name);
                self.index.insert(name.clone(), self.names.len());
                self.names.push(name);
                self.kinds.push(Kind::Unseen);
                self.names.len() - 1
            };
            let cell = cell(value);
            self.measure(cell.text());
            self.kinds[column] = self.kinds[column].with(&cell, value);
        }
    }

    /// Counts `text` in the longest cell's length when it is longer.
    fn measure(&mut self, text: &str) {
        // A character takes no more UTF-16 code units than UTF-8 bytes, so
        // a text of no more bytes than the longest is no longer than it.
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > self.longest {
            let units = text.encode_utf16().count();
            self.longest = self.longest.max(u64::try_from(units).unwrap_or(u64::MAX));
        }
    }

    /// How many Records it saw.
    pub(crate) fn records(&self) -> u64 {
        self.records
    }

    /// The length of its longest cell, a column's name included, in UTF-16
    /// code units, as a spreadsheet counts a cell's characters.
    pub(crate) fn longest(&self) -> u64 {
        self.longest
    }

    /// Each column's type, in the columns' order.
    pub(crate) fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    /// The columns' names, in first-seen order.
    #[must_use]
    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// The cells of the Record whose committed JSON text is `json`, one per
    /// column; a field no column names, which [`Columns::see`] never saw,
    /// is left out.
    pub(crate) fn row(&self, json: &str) -> Vec<Cell> {
        let mut cells = vec![Cell::Empty; self.names.len()];
        for (name, value) in self.fields(&parsed(json)) {
            if let Some(&column) = self.index.get(&name) {
                cells[column] = cell(value);
            }
        }
        cells
    }

    /// The Record's fields as columns: its own, or under
    /// [`Nested::Flatten`] each leaf of its objects by its dotted path.
    fn fields<'a>(&self, record: &'a Value) -> Vec<(String, &'a Value)> {
        let mut fields = Vec::new();
        if let Value::Object(object) = record {
            for (name, value) in object {
                self.field(name.clone(), value, &mut fields);
            }
        }
        fields
    }

    fn field<'a>(&self, name: String, value: &'a Value, fields: &mut Vec<(String, &'a Value)>) {
        match value {
            Value::Object(object) if self.nested == Nested::Flatten && !object.is_empty() => {
                for (inner, value) in object {
                    self.field(format!("{name}.{inner}"), value, fields);
                }
            }
            _ => fields.push((name, value)),
        }
    }
}

/// The committed JSON text of a Record, which the Dataset checked was a
/// JSON object when it was pushed.
fn parsed(json: &str) -> Value {
    serde_json::from_str(json).expect("invariant: a committed Record's text is JSON")
}

fn cell(value: &Value) -> Cell {
    match value {
        Value::Null => Cell::Empty,
        Value::String(text) => Cell::Text(text.clone()),
        Value::Number(number) => Cell::Number(number.to_string()),
        Value::Bool(boolean) => Cell::Boolean(*boolean),
        Value::Array(_) | Value::Object(_) => Cell::Json(value.to_string()),
    }
}
