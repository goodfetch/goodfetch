//! The committed JSON Schema bundle and catalog file, emitted from every
//! module's entries through schemars (ADR 0038). The bundle holds every
//! type's schema under `$defs`; the catalog lists every entry with its
//! Product mark (ADR 0095) and refers to its types in the bundle.
//!
//! The bundle's `files` maps each repository file a contract shapes to its
//! schema: `compat/releases.toml` (ADR 0066), and the Journey catalog's
//! files, every `journeys/J-*.toml` and `journeys/catalog.toml` (ADR 0063),
//! whose Journey schema `xtask codegen` also writes on its own to
//! `journeys/journey.schema.json`. What later decisions put in the bundle
//! joins here once its types exist, through the same `xtask codegen`: the
//! format registry (ADR 0066), the `--json` output types and the exit-code
//! table (ADR 0069), and the Feature inventory schemas (ADR 0072). The
//! format registry is the bundle's `formats`, which `xtask compat` reads: an
//! array of `{name, version, storage, rule, schema}`, where `storage` is
//! `json`, `sqlite` or `transcript`. The bundle's `connection` holds the
//! types of `hello` and `$/cancel`, which open and govern every Connection
//! outside the catalog; each Error code's entry carries its row of the
//! exit-code table, `exit_code`. A new catalog section
//! needs no change in `xtask`: each Product's projection, the names only
//! Foxhound carries and the containment check read every section.

use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use crate::catalog::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, HookStage, Metric, Operation,
    PROTOCOL_VERSION, PluginKind, Product, Setting, SharedTypeValue,
};
use crate::connection::{CancelParams, HelloParams, HelloResult};
use crate::entries::MODULES;
use crate::formats::FORMATS;
use crate::journey::{Journey, JourneyCatalog};
use crate::releases::ReleasesFile;

/// The bundle's `files` key of every Journey file, `journeys/J-NNN.toml`.
pub const JOURNEY_FILES: &str = "journeys/J-*.toml";

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
    let journey = generator.subschema_for::<Journey>();
    let journey_catalog = generator.subschema_for::<JourneyCatalog>();
    let formats: Vec<Value> = FORMATS
        .iter()
        .map(|format| {
            json!({
                "name": format.name,
                "version": format.version.to_string(),
                "storage": format.storage.as_str(),
                "rule": format.rule.as_str(),
                "schema": (format.schema)(&mut generator),
            })
        })
        .collect();
    let connection = json!({
        "hello": {
            "params": generator.subschema_for::<HelloParams>(),
            "result": generator.subschema_for::<HelloResult>(),
        },
        "cancel": { "params": generator.subschema_for::<CancelParams>() },
    });
    let definitions = generator.take_definitions(true);
    Emitted {
        bundle: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Goodfetch protocol",
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "files": {
                "compat/releases.toml": releases,
                JOURNEY_FILES: journey,
                "journeys/catalog.toml": journey_catalog,
            },
            "formats": formats,
            "connection": connection,
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
    settings: Vec<Value>,
    hook_stages: Vec<Value>,
    shared_type_values: Vec<Value>,
}

impl Sections {
    fn add(&mut self, entries: &Entries, generator: &mut SchemaGenerator) {
        let product = Product::of_module(entries.defined_in);
        self.operations.extend(
            entries
                .operations
                .iter()
                .map(|operation| operation_entry(product, operation, generator)),
        );
        self.events.extend(
            entries
                .events
                .iter()
                .map(|event| event_entry(product, event, generator)),
        );
        self.error_codes.extend(
            entries
                .error_codes
                .iter()
                .map(|code| error_code_entry(product, code)),
        );
        self.plugin_kinds.extend(
            entries
                .plugin_kinds
                .iter()
                .map(|kind| plugin_kind_entry(product, kind, generator)),
        );
        self.metrics.extend(
            entries
                .metrics
                .iter()
                .map(|metric| metric_entry(product, metric)),
        );
        self.settings.extend(
            entries
                .settings
                .iter()
                .map(|setting| setting_entry(product, setting)),
        );
        self.hook_stages.extend(
            entries
                .hook_stages
                .iter()
                .map(|stage| hook_stage_entry(product, stage)),
        );
        self.shared_type_values.extend(
            entries
                .shared_type_values
                .iter()
                .map(|value| shared_type_value_entry(product, value)),
        );
    }

    fn into_catalog(self) -> Value {
        json!({
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "operations": sorted(self.operations),
            "events": sorted(self.events),
            "error_groups": ErrorGroup::ALL.iter().map(|group| error_group_entry(*group)).collect::<Vec<_>>(),
            "error_codes": sorted(self.error_codes),
            "plugin_kinds": sorted(self.plugin_kinds),
            "metrics": sorted(self.metrics),
            "settings": sorted(self.settings),
            "hook_stages": sorted(self.hook_stages),
            "shared_type_values": sorted(self.shared_type_values),
        })
    }
}

/// Entries sorted by name, so the files never change with declaration order.
fn sorted(mut entries: Vec<Value>) -> Vec<Value> {
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    entries
}

fn deprecation(deprecation: Option<Deprecation>) -> Value {
    deprecation.map_or(
        Value::Null,
        |deprecation| json!({ "since": deprecation.since.to_string(), "note": deprecation.note }),
    )
}

fn operation_entry(
    product: Product,
    operation: &Operation,
    generator: &mut SchemaGenerator,
) -> Value {
    json!({
        "name": operation.name,
        "product": product.as_str(),
        "description": operation.description,
        "class": operation.class.as_str(),
        "params": (operation.params)(generator),
        "result": (operation.result)(generator),
        "events": operation.events.iter().map(|event| event.name).collect::<Vec<_>>(),
        "cancellable": operation.cancellable,
        "stability": operation.stability.as_str(),
        "deprecated": deprecation(operation.deprecated),
        "arrived_in": operation.arrived_in.to_string(),
    })
}

fn event_entry(product: Product, event: &Event, generator: &mut SchemaGenerator) -> Value {
    json!({
        "name": event.name,
        "product": product.as_str(),
        "description": event.description,
        "payload": (event.payload)(generator),
        "stability": event.stability.as_str(),
        "deprecated": deprecation(event.deprecated),
        "arrived_in": event.arrived_in.to_string(),
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

fn error_code_entry(product: Product, code: &ErrorCodeDefinition) -> Value {
    json!({
        "name": code.name,
        "product": product.as_str(),
        "description": code.description,
        "group": code.group.as_str(),
        "json_rpc_class": code.group.json_rpc_class(),
        "retryable": code.retryable,
        "exit_code": code.exit_code,
        "arrived_in": code.arrived_in.to_string(),
    })
}

fn plugin_kind_entry(
    product: Product,
    kind: &PluginKind,
    generator: &mut SchemaGenerator,
) -> Value {
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
        "product": product.as_str(),
        "description": kind.description,
        "methods": methods,
        "stability": kind.stability.as_str(),
        "arrived_in": kind.arrived_in.to_string(),
    })
}

fn metric_entry(product: Product, metric: &Metric) -> Value {
    json!({
        "name": metric.name,
        "product": product.as_str(),
        "description": metric.description,
        "instrument": metric.instrument.as_str(),
        "unit": metric.unit,
        "stability": metric.stability.as_str(),
        "arrived_in": metric.arrived_in.to_string(),
    })
}

fn setting_entry(product: Product, setting: &Setting) -> Value {
    json!({
        "name": setting.name,
        "product": product.as_str(),
        "description": setting.description,
        "arrived_in": setting.arrived_in.to_string(),
    })
}

fn hook_stage_entry(product: Product, stage: &HookStage) -> Value {
    json!({
        "name": stage.name,
        "product": product.as_str(),
        "description": stage.description,
        "arrived_in": stage.arrived_in.to_string(),
    })
}

fn shared_type_value_entry(product: Product, value: &SharedTypeValue) -> Value {
    json!({
        "name": value.name,
        "product": product.as_str(),
        "type": value.shared_type.as_str(),
        "description": value.description,
        "arrived_in": value.arrived_in.to_string(),
    })
}

#[cfg(test)]
mod tests;
