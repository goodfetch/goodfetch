//! The committed JSON Schema bundle and catalog, read as files: each
//! operation's params and result schemas and each Error code, and how an
//! answer is judged against them (ADR 0038).
#![expect(
    clippy::disallowed_methods,
    reason = "the schema is read inside a test, outside any async worker (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jsonschema::Validator;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The bundle `xtask codegen` commits.
const COMMITTED_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/bundle.json"
);

/// The catalog file `xtask codegen` commits beside it.
const COMMITTED_CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/catalog.json"
);

/// JSON-RPC's predefined codes that refuse a call: a parse error, an
/// invalid request, a method not found and invalid params.
const PROTOCOL_REFUSALS: [i64; 4] = [-32700, -32600, -32601, -32602];

/// JSON-RPC's predefined code of an internal error.
const PROTOCOL_INTERNAL: i64 = -32603;

/// The Error group of a bug in Goodfetch (ADR 0079).
const INTERNAL_GROUP: &str = "internal";

/// The protocol's contract as the committed bundle and catalog give it:
/// what the generated calls and the checks of their answers are made from.
#[derive(Clone, Debug)]
pub struct ProtocolSchema {
    pub(super) contract: Arc<Contract>,
}

impl ProtocolSchema {
    /// The bundle and catalog `xtask codegen` committed in
    /// `goodfetch-protocol`'s `generated/` folder.
    ///
    /// # Errors
    ///
    /// When either file cannot be read as the shape `xtask codegen` writes.
    pub fn committed() -> Result<Self, SchemaUnreadable> {
        Self::read(Path::new(COMMITTED_BUNDLE), Path::new(COMMITTED_CATALOG))
    }

    /// The JSON Schema bundle at `bundle` and the catalog file at
    /// `catalog`, whose references point into the bundle's `$defs`.
    ///
    /// # Errors
    ///
    /// When either file cannot be read as the shape `xtask codegen` writes.
    pub fn read(bundle: &Path, catalog: &Path) -> Result<Self, SchemaUnreadable> {
        let bundle_file: BundleFile = read_json(bundle)?;
        let catalog_file: CatalogFile = read_json(catalog)?;
        let defs = bundle_file.defs;
        let unexpected = |path: &Path, message: String| SchemaUnreadable::Unexpected {
            path: path.to_owned(),
            message,
        };
        let validator = |path: &Path, schema: &Value| {
            validator(&defs, schema).map_err(|message| unexpected(path, message))
        };
        let error_response = validator(bundle, &json!({ "$ref": "#/$defs/ErrorResponse" }))?;
        let operations = catalog_file
            .operations
            .into_iter()
            .map(|entry| {
                Ok(Operation {
                    params_validator: validator(catalog, &entry.params)?,
                    result_validator: validator(catalog, &entry.result)?,
                    name: entry.name,
                    params: entry.params,
                })
            })
            .collect::<Result<Vec<_>, SchemaUnreadable>>()?;
        let error_codes = catalog_file
            .error_codes
            .into_iter()
            .map(|entry| {
                let code = ErrorCode {
                    class: entry.json_rpc_class,
                    retryable: entry.retryable,
                    internal: entry.group == INTERNAL_GROUP,
                };
                (entry.name, code)
            })
            .collect();
        Ok(Self {
            contract: Arc::new(Contract {
                defs,
                operations,
                error_codes,
                error_response,
            }),
        })
    }
}

/// Why the bundle or the catalog cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaUnreadable {
    /// The file cannot be read.
    #[error("{} cannot be read: {kind}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// Why.
        kind: io::ErrorKind,
    },
    /// The file is not the JSON `xtask codegen` writes.
    #[error("{} is not the bundle or catalog `xtask codegen` writes: {message}", path.display())]
    Unexpected {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
}

/// The bundle's part a call is made from: its definitions.
#[derive(Deserialize)]
struct BundleFile {
    #[serde(rename = "$defs")]
    defs: Map<String, Value>,
}

/// The catalog's parts a call and its answer are judged by.
#[derive(Deserialize)]
struct CatalogFile {
    operations: Vec<OperationEntry>,
    error_codes: Vec<ErrorCodeEntry>,
}

#[derive(Deserialize)]
struct OperationEntry {
    name: String,
    params: Value,
    result: Value,
}

#[derive(Deserialize)]
struct ErrorCodeEntry {
    name: String,
    group: String,
    json_rpc_class: i64,
    retryable: bool,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, SchemaUnreadable> {
    let bytes = std::fs::read(path).map_err(|error| SchemaUnreadable::Unreadable {
        path: path.to_owned(),
        kind: error.kind(),
    })?;
    serde_json::from_slice(&bytes).map_err(|error| SchemaUnreadable::Unexpected {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

/// A validator of `schema`, whose references point into `defs`.
fn validator(defs: &Map<String, Value>, schema: &Value) -> Result<Validator, String> {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": defs,
        "allOf": [schema],
    });
    jsonschema::validator_for(&schema).map_err(|error| error.to_string())
}

/// What the generated calls and the checks of their answers are made from.
#[derive(Debug)]
pub(super) struct Contract {
    /// The bundle's definitions, which every schema's references name.
    pub(super) defs: Map<String, Value>,
    /// Every operation of the catalog, in its order.
    pub(super) operations: Vec<Operation>,
    error_codes: BTreeMap<String, ErrorCode>,
    error_response: Validator,
}

/// One operation of the catalog.
#[derive(Debug)]
pub(super) struct Operation {
    /// Its name, the call's method.
    pub(super) name: String,
    /// Its params' schema, a reference into the bundle.
    pub(super) params: Value,
    params_validator: Validator,
    result_validator: Validator,
}

impl Operation {
    /// Whether `params` meet its params' schema.
    pub(super) fn takes(&self, params: &Value) -> bool {
        self.params_validator.is_valid(params)
    }
}

/// An Error code of the catalog.
#[derive(Debug)]
struct ErrorCode {
    class: i64,
    retryable: bool,
    internal: bool,
}

/// How an engine answered a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// A result, which meets the operation's result schema or does not.
    Result {
        /// Whether it meets it.
        meets_schema: bool,
    },
    /// A typed error that refuses the call.
    Refused,
    /// A typed error that reports a bug in Goodfetch.
    Internal,
    /// No reply of the protocol's to this call: no typed error, a reply to
    /// another call or a result in a broken envelope.
    Untyped,
}

impl Contract {
    /// The operation named `method`.
    pub(super) fn operation(&self, method: &str) -> Option<&Operation> {
        self.operations
            .iter()
            .find(|operation| operation.name == method)
    }

    /// How `reply` answers the call of `method` whose id is `id`.
    pub(super) fn answer(&self, id: u64, method: &str, reply: &Value) -> Answer {
        if reply.get("id") != Some(&Value::from(id)) {
            return Answer::Untyped;
        }
        if let Some(result) = reply.get("result") {
            let envelope = reply.as_object().is_some_and(|object| {
                object.len() == 3 && object.get("jsonrpc") == Some(&Value::from("2.0"))
            });
            if !envelope {
                return Answer::Untyped;
            }
            let meets_schema = self
                .operation(method)
                .is_some_and(|operation| operation.result_validator.is_valid(result));
            return Answer::Result { meets_schema };
        }
        if !self.error_response.is_valid(reply) {
            return Answer::Untyped;
        }
        let error = &reply["error"];
        let class = error["code"].as_i64();
        let retryable = error["data"]["retryable"].as_bool();
        let code = error["data"]["code"].as_str().unwrap_or_default();
        match self.error_codes.get(code) {
            Some(known) if class != Some(known.class) || retryable != Some(known.retryable) => {
                Answer::Untyped
            }
            Some(known) if known.internal => Answer::Internal,
            Some(_) => Answer::Refused,
            None => match class {
                Some(PROTOCOL_INTERNAL) => Answer::Internal,
                Some(class) if PROTOCOL_REFUSALS.contains(&class) => Answer::Refused,
                _ => Answer::Untyped,
            },
        }
    }
}
