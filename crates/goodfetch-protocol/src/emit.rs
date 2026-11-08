//! The committed JSON Schema bundle and catalog file, emitted from every
//! module's entries through schemars (ADR 0038). The bundle holds every
//! type's schema under `$defs`; the catalog lists every entry with its
//! Product mark (ADR 0095) and refers to its types in the bundle.
//!
//! The bundle's `files` maps each repository file a contract shapes to its
//! schema: `compat/releases.toml` (ADR 0066), and the Journey catalog's
//! files, every `journeys/J-*.toml` and `journeys/catalog.toml` (ADR 0063),
//! whose Journey schema `xtask codegen` also writes on its own to
//! `journeys/journey.schema.json`. Its `documents` holds the schemas of the
//! documents tests write rather than files the repository keeps: the result
//! document and the case log's entry (ADRs 0063 and 0069). What later
//! decisions put in the bundle
//! joins here once its types exist, through the same `xtask codegen`: the
//! Feature inventory schemas (ADR 0072). The catalog's `exit_codes` is ADR
//! 0069's exit-code table by exit code, each row `{name, product, code,
//! description}`, whose names are the `exit:` Features.
//! The bundle's `outputs` maps each hand-written command's path, such as
//! `daemon.start`, to the type of its `--json` output (ADR 0069); a command
//! generated from an operation prints the operation's `result`, and every
//! command that fails prints the bundle's `command_failed`. The
//! format registry is the bundle's `formats`, which `xtask compat` reads: an
//! array of `{name, version, storage, rule, hello_from, schema}`, where
//! `storage` is `json`, `sqlite`, `toml` or `transcript`, and `hello_from`
//! lists who opens a protocol with `hello`. The bundle's `connection` holds the
//! types of `hello` and `$/cancel`, which open and govern every Connection
//! outside the catalog, and those of the streaming calls' notifications,
//! the Client's `$/credit` and the engine's `$/events` and `$/chunk`, and
//! `meta`, the Trace context that every request's params type holds as
//! `_meta`, which emitting adds to each of them (ADR 0132); each Error code's entry carries its row of the
//! exit-code table, `exit_code`. A new catalog section
//! needs no change in `xtask`: each Product's projection, the names only
//! Foxhound carries and the containment check read every section.

use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use crate::block::BlockCause;
use crate::browser_cache::{BrowserCleaned, BrowserInstalled};
use crate::case_log::CaseLogEntry;
use crate::catalog::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, HandlerRequest, HookStage,
    Metric, Operation, PROTOCOL_VERSION, PluginKind, Product, Setting, SharedTypeValue,
};
use crate::cli::{CommandFailed, ExitCodeRow, exit_code_table};
use crate::connection::{CancelParams, HelloParams, HelloResult};
use crate::console::ConsoleOpened;
use crate::daemon::{
    DaemonHealthOutput, DaemonStatus, DaemonUninstallOutput, HttpListenerToken, TokenRotated,
};
use crate::entries::MODULES;
use crate::events::{CreditParams, EventBatch};
use crate::formats::FORMATS;
use crate::foxhound::dataset::RecordRead;
use crate::foxhound::frontier::{EnqueuedBatch, SeedBatch};
use crate::journey::{Journey, JourneyCatalog};
use crate::jsonrpc::{META, TraceContext};
use crate::key_value_store::KeyValueStoreKey;
use crate::log::LogEvent;
use crate::page::Chunk;
use crate::releases::ReleasesFile;
use crate::result_document::ResultDocument;
use crate::run::RunsReindexOutput;
use crate::settings::SettingsExplanation;
use crate::versions::FilesUpgraded;

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
                "hello_from": format.hello_from.iter().map(|from| from.as_str()).collect::<Vec<_>>(),
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
        "credit": { "params": generator.subschema_for::<CreditParams>() },
        "events": { "params": generator.subschema_for::<EventBatch>() },
        "chunk": { "params": generator.subschema_for::<Chunk>() },
        "log": { "params": generator.subschema_for::<LogEvent>() },
        "key": { "params": generator.subschema_for::<KeyValueStoreKey>() },
        "seeds": { "params": generator.subschema_for::<SeedBatch>() },
        "enqueued": { "params": generator.subschema_for::<EnqueuedBatch>() },
        "record": { "params": generator.subschema_for::<RecordRead>() },
        "meta": generator.subschema_for::<TraceContext>(),
    });
    let outputs = json!({
        "browser.clean": generator.subschema_for::<BrowserCleaned>(),
        "browser.install": generator.subschema_for::<BrowserInstalled>(),
        "config.explain": generator.subschema_for::<SettingsExplanation>(),
        "console": generator.subschema_for::<ConsoleOpened>(),
        "daemon.health": generator.subschema_for::<DaemonHealthOutput>(),
        "daemon.http-listener-token": generator.subschema_for::<HttpListenerToken>(),
        "daemon.install": generator.subschema_for::<DaemonStatus>(),
        "daemon.restart": generator.subschema_for::<DaemonStatus>(),
        "daemon.rotate-token": generator.subschema_for::<TokenRotated>(),
        "daemon.run": generator.subschema_for::<DaemonStatus>(),
        "daemon.start": generator.subschema_for::<DaemonStatus>(),
        "daemon.status": generator.subschema_for::<DaemonStatus>(),
        "daemon.stop": generator.subschema_for::<DaemonStatus>(),
        "daemon.uninstall": generator.subschema_for::<DaemonUninstallOutput>(),
        "runs.reindex": generator.subschema_for::<RunsReindexOutput>(),
        "upgrade-files": generator.subschema_for::<FilesUpgraded>(),
    });
    let command_failed = generator.subschema_for::<CommandFailed>();
    let shared_types = json!({ "block_cause": generator.subschema_for::<BlockCause>() });
    let result_document = generator.subschema_for::<ResultDocument>();
    let case_log_entry = generator.subschema_for::<CaseLogEntry>();
    let mut definitions = generator.take_definitions(true);
    for params in sections
        .request_params()
        .chain([&connection["hello"]["params"]])
    {
        add_meta(&mut definitions, params);
    }
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
            "outputs": outputs,
            "command_failed": command_failed,
            "shared_types": shared_types,
            "documents": {
                "result_document": result_document,
                "case_log_entry": case_log_entry,
            },
            "$defs": definitions,
        }),
        catalog: sections.into_catalog(),
    }
}

#[derive(Default)]
struct Sections {
    operations: Vec<Value>,
    events: Vec<Value>,
    handler_requests: Vec<Value>,
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
        self.handler_requests.extend(
            entries
                .handler_requests
                .iter()
                .map(|request| handler_request_entry(product, request, generator)),
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

    /// The params schema of every request the catalog holds: each
    /// operation's, handler request's and Plugin method's.
    fn request_params(&self) -> impl Iterator<Item = &Value> {
        let methods = self
            .plugin_kinds
            .iter()
            .flat_map(|kind| kind["methods"].as_array().into_iter().flatten());
        self.operations
            .iter()
            .chain(&self.handler_requests)
            .chain(methods)
            .map(|entry| &entry["params"])
    }

    fn into_catalog(self) -> Value {
        json!({
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "operations": sorted(self.operations),
            "events": sorted(self.events),
            "handler_requests": sorted(self.handler_requests),
            "error_groups": ErrorGroup::ALL.iter().map(|group| error_group_entry(*group)).collect::<Vec<_>>(),
            "error_codes": sorted(self.error_codes),
            "exit_codes": exit_code_table().iter().map(exit_code_entry).collect::<Vec<_>>(),
            "plugin_kinds": sorted(self.plugin_kinds),
            "metrics": sorted(self.metrics),
            "settings": sorted(self.settings),
            "hook_stages": sorted(self.hook_stages),
            "shared_type_values": sorted(self.shared_type_values),
        })
    }
}

/// Adds `_meta`, the request's Trace context, to the params type `params`
/// refers to (ADR 0132): an envelope field each runtime fills in, which no
/// Rust type of the params declares.
fn add_meta(definitions: &mut serde_json::Map<String, Value>, params: &Value) {
    let name = params["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
        .expect("invariant: a request's params refer to a type of the bundle");
    let properties = definitions
        .get_mut(name)
        .and_then(Value::as_object_mut)
        .expect("invariant: a request's params type is in the bundle")
        .entry("properties")
        .or_insert_with(|| json!({}));
    properties
        .as_object_mut()
        .expect("invariant: a request's params type is an object")
        .insert(
            META.to_owned(),
            json!({ "$ref": "#/$defs/TraceContext", "arrived_in": "0.1" }),
        );
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
        "kind": operation.kind.as_str(),
        "reach": operation.reach.as_str(),
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
        "class": event.class.as_str(),
        "payload": (event.payload)(generator),
        "stability": event.stability.as_str(),
        "deprecated": deprecation(event.deprecated),
        "arrived_in": event.arrived_in.to_string(),
    })
}

fn handler_request_entry(
    product: Product,
    request: &HandlerRequest,
    generator: &mut SchemaGenerator,
) -> Value {
    json!({
        "name": request.name,
        "product": product.as_str(),
        "description": request.description,
        "params": (request.params)(generator),
        "result": (request.result)(generator),
        "stability": request.stability.as_str(),
        "deprecated": deprecation(request.deprecated),
        "arrived_in": request.arrived_in.to_string(),
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

/// A row of ADR 0069's exit-code table, which both Products' executables
/// share (ADR 0090).
fn exit_code_entry(row: &ExitCodeRow) -> Value {
    json!({
        "name": row.name,
        "product": Product::Collie.as_str(),
        "code": row.code,
        "description": row.description,
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
        "labels": metric
            .labels
            .iter()
            .map(|label| json!({ "name": label.name, "values": (label.values)() }))
            .collect::<Vec<_>>(),
        "stability": metric.stability.as_str(),
        "arrived_in": metric.arrived_in.to_string(),
    })
}

/// A setting's entry in the settings schema: its value's schema, default and
/// authority, and the Products that honour it, read from its mark: both for
/// Collie's, which Foxhound contains, only Foxhound for Foxhound's (ADRs
/// 0087 and 0095).
fn setting_entry(product: Product, setting: &Setting) -> Value {
    let products: Vec<&str> = [Product::Collie, Product::Foxhound]
        .into_iter()
        .filter(|honours| honours.contains(product))
        .map(Product::as_str)
        .collect();
    json!({
        "name": setting.name,
        "product": product.as_str(),
        "products": products,
        "description": setting.description,
        "value": setting.kind.schema(),
        "default": (setting.default)(),
        "clearable": setting.clearable,
        "authority": setting.authority.as_str(),
        "weakens": setting.weakens.is_some(),
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
