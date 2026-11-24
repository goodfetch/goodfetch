//! The format registry (ADR 0066): every versioned format an engine reads
//! or a peer speaks, with its current `major.minor`, how its Compatibility
//! corpus stores it, its schema and the rule of ADR 0030 it follows, and one
//! compatibility function per rule, which decides each pair of versions.
//! `xtask codegen` writes the registry into the bundle as `formats`, which
//! `xtask compat` reads.

use std::cmp::Ordering;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::audit::AuditEntry;
use crate::browser_cache::BrowserCacheEntry;
use crate::catalog::{PROTOCOL_VERSION, ProtocolVersion, SchemaOf, Setting, ValueKind, schema_of};
use crate::clean_up_rules::CleanUpRulesManifest;
use crate::entries::MODULES;
use crate::executables::InstalledExecutables;
use crate::jsonrpc::Message;
use crate::politeness::PermitRow;
use crate::run::{RunIndexRow, RunStateRow};
use crate::session_export::SessionExport;
use crate::tls::CertificatePin;
use crate::usage::Limit;

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
/// format. Each later rule (the Response cache's) joins with the first
/// format that follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// A reader reads its own major up to its own minor.
    Default,
    /// The Usage ledger's family (ADRs 0030 and 0098): the major never
    /// changes and newer minors only add, so a reader reads every minor of
    /// its major, keeping what it does not know.
    FrozenMajor,
    /// The shared stores' (ADR 0098): the Run index, the Schedule store,
    /// the Saved identity store, user configuration and the Operator policy,
    /// which engines of several minors open at once, open in every minor
    /// of their format's major and record the lowest minor of their Product
    /// major that can use them, which an engine's own minor must meet.
    SharedStore,
    /// A handshake whose two sides settle on the lower of their minors, a
    /// Browser host's with its engine (ADR 0096): only another major is
    /// refused.
    LowerMinor,
    /// The Clean-up rules' manifest (ADRs 0030 and 0047): a reader reads a
    /// version of its own major up to its own minor, as the default rule
    /// does, but a version it cannot read is skipped, never refused, for the
    /// newest one it can read.
    NewestReadable,
}

impl Rule {
    /// Every value, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::FrozenMajor,
        Self::SharedStore,
        Self::LowerMinor,
        Self::NewestReadable,
    ];

    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// What a reader at `reader` makes of a file, or a peer's `hello`, at
    /// `found`. For a shared store it is its format's part alone, before
    /// the lowest minor it records ([`Rule::compatibility_of_store`]).
    ///
    /// During `0.y` previews each `0.y` minor counts as a major (ADR 0030),
    /// as Cargo reads `0.y` versions, except in the Usage ledger family,
    /// whose major never changes.
    pub fn compatibility(self, reader: ProtocolVersion, found: ProtocolVersion) -> Compatibility {
        let (reader_line, reader_minor) = self.line(reader);
        let (found_line, found_minor) = self.line(found);
        match found_line.cmp(&reader_line) {
            Ordering::Less => Compatibility::OlderMajor,
            Ordering::Greater => Compatibility::NewerMajor,
            Ordering::Equal => match self {
                Self::Default | Self::NewestReadable if found_minor > reader_minor => {
                    Compatibility::NewerMinor
                }
                Self::Default
                | Self::FrozenMajor
                | Self::SharedStore
                | Self::LowerMinor
                | Self::NewestReadable => Compatibility::Compatible,
            },
        }
    }

    /// What an engine whose Product version is `engine` (its major and
    /// minor) makes of a shared store (ADR 0098) of its format at `found`,
    /// read at `reader`, which records `lowest_minor`, the lowest minor of
    /// the engine's Product major that can use it: the format's part, then
    /// the engine's own minor against that lowest one. A `0.y` engine's line
    /// has the minor 0 alone, since its minor counts as a major.
    pub fn compatibility_of_store(
        self,
        reader: ProtocolVersion,
        found: ProtocolVersion,
        engine: ProtocolVersion,
        lowest_minor: u16,
    ) -> Compatibility {
        match self.compatibility(reader, found) {
            Compatibility::Compatible if lowest_minor > line_minor(engine) => {
                Compatibility::NewerMinor
            }
            other => other,
        }
    }

    /// A version's line, the major its rule compares, and its minor in it.
    fn line(self, version: ProtocolVersion) -> ((u16, u16), u16) {
        match self {
            Self::FrozenMajor => ((version.major, 0), version.minor),
            Self::Default | Self::SharedStore | Self::LowerMinor | Self::NewestReadable => {
                preview_line(version)
            }
        }
    }
}

/// A version's line under ADR 0030's `0.y` rule, and its minor in it: a
/// `0.y` version is a line of its own, whose only minor is 0.
fn preview_line(version: ProtocolVersion) -> ((u16, u16), u16) {
    if version.major == 0 {
        ((0, version.minor), 0)
    } else {
        ((version.major, 0), version.minor)
    }
}

/// A Product version's (its major and minor) minor in its line: its minor,
/// or 0 during `0.y` previews, whose minor counts as a major (ADR 0030).
pub fn line_minor(version: ProtocolVersion) -> u16 {
    preview_line(version).1
}

/// A compatibility function's verdict on one pair of versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// Who opens a protocol with `hello`, which decides the handshake's checks
/// (ADRs 0066, 0095 and 0096).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HelloFrom {
    /// A Client, which names the Product it needs: the engine checks it
    /// before the version, under the format's rule.
    Client,
    /// A Browser host, which needs no Product and settles on the lower
    /// minor with its engine ([`Rule::LowerMinor`]).
    BrowserHost,
    /// A Group member, refused by a group of the other Product.
    GroupMember,
}

impl HelloFrom {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Client, Self::BrowserHost, Self::GroupMember];

    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
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
    /// Who opens it with `hello`, for a protocol; none for a file.
    pub hello_from: &'static [HelloFrom],
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
    hello_from: &[HelloFrom::Client, HelloFrom::BrowserHost],
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
    hello_from: &[],
    schema: schema_of::<BrowserCacheEntry>,
};

/// The Clean-up rules' manifest (ADRs 0047 and 0066), one `manifest.json`
/// per version of the Clean-up rules beside its Filter lists: an engine
/// reads its own major up to its own minor, and skips any other version for
/// the newest one it can read.
pub const CLEAN_UP_RULES: Format = Format {
    name: "clean-up-rules",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::NewestReadable,
    hello_from: &[],
    schema: schema_of::<CleanUpRulesManifest>,
};

/// A Run's own SQLite file (ADRs 0002 and 0031): its metadata row, its
/// tags and the tables each Product's composition adds, such as a crawl's
/// work items and Records.
pub const RUN_STATE: Format = Format {
    name: "run-state",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::Default,
    hello_from: &[],
    schema: schema_of::<RunStateRow>,
};

/// The per-user, per-major Run index (ADR 0031): one row per Run.
pub const RUN_INDEX: Format = Format {
    name: "run-index",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: schema_of::<RunIndexRow>,
};

/// The per-major Operator policy of Goodfetch home (ADRs 0028 and 0098):
/// what it sets, locks and caps, nested by key, read by both Products.
pub const OPERATOR_POLICY: Format = Format {
    name: "operator-policy",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Toml,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: operator_policy_schema,
};

/// The per-major user configuration of Goodfetch home (ADRs 0028 and
/// 0098), whose shape a project configuration shares: the settings it
/// sets, nested by key, read by both Products.
pub const USER_CONFIGURATION: Format = Format {
    name: "user-configuration",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Toml,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: configuration_schema,
};

/// The per-user list of installed executables in Goodfetch home (ADR
/// 0098), outside the per-major folders: which executable runs each major's
/// Daemon, and which Pinned artifacts are still needed. Engines of every
/// major and of both Products read it, so it is of the Usage ledger's
/// family.
pub const INSTALLED_EXECUTABLES: Format = Format {
    name: "installed-executables",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::FrozenMajor,
    hello_from: &[],
    schema: schema_of::<InstalledExecutables>,
};

/// The per-user Usage ledger in Goodfetch home (ADRs 0041 and 0098), outside
/// the per-major folders: the Limits across runs, the Slices runs take
/// against them and the Usage charged. Engines of every major and of both
/// Products open it, so its major never changes and newer minors only add
/// rows, tables and columns; its schema is one Limit's.
pub const USAGE_LEDGER: Format = Format {
    name: "usage-ledger",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::FrozenMajor,
    hello_from: &[],
    schema: schema_of::<Limit>,
};

/// The per-user Politeness file in Goodfetch home (ADRs 0053 and 0098),
/// outside the per-major folders: the Permits every engine of the user, of
/// either Product, takes and gives back, its wake endpoints' waiters, and
/// the per-key state a Product's composition adds, such as Foxhound's domain
/// limits. Engines of every major read it, so it is of the Usage ledger's
/// family.
pub const POLITENESS_FILE: Format = Format {
    name: "politeness-file",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::FrozenMajor,
    hello_from: &[],
    schema: schema_of::<PermitRow>,
};

/// The per-user Audit log in Goodfetch home (ADRs 0016 and 0098), outside
/// the per-major folders: its append-only, hash-chained entries, and beside
/// it the head file that names the last one. Engines of every major and of
/// both Products append to the one chain, so it is of the Usage ledger's
/// family; its schema is one entry's.
pub const AUDIT_LOG: Format = Format {
    name: "audit-log",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::FrozenMajor,
    hello_from: &[],
    schema: schema_of::<AuditEntry>,
};

/// The per-user known-certificates store in Goodfetch home (ADR 0042),
/// beside the Usage ledger, outside the per-major folders: the pins of the
/// certificates each person's trust-on-first-use exceptions accepted, one
/// partition per person on a shared Daemon. Engines of every major read
/// it, so it is of the Usage ledger's family; its schema is one pin's.
pub const KNOWN_CERTIFICATES: Format = Format {
    name: "known-certificates",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::FrozenMajor,
    hello_from: &[],
    schema: schema_of::<CertificatePin>,
};

/// A Dataset's Export file (ADRs 0066, 0103 and 0119), the profile every
/// format of it keeps: UTF-8 with no BOM, each Record's value as
/// committed, in number order; `json` one array of the values, `jsonl` one
/// value per line. A consumer reads it, never an engine, so it keeps the
/// default version rule; its schema is a `json` Export's document, an
/// array of values.
pub const EXPORT: Format = Format {
    name: "export",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::Default,
    hello_from: &[],
    schema: schema_of::<Vec<Value>>,
};

/// A session export bundle's document (ADRs 0015 and 0056), which the
/// bundle encrypts with `age`: a Session's cookies and storage. An import
/// reads its own major up to its own minor. Its corpus holds the document
/// alone, since each encryption of it differs.
pub const SESSION_EXPORT: Format = Format {
    name: "session-export",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::Default,
    hello_from: &[],
    schema: schema_of::<SessionExport>,
};

/// Every registered format.
pub const FORMATS: &[&Format] = &[
    &PROTOCOL,
    &BROWSER_CACHE,
    &CLEAN_UP_RULES,
    &RUN_INDEX,
    &RUN_STATE,
    &INSTALLED_EXECUTABLES,
    &USAGE_LEDGER,
    &POLITENESS_FILE,
    &AUDIT_LOG,
    &KNOWN_CERTIFICATES,
    &OPERATOR_POLICY,
    &USER_CONFIGURATION,
    &EXPORT,
    &SESSION_EXPORT,
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
    properties.insert(
        "lowest_minor".to_owned(),
        json!({
            "type": "integer",
            "minimum": 0,
            "maximum": u16::MAX,
            "description": "The lowest minor of its Product major that can use it (ADR 0098); 0 when absent. A project's configuration ignores it.",
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
    let mut schema = settings_file_schema(
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
    );
    let managed = json!({
        "allOf": [generator.subschema_for::<crate::usage::Managed>()],
        "description": "Marks the policy managed, deployed by MDM or Ansible (ADR 0031), with the install Limits each engine of its major writes to the Usage ledger when it starts (ADR 0041).",
        "arrived_in": "0.1",
    });
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("managed".to_owned(), managed);
    }
    schema
}

fn any_key(_: &Setting) -> bool {
    true
}

/// A key a cap bounds: a count or a duration.
fn bounded_key(setting: &Setting) -> bool {
    matches!(
        setting.kind,
        ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::Duration
    )
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
