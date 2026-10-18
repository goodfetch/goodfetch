//! The committed JSON Schema bundle and catalog file, emitted from every
//! module's entries through schemars (ADR 0038). The bundle holds every
//! type's schema under `$defs`; the catalog lists every entry with its
//! Product mark (ADR 0095) and refers to its types in the bundle.
//!
//! The bundle's `files` maps each repository file a contract shapes to its
//! schema: `compat/releases.toml` (ADR 0066) so far. What later decisions put
//! in the bundle joins here once its types exist, through the same `xtask
//! codegen`: the format registry (ADR 0066), the `--json` output types and the
//! exit-code table (ADR 0069), and the Journey and Feature inventory schemas
//! (ADRs 0063 and 0072). `xtask compat` reads the format registry as the
//! bundle's `formats`: an array of `{name, version, storage, schema}`, where
//! `storage` is `json`, `sqlite` or `transcript`; without it, no format is
//! registered. A new catalog section
//! needs no change in `xtask`: each Product's projection, the names only
//! Foxhound carries and the containment check read every section.

use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use crate::catalog::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, Metric, Operation,
    OperationClass, PROTOCOL_VERSION, PluginKind, Product, ProtocolVersion, Stability,
};
use crate::releases::ReleasesFile;
use crate::{foxhound, sample};

/// Every module that defines entries.
const MODULES: [&Entries; 2] = [&sample::ENTRIES, &foxhound::ENTRIES];

/// The two committed files, as JSON values.
pub struct Emitted {
    /// The JSON Schema bundle.
    pub bundle: Value,
    /// The catalog file.
    pub catalog: Value,
}

/// Emits the bundle and catalog of every entry this crate defines.
pub fn emit() -> Emitted {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let mut sections = Sections::default();
    for entries in MODULES {
        sections.add(entries, &mut generator);
    }
    let releases = generator.subschema_for::<ReleasesFile>();
    let definitions = generator.take_definitions(true);
    Emitted {
        bundle: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Goodfetch protocol",
            "protocol_version": version(PROTOCOL_VERSION),
            "files": { "compat/releases.toml": releases },
            "$defs": definitions,
        }),
        catalog: sections.into_catalog(),
    }
}

#[derive(Default)]
struct Sections {
    operations: Vec<Value>,
    events: Vec<Value>,
    error_codes: Vec<Value>,
    plugin_kinds: Vec<Value>,
    metrics: Vec<Value>,
}

impl Sections {
    fn add(&mut self, entries: &Entries, generator: &mut SchemaGenerator) {
        self.operations.extend(
            entries
                .operations
                .iter()
                .map(|operation| operation_entry(operation, generator)),
        );
        self.events.extend(
            entries
                .events
                .iter()
                .map(|event| event_entry(event, generator)),
        );
        self.error_codes
            .extend(entries.error_codes.iter().map(error_code_entry));
        self.plugin_kinds.extend(
            entries
                .plugin_kinds
                .iter()
                .map(|kind| plugin_kind_entry(kind, generator)),
        );
        self.metrics
            .extend(entries.metrics.iter().map(metric_entry));
    }

    fn into_catalog(self) -> Value {
        json!({
            "protocol_version": version(PROTOCOL_VERSION),
            "operations": sorted(self.operations),
            "events": sorted(self.events),
            "error_groups": ErrorGroup::ALL.iter().map(|group| error_group_entry(*group)).collect::<Vec<_>>(),
            "error_codes": sorted(self.error_codes),
            "plugin_kinds": sorted(self.plugin_kinds),
            "metrics": sorted(self.metrics),
        })
    }
}

/// Entries sorted by name, so the files never change with declaration order.
fn sorted(mut entries: Vec<Value>) -> Vec<Value> {
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    entries
}

fn version(version: ProtocolVersion) -> String {
    format!("{}.{}", version.major, version.minor)
}

fn product(defined_in: &str) -> &'static str {
    Product::of_module(defined_in).as_str()
}

fn stability(stability: Stability) -> &'static str {
    match stability {
        Stability::Stable => "stable",
        Stability::Experimental => "experimental",
    }
}

fn deprecation(deprecation: Option<Deprecation>) -> Value {
    deprecation.map_or(
        Value::Null,
        |deprecation| json!({ "since": version(deprecation.since), "note": deprecation.note }),
    )
}

fn class(class: OperationClass) -> &'static str {
    match class {
        OperationClass::Read => "read",
        OperationClass::IdempotentWrite => "idempotent_write",
        OperationClass::Effect => "effect",
    }
}

fn operation_entry(operation: &Operation, generator: &mut SchemaGenerator) -> Value {
    json!({
        "name": operation.name,
        "product": product(operation.defined_in),
        "description": operation.description,
        "class": class(operation.class),
        "params": (operation.params)(generator),
        "result": (operation.result)(generator),
        "events": operation.events,
        "cancellable": operation.cancellable,
        "stability": stability(operation.stability),
        "deprecated": deprecation(operation.deprecated),
        "arrived_in": version(operation.arrived_in),
    })
}

fn event_entry(event: &Event, generator: &mut SchemaGenerator) -> Value {
    json!({
        "name": event.name,
        "product": product(event.defined_in),
        "description": event.description,
        "payload": (event.payload)(generator),
        "stability": stability(event.stability),
        "deprecated": deprecation(event.deprecated),
        "arrived_in": version(event.arrived_in),
    })
}

fn error_group_entry(group: ErrorGroup) -> Value {
    json!({
        "name": group.as_str(),
        "product": Product::Collie.as_str(),
        "description": group.description(),
        "json_rpc_class": group.json_rpc_class(),
    })
}

fn error_code_entry(code: &ErrorCodeDefinition) -> Value {
    json!({
        "name": code.name,
        "product": product(code.defined_in),
        "description": code.description,
        "group": code.group.as_str(),
        "json_rpc_class": code.group.json_rpc_class(),
        "retryable": code.retryable,
        "arrived_in": version(code.arrived_in),
    })
}

fn plugin_kind_entry(kind: &PluginKind, generator: &mut SchemaGenerator) -> Value {
    let methods: Vec<Value> = kind
        .methods
        .iter()
        .map(|method| {
            json!({
                "name": method.name,
                "description": method.description,
                "params": (method.params)(generator),
                "result": (method.result)(generator),
            })
        })
        .collect();
    json!({
        "name": kind.name,
        "product": product(kind.defined_in),
        "description": kind.description,
        "methods": methods,
        "stability": stability(kind.stability),
        "arrived_in": version(kind.arrived_in),
    })
}

fn metric_entry(metric: &Metric) -> Value {
    json!({
        "name": metric.name,
        "product": product(metric.defined_in),
        "description": metric.description,
        "instrument": metric.instrument.as_str(),
        "unit": metric.unit,
        "stability": stability(metric.stability),
        "arrived_in": version(metric.arrived_in),
    })
}

#[cfg(test)]
mod tests;
