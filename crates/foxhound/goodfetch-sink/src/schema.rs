//! Sink schema inference, normalization, widening, variant columns and
//! contracts (ADR 0026): shared across every row-store Sink (ADR 0129).
//!
//! A Dataset's declared JSON Schema comes first; otherwise columns are
//! inferred from the Records. Field names are normalized to `snake_case`
//! with deterministic collision suffixes (`_2`, `_3`, …). Nested objects
//! and arrays stay as JSON unless `schema.flatten` is on, which expands
//! nested objects into `snake_case` columns while keeping arrays as JSON.
//! Under `evolve` (the default), columns are only added, widened
//! (`integer` -> `float`), or split into `<field>__v_<type>`; `freeze`
//! refuses drifting Records; `discard_row` and `discard_value` drop them
//! with counts, and `discard_value` records dropped field names in
//! `_goodfetch_meta`.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use serde_json::{Value, json};

use crate::contract::{Batch, Delivered};

/// A column's logical type across row-store Sinks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    /// Boolean (`INTEGER` 0/1 in SQLite).
    Bool,
    /// 64-bit signed integer (`INTEGER` in SQLite).
    Integer,
    /// 64-bit floating-point number (`REAL` in SQLite).
    Float,
    /// UTF-8 text (`TEXT` in SQLite).
    Text,
    /// Nested JSON object or array (`TEXT` holding JSON in SQLite).
    Json,
}

impl ColumnType {
    /// The variant column suffix `<field>__v_<suffix>` (ADR 0026).
    #[must_use]
    pub const fn variant_suffix(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Integer => "int",
            Self::Float => "float",
            Self::Text => "text",
            Self::Json => "json",
        }
    }

    /// The SQLite `STRICT` column type for this logical type.
    #[must_use]
    pub const fn sqlite_type(self) -> &'static str {
        match self {
            Self::Bool | Self::Integer => "INTEGER",
            Self::Float => "REAL",
            Self::Text | Self::Json => "TEXT",
        }
    }

    /// The PostgreSQL column type for this logical type (ADR 0026).
    #[must_use]
    pub const fn postgres_type(self) -> &'static str {
        match self {
            Self::Bool => "BOOLEAN",
            Self::Integer => "BIGINT",
            Self::Float => "DOUBLE PRECISION",
            Self::Text => "TEXT",
            Self::Json => "JSONB",
        }
    }

    /// Infers a logical type from a PostgreSQL `information_schema.columns.data_type`.
    #[must_use]
    pub fn from_postgres_type(data_type: &str) -> Self {
        match data_type.to_ascii_lowercase().as_str() {
            "boolean" => Self::Bool,
            "bigint" | "integer" | "smallint" => Self::Integer,
            "double precision" | "real" | "numeric" => Self::Float,
            "jsonb" | "json" => Self::Json,
            _ => Self::Text,
        }
    }

    /// Infers a logical type from a SQLite declared column type read back
    /// from `pragma_table_info`.
    #[must_use]
    pub fn from_sqlite_type(sql_type: &str) -> Self {
        match sql_type.to_ascii_uppercase().as_str() {
            "INTEGER" | "INT" => Self::Integer,
            "REAL" => Self::Float,
            _ => Self::Text,
        }
    }

    /// Infers a logical type from a non-null JSON value.
    #[must_use]
    pub fn of_value(value: &Value) -> Option<Self> {
        match value {
            Value::Null => None,
            Value::Bool(_) => Some(Self::Bool),
            Value::Number(number) => {
                if number.is_i64() || number.is_u64() {
                    Some(Self::Integer)
                } else {
                    Some(Self::Float)
                }
            }
            Value::String(_) => Some(Self::Text),
            Value::Array(_) | Value::Object(_) => Some(Self::Json),
        }
    }

    /// Whether a value of type `incoming` fits in a column of type `self`
    /// without altering the column.
    #[must_use]
    pub const fn accepts(self, incoming: Self) -> bool {
        matches!(
            (self, incoming),
            (Self::Bool, Self::Bool)
                | (Self::Integer, Self::Integer)
                | (Self::Float, Self::Float | Self::Integer)
                | (Self::Text, Self::Text)
                | (Self::Json, Self::Json)
        )
    }

    /// The lossless supertype when a column of type `self` receives a value
    /// of type `incoming`, or `None` when a variant column is required.
    #[must_use]
    pub const fn widen(self, incoming: Self) -> Option<Self> {
        match (self, incoming) {
            (Self::Integer, Self::Float) => Some(Self::Float),
            _ if self.accepts(incoming) => Some(self),
            _ => None,
        }
    }
}

/// One typed column value ready to bind into a destination row.
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnValue {
    /// SQL `NULL`.
    Null,
    /// Boolean value.
    Bool(bool),
    /// 64-bit signed integer.
    Integer(i64),
    /// 64-bit floating-point number.
    Float(f64),
    /// UTF-8 string.
    Text(String),
    /// Serialized JSON for a nested object or array.
    Json(String),
}

impl ColumnValue {
    /// Converts `value` into a [`ColumnValue`] matching `target_type`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "widening an integer column to REAL converts 64-bit integers to f64"
    )]
    fn from_json(value: &Value, target_type: ColumnType) -> Self {
        match (value, target_type) {
            (Value::Null, _) => Self::Null,
            (Value::Bool(boolean), ColumnType::Bool) => Self::Bool(*boolean),
            (Value::Number(number), ColumnType::Integer) => number
                .as_i64()
                .or_else(|| number.as_u64().and_then(|u| i64::try_from(u).ok()))
                .map_or_else(|| Self::Text(number.to_string()), Self::Integer),
            (Value::Number(number), ColumnType::Float) => number
                .as_f64()
                .or_else(|| number.as_i64().map(|i| i as f64))
                .map_or(Self::Null, Self::Float),
            (Value::String(text), ColumnType::Text) => Self::Text(text.clone()),
            (other, ColumnType::Json) => Self::Json(other.to_string()),
            (other, _) => Self::Text(other.to_string()),
        }
    }
}

/// A schema evolution event applied to a destination table (ADR 0026).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaChange {
    /// A new column was added for a field.
    AddedColumn {
        /// The normalized column name.
        name: String,
        /// Its type.
        column_type: ColumnType,
    },
    /// An existing column was widened losslessly.
    WidenedColumn {
        /// The column name.
        name: String,
        /// Its previous type.
        from: ColumnType,
        /// Its widened type.
        to: ColumnType,
    },
    /// A variant column `<base>__v_<type>` was added for an incompatible
    /// value.
    AddedVariantColumn {
        /// The original column name.
        base: String,
        /// The variant column name.
        name: String,
        /// The variant's type.
        column_type: ColumnType,
    },
}

/// Counts of rows and values dropped by non-evolving schema contracts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchemaCounters {
    /// Rows dropped under `discard_row`.
    pub discarded_rows: u64,
    /// Individual field values dropped under `discard_value`.
    pub discarded_values: u64,
}

impl SchemaCounters {
    /// Accumulates `other` into `self`.
    pub fn add(&mut self, other: Self) {
        self.discarded_rows += other.discarded_rows;
        self.discarded_values += other.discarded_values;
    }
}

/// A prepared Record's typed column values and optional `_goodfetch_meta`.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRow {
    /// Normalized column names to typed values.
    pub columns: BTreeMap<String, ColumnValue>,
    /// Optional `_goodfetch_meta` JSON object (e.g. `discarded_values`).
    pub meta: Option<Value>,
}

/// What preparing one Record against a [`TableSchema`] decided.
#[derive(Clone, Debug, PartialEq)]
pub enum RowOutcome {
    /// Write this row to the destination.
    Write(PreparedRow),
    /// Drop the whole row (`discard_row`).
    DiscardRow,
    /// Refuse the row (`freeze`), carrying the reason for dead-lettering.
    Refuse(String),
}

/// Normalizes `raw` to an ASCII `snake_case` SQL column identifier,
/// collapsing separators and splitting CamelCase boundaries (ADR 0026).
/// Leading underscores are trimmed so user columns never collide with
/// `_goodfetch_*` system columns; empty or digit-leading names are
/// prefixed with `f_`.
#[must_use]
pub fn normalize_name(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len() + 4);
    let mut prev_was_sep = true;
    let mut prev_was_lower_or_digit = false;
    let mut prev_was_upper = false;

    for (index, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_alphanumeric() {
            let next_is_lower = chars.get(index + 1).is_some_and(char::is_ascii_lowercase);
            if ch.is_ascii_uppercase()
                && !prev_was_sep
                && (prev_was_lower_or_digit || (prev_was_upper && next_is_lower))
            {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_was_sep = false;
            prev_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            prev_was_upper = ch.is_ascii_uppercase();
        } else if !prev_was_sep {
            out.push('_');
            prev_was_sep = true;
            prev_was_lower_or_digit = false;
            prev_was_upper = false;
        }
    }

    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() || trimmed.as_bytes()[0].is_ascii_digit() {
        format!("f_{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

/// A destination table's evolving column schema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableSchema {
    columns: BTreeMap<String, ColumnType>,
    order: Vec<String>,
    raw_to_column: BTreeMap<String, String>,
    established: bool,
}

/// Output of [`TableSchema::prepare_batch`]: prepared rows paired with their
/// source [`Delivered`] metadata, plus any [`SchemaChange`]s to apply.
pub type PreparedBatch<'a> = (Vec<(&'a Delivered, Option<PreparedRow>)>, Vec<SchemaChange>);

impl TableSchema {
    /// Reconstructs a schema from existing non-system columns in a
    /// destination table.
    #[must_use]
    pub fn from_existing(existing: &[(String, ColumnType)]) -> Self {
        let mut schema = Self::default();
        for (name, column_type) in existing {
            schema.insert_column(name.clone(), *column_type);
            schema.raw_to_column.insert(name.clone(), name.clone());
        }
        schema.established = !existing.is_empty();
        schema
    }

    /// The table's user columns in deterministic declaration/first-seen
    /// order.
    #[must_use]
    pub fn columns(&self) -> Vec<(&str, ColumnType)> {
        self.order
            .iter()
            .filter_map(|name| self.columns.get(name).map(|ty| (name.as_str(), *ty)))
            .collect()
    }

    /// Looks up a column's type by normalized name.
    #[must_use]
    pub fn column_type(&self, name: &str) -> Option<ColumnType> {
        self.columns.get(name).copied()
    }

    /// Applies a Dataset's declared JSON Schema (ADR 0026: declared schema
    /// comes first), adding any declared properties not yet present and
    /// marking the schema established.
    pub fn apply_declared_schema(&mut self, schema: &Value, flatten: bool) -> Vec<SchemaChange> {
        let mut fields = Vec::new();
        collect_json_schema_fields(schema, "", flatten, &mut fields);
        let mut changes = Vec::new();
        for (raw_path, column_type) in fields {
            let col_name = self.assign_column_name(&raw_path);
            if !self.columns.contains_key(&col_name) {
                self.insert_column(col_name.clone(), column_type);
                changes.push(SchemaChange::AddedColumn {
                    name: col_name,
                    column_type,
                });
            }
        }
        if !self.columns.is_empty() {
            self.established = true;
        }
        changes
    }

    /// Prepares all Records in `batch` against this schema, applying any
    /// declared schema once per Dataset and checking keyed write-mode
    /// preconditions.
    ///
    /// # Errors
    ///
    /// Returns a human-readable rejection reason if a Record violates the
    /// key requirement or the active [`SchemaContract`].
    pub fn prepare_batch<'a>(
        &mut self,
        schema_applied_for: &mut BTreeMap<String, bool>,
        batch: &'a Batch,
        mode: WriteMode,
        contract: SchemaContract,
        flatten: bool,
    ) -> Result<PreparedBatch<'a>, String> {
        let mut pending_changes = Vec::new();
        if let Some(declared) = &batch.declared_schema
            && !schema_applied_for
                .get(&batch.dataset)
                .copied()
                .unwrap_or(false)
        {
            pending_changes.extend(self.apply_declared_schema(declared, flatten));
            schema_applied_for.insert(batch.dataset.clone(), true);
        }

        let mut prepared = Vec::with_capacity(batch.records.len());
        for record in &batch.records {
            if mode != WriteMode::Append && record.key.is_none() {
                return Err("record missing required key fields".to_owned());
            }
            let (outcome, changes, _) = self.prepare_record(&record.value, contract, flatten);
            pending_changes.extend(changes);
            match outcome {
                RowOutcome::Write(row) => prepared.push((record, Some(row))),
                RowOutcome::DiscardRow => prepared.push((record, None)),
                RowOutcome::Refuse(reason) => return Err(reason),
            }
        }
        Ok((prepared, pending_changes))
    }

    /// Prepares one Record against this schema under `contract` and
    /// `flatten`, returning its outcome, any schema changes to apply, and
    /// any discard counters.
    pub fn prepare_record(
        &mut self,
        value: &Value,
        contract: SchemaContract,
        flatten: bool,
    ) -> (RowOutcome, Vec<SchemaChange>, SchemaCounters) {
        let fields = extract_fields(value, flatten);
        // When no schema was declared or loaded yet, the first Record
        // establishes the inferred schema before non-evolving contracts
        // enforce it on subsequent Records.
        let effective = if self.established {
            contract
        } else {
            SchemaContract::Evolve
        };
        match effective {
            SchemaContract::Evolve => self.prepare_evolve(&fields),
            SchemaContract::Freeze => self.prepare_strict(&fields, true),
            SchemaContract::DiscardRow => self.prepare_strict(&fields, false),
            SchemaContract::DiscardValue => self.prepare_discard_value(&fields),
        }
    }

    fn prepare_evolve(
        &mut self,
        fields: &[(String, Value)],
    ) -> (RowOutcome, Vec<SchemaChange>, SchemaCounters) {
        let mut changes = Vec::new();
        let mut row = BTreeMap::new();
        for (raw, value) in fields {
            let Some(val_type) = ColumnType::of_value(value) else {
                if let Some(col_name) = self.existing_column_for(raw) {
                    row.insert(col_name, ColumnValue::Null);
                }
                continue;
            };
            let col_name = self.assign_column_name(raw);
            match self.columns.get(&col_name).copied() {
                None => {
                    self.insert_column(col_name.clone(), val_type);
                    changes.push(SchemaChange::AddedColumn {
                        name: col_name.clone(),
                        column_type: val_type,
                    });
                    row.insert(col_name, ColumnValue::from_json(value, val_type));
                }
                Some(existing) if existing.accepts(val_type) => {
                    row.insert(col_name, ColumnValue::from_json(value, existing));
                }
                Some(existing) => {
                    if let Some(widened) = existing.widen(val_type) {
                        self.columns.insert(col_name.clone(), widened);
                        changes.push(SchemaChange::WidenedColumn {
                            name: col_name.clone(),
                            from: existing,
                            to: widened,
                        });
                        row.insert(col_name, ColumnValue::from_json(value, widened));
                    } else {
                        let variant = format!("{col_name}__v_{}", val_type.variant_suffix());
                        if !self.columns.contains_key(&variant) {
                            self.insert_column(variant.clone(), val_type);
                            changes.push(SchemaChange::AddedVariantColumn {
                                base: col_name,
                                name: variant.clone(),
                                column_type: val_type,
                            });
                        }
                        row.insert(variant, ColumnValue::from_json(value, val_type));
                    }
                }
            }
        }
        if !self.columns.is_empty() {
            self.established = true;
        }
        (
            RowOutcome::Write(PreparedRow {
                columns: row,
                meta: None,
            }),
            changes,
            SchemaCounters::default(),
        )
    }

    fn prepare_strict(
        &self,
        fields: &[(String, Value)],
        refuse: bool,
    ) -> (RowOutcome, Vec<SchemaChange>, SchemaCounters) {
        let mut row = BTreeMap::new();
        for (raw, value) in fields {
            let Some(col_name) = self.existing_column_for(raw) else {
                return if refuse {
                    (
                        RowOutcome::Refuse(format!("field `{raw}` is not in the frozen schema")),
                        Vec::new(),
                        SchemaCounters::default(),
                    )
                } else {
                    (
                        RowOutcome::DiscardRow,
                        Vec::new(),
                        SchemaCounters {
                            discarded_rows: 1,
                            discarded_values: 0,
                        },
                    )
                };
            };
            let existing = self.columns[&col_name];
            match ColumnType::of_value(value) {
                None => {
                    row.insert(col_name, ColumnValue::Null);
                }
                Some(val_type) if existing.accepts(val_type) => {
                    row.insert(col_name, ColumnValue::from_json(value, existing));
                }
                Some(val_type) => {
                    return if refuse {
                        (
                            RowOutcome::Refuse(format!(
                                "field `{raw}` has type `{}` which conflicts with frozen column `{col_name}` ({})",
                                val_type.variant_suffix(),
                                existing.variant_suffix(),
                            )),
                            Vec::new(),
                            SchemaCounters::default(),
                        )
                    } else {
                        (
                            RowOutcome::DiscardRow,
                            Vec::new(),
                            SchemaCounters {
                                discarded_rows: 1,
                                discarded_values: 0,
                            },
                        )
                    };
                }
            }
        }
        (
            RowOutcome::Write(PreparedRow {
                columns: row,
                meta: None,
            }),
            Vec::new(),
            SchemaCounters::default(),
        )
    }

    fn prepare_discard_value(
        &self,
        fields: &[(String, Value)],
    ) -> (RowOutcome, Vec<SchemaChange>, SchemaCounters) {
        let mut row = BTreeMap::new();
        let mut discarded = Vec::new();
        for (raw, value) in fields {
            let Some(col_name) = self.existing_column_for(raw) else {
                discarded.push(raw.clone());
                continue;
            };
            let existing = self.columns[&col_name];
            match ColumnType::of_value(value) {
                None => {
                    row.insert(col_name, ColumnValue::Null);
                }
                Some(val_type) if existing.accepts(val_type) => {
                    row.insert(col_name, ColumnValue::from_json(value, existing));
                }
                Some(_) => {
                    discarded.push(raw.clone());
                }
            }
        }
        let counters = SchemaCounters {
            discarded_rows: 0,
            discarded_values: discarded.len() as u64,
        };
        let meta = if discarded.is_empty() {
            None
        } else {
            Some(json!({ "discarded_values": discarded }))
        };
        (
            RowOutcome::Write(PreparedRow { columns: row, meta }),
            Vec::new(),
            counters,
        )
    }

    fn insert_column(&mut self, name: String, column_type: ColumnType) {
        if !self.columns.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.columns.insert(name, column_type);
    }

    /// Resolves `raw` to an already-existing column name without creating a
    /// new mapping.
    fn existing_column_for(&self, raw: &str) -> Option<String> {
        if let Some(mapped) = self.raw_to_column.get(raw)
            && self.columns.contains_key(mapped)
        {
            return Some(mapped.clone());
        }
        let normalized = normalize_name(raw);
        if self.columns.contains_key(&normalized)
            && !self
                .raw_to_column
                .iter()
                .any(|(other_raw, col)| other_raw != raw && col == &normalized)
        {
            return Some(normalized);
        }
        None
    }

    /// Maps `raw` to its normalized column name, suffixing collisions
    /// (`_2`, `_3`, …) when another raw field already claimed that name.
    fn assign_column_name(&mut self, raw: &str) -> String {
        if let Some(mapped) = self.raw_to_column.get(raw) {
            return mapped.clone();
        }
        let base = normalize_name(raw);
        let claimed: BTreeSet<&str> = self
            .raw_to_column
            .values()
            .map(String::as_str)
            .chain(self.columns.keys().map(String::as_str))
            .collect();
        let chosen = if claimed.contains(base.as_str()) {
            let mut suffix = 2u64;
            loop {
                let candidate = format!("{base}_{suffix}");
                if !claimed.contains(candidate.as_str()) {
                    break candidate;
                }
                suffix += 1;
            }
        } else {
            base
        };
        self.raw_to_column.insert(raw.to_owned(), chosen.clone());
        chosen
    }
}

/// Extracts a Record's fields as `(raw_path, value)` pairs; flattens nested
/// non-empty objects when `flatten` is true.
fn extract_fields(value: &Value, flatten: bool) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    match value {
        Value::Object(map) => {
            for (key, field_val) in map {
                push_field(key, field_val, flatten, &mut out);
            }
        }
        other => out.push(("value".to_owned(), other.clone())),
    }
    out
}

fn push_field(path: &str, value: &Value, flatten: bool, out: &mut Vec<(String, Value)>) {
    if flatten
        && let Value::Object(map) = value
        && !map.is_empty()
    {
        for (child, child_val) in map {
            let nested = format!("{path}.{child}");
            push_field(&nested, child_val, flatten, out);
        }
        return;
    }
    out.push((path.to_owned(), value.clone()));
}

/// Collects `(raw_path, ColumnType)` entries from a JSON Schema's
/// `properties`.
fn collect_json_schema_fields(
    schema: &Value,
    prefix: &str,
    flatten: bool,
    out: &mut Vec<(String, ColumnType)>,
) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, prop_schema) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if flatten
            && prop_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|nested| !nested.is_empty())
        {
            collect_json_schema_fields(prop_schema, &path, flatten, out);
            continue;
        }
        if let Some(column_type) = json_schema_type(prop_schema) {
            out.push((path, column_type));
        }
    }
}

fn json_schema_type(schema: &Value) -> Option<ColumnType> {
    match schema.get("type") {
        Some(Value::String(ty)) => map_schema_type(ty),
        Some(Value::Array(types)) => {
            let non_null: Vec<ColumnType> = types
                .iter()
                .filter_map(Value::as_str)
                .filter(|ty| *ty != "null")
                .filter_map(map_schema_type)
                .collect();
            if non_null.len() == 1 {
                Some(non_null[0])
            } else {
                None
            }
        }
        _ => None,
    }
}

fn map_schema_type(ty: &str) -> Option<ColumnType> {
    match ty {
        "boolean" => Some(ColumnType::Bool),
        "integer" => Some(ColumnType::Integer),
        "number" => Some(ColumnType::Float),
        "string" => Some(ColumnType::Text),
        "object" | "array" => Some(ColumnType::Json),
        _ => None,
    }
}
