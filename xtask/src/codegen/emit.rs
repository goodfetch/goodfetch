//! The committed JSON Schema bundle and catalog file, emitted from every
//! module of `goodfetch-protocol`'s entries through schemars (ADR 0038). The
//! bundle holds every type's schema under `$defs`; the catalog lists every
//! entry with its Product mark (ADR 0095) and refers to its types in the
//! bundle.
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

use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::browser_cache::{BrowserCleaned, BrowserInstalled};
use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::catalog::sections::Sections;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::cli::{CommandFailed, ExitCodeRow, exit_code_table};
use goodfetch_protocol::connection::{CancelParams, HelloParams, HelloResult};
use goodfetch_protocol::console::ConsoleOpened;
use goodfetch_protocol::daemon::{
    DaemonHealthOutput, DaemonStatus, DaemonUninstallOutput, HttpListenerToken, TokenRotated,
};
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::events::{CreditParams, EventBatch};
use goodfetch_protocol::formats::FORMATS;
use goodfetch_protocol::foxhound::dataset::RecordRead;
use goodfetch_protocol::foxhound::frontier::{EnqueuedBatch, SeedBatch};
use goodfetch_protocol::journey::{Journey, JourneyCatalog};
use goodfetch_protocol::jsonrpc::{META, TraceContext};
use goodfetch_protocol::key_value_store::KeyValueStoreKey;
use goodfetch_protocol::log::LogEvent;
use goodfetch_protocol::page::Chunk;
use goodfetch_protocol::releases::ReleasesFile;
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_protocol::run::RunsReindexOutput;
use goodfetch_protocol::settings::SettingsExplanation;
use goodfetch_protocol::versions::FilesUpgraded;

/// The bundle's `files` key of every Journey file, `journeys/J-NNN.toml`.
pub(super) const JOURNEY_FILES: &str = "journeys/J-*.toml";

/// The two committed files, as JSON values.
pub(super) struct Emitted {
    /// The JSON Schema bundle.
    pub(super) bundle: Value,
    /// The catalog file.
    pub(super) catalog: Value,
}

/// Emits the bundle and catalog of every entry `goodfetch-protocol` defines.
pub(super) fn emit() -> Emitted {
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
    let exit_codes: Vec<Value> = exit_code_table().iter().map(exit_code_entry).collect();
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
        catalog: sections.into_catalog(&exit_codes),
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

#[cfg(test)]
mod tests;
