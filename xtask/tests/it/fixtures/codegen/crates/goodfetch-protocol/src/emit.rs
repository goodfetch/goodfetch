//! The committed JSON Schema bundle and catalog file, emitted from every
//! module's entries through schemars (ADR 0038). The bundle holds every
//! type's schema under `$defs`; the catalog lists every entry with its
//! Product mark (ADR 0095) and refers to its types in the bundle. Its
//! catalog comes from `goodfetch-protocol`'s own `catalog` module, which
//! this fixture holds as it is.

use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use crate::catalog::sections::Sections;
use crate::catalog::{Entries, PROTOCOL_VERSION, Setting, SettingScope};
use crate::journey::Journey;
use crate::result_document::{CaseLogEntry, ResultDocument};
use crate::{foxhound, sample};

/// Every module that defines entries.
const MODULES: [&Entries; 2] = [&sample::ENTRIES, &foxhound::ENTRIES];

/// The scopes a program's code sets each of this crate's settings at: the
/// connection and a Run.
fn scopes_of(_setting: &Setting) -> &'static [SettingScope] {
    &[SettingScope::Connection, SettingScope::Run]
}

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
    let mut sections = Sections::new(scopes_of);
    for entries in MODULES {
        sections.add(entries, &mut generator);
    }
    let journey = generator.subschema_for::<Journey>();
    let result_document = generator.subschema_for::<ResultDocument>();
    let case_log_entry = generator.subschema_for::<CaseLogEntry>();
    let hello = generator.subschema_for::<sample::HelloParams>();
    let definitions = generator.take_definitions(true);
    Emitted {
        bundle: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Goodfetch protocol",
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "files": { "journeys/J-*.toml": journey },
            "formats": [{
                "name": "sample-store",
                "version": "0.1",
                "storage": "sqlite",
                "rule": "default",
                "hello_from": [],
                "schema": { "type": "object" },
            }],
            "connection": { "hello": { "params": hello } },
            "documents": {
                "result_document": result_document,
                "case_log_entry": case_log_entry,
            },
            "$defs": definitions,
        }),
        catalog: sections.into_catalog(&[], &[]),
    }
}
