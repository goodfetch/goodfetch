//! The format registry (ADR 0066): every versioned format an engine reads
//! or a peer speaks, with its current `major.minor`, how its Compatibility
//! corpus stores it, its schema and the rule of ADR 0030 it follows, and one
//! compatibility function per rule, which decides each pair of versions.
//! `xtask codegen` writes the registry into the bundle as `formats`, which
//! `xtask compat` reads.

use schemars::{Schema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::browser_cache::BrowserCacheEntry;
use crate::catalog::{PROTOCOL_VERSION, ProtocolVersion, SchemaOf, Setting, ValueKind, schema_of};
use crate::entries::MODULES;
use crate::jsonrpc::Message;
use crate::run::{RunIndexRow, RunStateRow};

/// How a format's files are stored in its corpus sets (ADR 0066).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// JSON documents.
    Json,
    /// A SQLite-backed store, as SQL text dumps.
    Sqlite,
    /// A protocol's recorded transcripts, one message per line.
    Transcript,
    /// TOML documents, such as settings files.
    Toml,
}

impl Storage {
    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Sqlite => "sqlite",
            Self::Transcript => "transcript",
            Self::Toml => "toml",
        }
    }
}

/// A rule of ADR 0030 by which a reader treats another version of its
/// format. Each later rule (the frozen-major family of the Usage ledger, the
/// Response cache's, the Clean-up rules') joins with the first format that
/// follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A reader reads its own major up to its own minor.
    Default,
}

impl Rule {
    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
        }
    }

    /// What a reader at `reader` makes of a file, or a peer's `hello`, at
    /// `found`.
    pub fn compatibility(self, reader: ProtocolVersion, found: ProtocolVersion) -> Compatibility {
        match self {
            Self::Default => {
                if found.major < reader.major {
                    Compatibility::OlderMajor
                } else if found.major > reader.major {
                    Compatibility::NewerMajor
                } else if found.minor > reader.minor {
                    Compatibility::NewerMinor
                } else {
                    Compatibility::Compatible
                }
            }
        }
    }
}

/// A compatibility function's verdict on one pair of versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    /// The reader reads it.
    Compatible,
    /// It is of a newer minor than the reader knows: upgrade the reader.
    NewerMinor,
    /// It is of an older major: upgrade it (`upgrade-files` for a file, a
    /// newer Client for a protocol).
    OlderMajor,
    /// It is of a newer major: upgrade the reader.
    NewerMajor,
}

/// One registered format.
pub struct Format {
    /// Its name, the folder of its corpus under `compat/`.
    pub name: &'static str,
    /// Its current version.
    pub version: ProtocolVersion,
    /// How its corpus stores it.
    pub storage: Storage,
    /// The rule it follows.
    pub rule: Rule,
    /// The type of one document, row or message.
    pub schema: SchemaOf,
}

/// The JSON-RPC protocol every Client speaks to an engine (ADRs 0024 and
/// 0066), at the protocol's version.
pub const PROTOCOL: Format = Format {
    name: "protocol",
    version: PROTOCOL_VERSION,
    storage: Storage::Transcript,
    rule: Rule::Default,
    schema: schema_of::<Message>,
};

/// The Browser cache's entries (ADRs 0066 and 0098), one `entry.json` per
/// Pinned artifact: an engine reads its own major up to its own minor, and
/// never deletes an entry it cannot read.
pub const BROWSER_CACHE: Format = Format {
    name: "browser-cache",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::Default,
    schema: schema_of::<BrowserCacheEntry>,
};

/// A Run's own SQLite file (ADRs 0002 and 0031): its metadata row, its
/// tags and the tables each Product's composition adds, such as a crawl's
/// work items and Records.
pub const RUN_STATE: Format = Format {
    name: "run-state",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::Default,
    schema: schema_of::<RunStateRow>,
};

/// The per-user, per-major Run index (ADR 0031): one row per Run.
pub const RUN_INDEX: Format = Format {
    name: "run-index",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::Default,
    schema: schema_of::<RunIndexRow>,
};

/// The per-major Operator policy of Goodfetch home (ADRs 0028 and 0098):
/// what it sets, locks and caps, nested by key, read by both Products.
pub const OPERATOR_POLICY: Format = Format {
    name: "operator-policy",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Toml,
    rule: Rule::Default,
    schema: operator_policy_schema,
};

/// The per-major user configuration of Goodfetch home (ADRs 0028 and
/// 0098), whose shape a project configuration shares: the settings it
/// sets, nested by key, read by both Products.
pub const USER_CONFIGURATION: Format = Format {
    name: "user-configuration",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Toml,
    rule: Rule::Default,
    schema: configuration_schema,
};

/// Every registered format.
pub const FORMATS: &[&Format] = &[
    &PROTOCOL,
    &BROWSER_CACHE,
    &RUN_INDEX,
    &RUN_STATE,
    &OPERATOR_POLICY,
    &USER_CONFIGURATION,
];

/// A settings file's schema: its `version` and each of `tables`, a table
/// of every key of either Product nested by its segments.
/// One table of a settings file: its name, what it holds and which keys.
struct Table {
    name: &'static str,
    description: &'static str,
    holds: fn(&Setting) -> bool,
}

fn settings_file_schema(generator: &mut SchemaGenerator, tables: &[Table]) -> Schema {
    let mut properties = Map::new();
    properties.insert(
        "version".to_owned(),
        json!({
            "allOf": [generator.subschema_for::<ProtocolVersion>()],
            "description": "The version of this format the file was written in.",
            "arrived_in": "0.1",
        }),
    );
    for table in tables {
        let mut schema = settings_table(table.holds);
        schema["description"] = json!(table.description);
        schema["arrived_in"] = json!("0.1");
        properties.insert(table.name.to_owned(), schema);
    }
    Schema::try_from(json!({
        "type": "object",
        "properties": properties,
        "required": ["version"],
        "additionalProperties": false,
    }))
    .expect("invariant: a settings file's schema is an object")
}

fn configuration_schema(generator: &mut SchemaGenerator) -> Schema {
    settings_file_schema(
        generator,
        &[Table {
            name: "settings",
            description: "The settings it sets, nested by key.",
            holds: any_key,
        }],
    )
}

fn operator_policy_schema(generator: &mut SchemaGenerator) -> Schema {
    settings_file_schema(
        generator,
        &[
            Table {
                name: "settings",
                description: "The values it sets, which higher layers may change.",
                holds: any_key,
            },
            Table {
                name: "lock",
                description: "The values it sets and locks: no higher layer changes them.",
                holds: any_key,
            },
            Table {
                name: "cap",
                description: "The largest value a higher layer may give a count or a duration.",
                holds: bounded_key,
            },
        ],
    )
}

fn any_key(_: &Setting) -> bool {
    true
}

/// A key a cap bounds: a count or a duration.
fn bounded_key(setting: &Setting) -> bool {
    matches!(setting.kind, ValueKind::Count | ValueKind::Duration)
}

/// Every key of either Product that `holds`, nested by its segments, its
/// last holding a value of the key's kind.
fn settings_table(holds: fn(&Setting) -> bool) -> Value {
    let mut tree = Map::new();
    for entries in MODULES {
        for setting in entries.settings.iter().filter(|setting| holds(setting)) {
            let mut table = &mut tree;
            let mut segments = setting.name.split('.').peekable();
            while let Some(segment) = segments.next() {
                if segments.peek().is_none() {
                    let mut leaf = setting.kind.schema();
                    if setting.clearable {
                        leaf = json!({ "anyOf": [leaf, { "type": "null" }] });
                    }
                    leaf["description"] = json!(setting.description);
                    table.insert(segment.to_owned(), json!({ "leaf": leaf }));
                } else {
                    table = table
                        .entry(segment.to_owned())
                        .or_insert_with(|| json!({ "branch": {} }))
                        .get_mut("branch")
                        .and_then(Value::as_object_mut)
                        .expect("invariant: no key's segment is another key's last");
                }
            }
        }
    }
    table_schema(&tree)
}

fn table_schema(tree: &Map<String, Value>) -> Value {
    let empty = Map::new();
    let properties: Map<String, Value> = tree
        .iter()
        .map(|(segment, node)| {
            let schema = match node.get("leaf") {
                Some(leaf) => leaf.clone(),
                None => table_schema(
                    node.get("branch")
                        .and_then(Value::as_object)
                        .unwrap_or(&empty),
                ),
            };
            (segment.clone(), schema)
        })
        .collect();
    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests;
