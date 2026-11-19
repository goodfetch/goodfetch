//! What one SDK package generates (ADR 0095): the typed entries of its
//! Product's own projection, the types they define and the types they take
//! from Collie's client package, read once from the projections' JSON so
//! each language's generator works on parsed values.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::jsonrpc::META;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::projection;

/// The generators' input file name for `product`'s layer.
pub(super) fn input_name(product: Product) -> &'static str {
    match product {
        Product::Collie => "collie.json",
        Product::Foxhound => "foxhound.json",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stability {
    Stable,
    Experimental,
}

/// The shape an operation takes on every surface (ADR 0099), the catalog's
/// `OperationKind`, from which each generator applies its one form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    RoundTrip,
    Query,
    Read,
    Opener,
    Sequence,
    Waiter,
    Event,
}

#[derive(Debug, Deserialize)]
pub(super) struct Deprecation {
    pub(super) since: String,
    pub(super) note: String,
}

/// A reference to a type of the bundle, `{"$ref": "#/$defs/<name>"}`.
#[derive(Debug, Deserialize)]
#[serde(try_from = "Reference")]
pub(super) struct TypeName(pub(super) String);

#[derive(Deserialize)]
struct Reference {
    #[serde(rename = "$ref")]
    reference: String,
}

impl TryFrom<Reference> for TypeName {
    type Error = String;

    fn try_from(value: Reference) -> Result<Self, Self::Error> {
        value
            .reference
            .strip_prefix("#/$defs/")
            .map(|name| Self(name.to_owned()))
            .ok_or_else(|| format!("`{}` is no type of the bundle", value.reference))
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Operation {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) kind: Kind,
    pub(super) params: TypeName,
    pub(super) result: TypeName,
    pub(super) cancellable: bool,
    pub(super) stability: Stability,
    pub(super) deprecated: Option<Deprecation>,
    /// Which surfaces offer it; absent from a catalog that marks none.
    #[serde(default)]
    reach: Option<String>,
}

impl Operation {
    /// Whether an SDK offers it: not a person's answer, which only a
    /// person's surface, such as the CLI's interactive prompt, gives (ADR
    /// 0041).
    fn offered(&self) -> bool {
        self.reach.as_deref() != Some("person_only")
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Event {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) payload: TypeName,
    pub(super) stability: Stability,
    pub(super) deprecated: Option<Deprecation>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorGroup {
    pub(super) name: String,
    pub(super) description: String,
    /// The JSON-RPC error code the wire carries for the group's codes.
    pub(super) json_rpc_class: i64,
    /// The CLI's exit code for the group's codes without a row of their own:
    /// the group's row of ADR 0069's table, which the projection lists as
    /// `exit_codes`; `None` when it lists none, as the codegen fixture's.
    #[serde(skip)]
    pub(super) exit_code: Option<u8>,
}

/// A failure of the protocol itself: the JSON-RPC code JSON-RPC 2.0
/// predefines that it takes, and the group an SDK raises it in (ADR 0079).
#[derive(Debug, Deserialize)]
struct ProtocolFailure {
    json_rpc_code: i64,
    group: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorCode {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) group: String,
    pub(super) retryable: bool,
    /// The code's own row of ADR 0069's exit-code table, when it has one.
    #[serde(default)]
    pub(super) exit_code: Option<u8>,
}

/// A request the engine sends a program's handler, such as
/// `crawl_request.handle`: only the types it takes, which the layer defines.
#[derive(Debug, Deserialize)]
struct HandlerRequest {
    product: Product,
    params: TypeName,
    result: TypeName,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginMethod {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) params: TypeName,
    pub(super) result: TypeName,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginKind {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) methods: Vec<PluginMethod>,
    pub(super) stability: Stability,
}

#[derive(Debug, Deserialize)]
struct SharedTypeValue {
    name: String,
    product: Product,
    #[serde(rename = "type")]
    shared_type: String,
    description: String,
}

/// A type both Products share (ADR 0095), as one package spells it: the
/// values its Product adds, which an SDK keeps open to a value it does not
/// know (ADR 0079).
#[derive(Debug)]
pub(super) struct SharedType {
    /// Its `snake_case` name, such as `block_cause`.
    pub(super) name: String,
    /// Whether Collie's client package spells it too, so that this layer's
    /// values add to Collie's.
    pub(super) extends_collie: bool,
    /// The values this layer adds, by name, each with its description.
    pub(super) values: Vec<(String, String)>,
}

/// The shared types of `values`, each with the values `own` keeps, by name.
fn shared_types(
    values: &[SharedTypeValue],
    own: impl Fn(Product) -> bool,
    extends_collie: impl Fn(&str) -> bool,
) -> Vec<SharedType> {
    let mut types: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
    for value in values.iter().filter(|value| own(value.product)) {
        types
            .entry(value.shared_type.as_str())
            .or_default()
            .push((value.name.clone(), value.description.clone()));
    }
    types
        .into_iter()
        .map(|(name, mut values)| {
            values.sort();
            SharedType {
                name: name.to_owned(),
                extends_collie: extends_collie(name),
                values,
            }
        })
        .collect()
}

/// The sections of a projection that the SDK layers read; any other section
/// is no SDK's (metrics, for instance).
#[derive(Deserialize)]
struct Projection {
    protocol_version: String,
    operations: Vec<Operation>,
    events: Vec<Event>,
    error_groups: Vec<ErrorGroup>,
    error_codes: Vec<ErrorCode>,
    /// Absent from the codegen fixture's.
    #[serde(default)]
    protocol_failures: Vec<ProtocolFailure>,
    /// ADR 0069's exit-code table; absent from the codegen fixture's.
    #[serde(default)]
    exit_codes: Vec<ExitCodeRow>,
    plugin_kinds: Vec<PluginKind>,
    /// Absent from a catalog that defines no handler request.
    #[serde(default)]
    handler_requests: Vec<HandlerRequest>,
    /// Absent from a catalog that defines no shared type.
    #[serde(default)]
    shared_type_values: Vec<SharedTypeValue>,
    /// Absent from a catalog that defines no setting.
    #[serde(default)]
    settings: Vec<Setting>,
    #[serde(rename = "$defs")]
    definitions: Map<String, Value>,
}

/// A setting of the projection's settings section: the SDK layers read its
/// default, and type each scope's `settings` option with those a program's
/// code sets (ADR 0083).
#[derive(Clone, Deserialize)]
pub(super) struct Setting {
    /// Its key, a dotted `snake_case` path.
    pub(super) name: String,
    /// What it shapes.
    #[serde(default)]
    pub(super) description: String,
    /// The JSON Schema of its values.
    #[serde(default)]
    pub(super) value: Value,
    /// Its value when no layer sets it, `null` for a setting with none.
    #[serde(default)]
    pub(super) default: Value,
    /// Whether `null` clears it.
    #[serde(default)]
    pub(super) clearable: bool,
    /// The scopes whose own `settings` set it, outermost first: none for a
    /// setting only configuration or an Operator policy sets.
    #[serde(default)]
    pub(super) scopes: Vec<String>,
}

/// Each setting's default, by name.
fn setting_defaults(settings: &[Setting]) -> BTreeMap<String, Value> {
    settings
        .iter()
        .map(|setting| (setting.name.clone(), setting.default.clone()))
        .collect()
}

/// A row of ADR 0069's exit-code table: an outcome's, a group's or an Error
/// code's exit code, by its name.
#[derive(Deserialize)]
struct ExitCodeRow {
    name: String,
    code: u8,
}

/// The group of each JSON-RPC code `failures` take: one group per code,
/// since an SDK reads no more than the code when a reply carries no Error
/// code.
fn protocol_failure_groups(failures: &[ProtocolFailure]) -> Result<BTreeMap<i64, String>> {
    let mut groups = BTreeMap::new();
    for failure in failures {
        let group = groups
            .entry(failure.json_rpc_code)
            .or_insert_with(|| failure.group.clone());
        anyhow::ensure!(
            *group == failure.group,
            "the protocol failures of code {} fall in two groups, `{group}` and `{}`",
            failure.json_rpc_code,
            failure.group
        );
    }
    Ok(groups)
}

/// One SDK package's layer.
pub(super) struct Layer {
    pub(super) product: Product,
    /// The protocol version `major.minor` the catalog speaks.
    pub(super) protocol_version: String,
    /// The Product version the workspace carries, which the client package's
    /// `hello` names.
    pub(super) product_version: String,
    pub(super) operations: Vec<Operation>,
    pub(super) events: Vec<Event>,
    /// The seven groups, generated once, in Collie's client package.
    pub(super) error_groups: Vec<ErrorGroup>,
    /// The group of each JSON-RPC code a protocol failure takes, generated
    /// once, in Collie's client package, beside the groups.
    pub(super) protocol_failure_groups: BTreeMap<i64, String>,
    pub(super) error_codes: Vec<ErrorCode>,
    pub(super) plugin_kinds: Vec<PluginKind>,
    /// The types both Products share that this layer adds values to.
    pub(super) shared_types: Vec<SharedType>,
    /// The types this layer defines.
    pub(super) definitions: Map<String, Value>,
    /// The types this layer's entries take from Collie's client package.
    pub(super) collie_types: BTreeSet<String>,
    /// Each setting's default, by name, which the Python handles document
    /// a call's own `timeout` with (ADR 0080).
    pub(super) setting_defaults: BTreeMap<String, Value>,
    /// The settings this layer's Product honours, which its scopes'
    /// `settings` options are typed with.
    pub(super) settings: Vec<Setting>,
}

impl Layer {
    /// The layer of `product`'s package: Collie's entries for Collie's client
    /// package, with the types of the bundle's `connection` section, which
    /// every engine speaks and the client package's runtime sends; Foxhound's
    /// own for Foxhound's, which builds on it.
    pub(super) fn new(
        bundle: &Value,
        collie: &Value,
        foxhound: &Value,
        product: Product,
        product_version: &str,
    ) -> Result<Self> {
        let mut collie: Projection =
            Projection::deserialize(collie).context("reading Collie's projection")?;
        without_meta(&mut collie.definitions);
        match product {
            Product::Collie => {
                let mut definitions = collie.definitions;
                let bundle_definitions = bundle
                    .get("$defs")
                    .and_then(Value::as_object)
                    .context("the bundle has no `$defs`")?;
                let connection = bundle.get("connection").cloned().unwrap_or(Value::Null);
                for name in projection::reached(&connection, bundle_definitions)? {
                    if let Some(schema) = bundle_definitions.get(&name) {
                        definitions.insert(name, schema.clone());
                    }
                }
                without_meta(&mut definitions);
                definitions.sort_keys();
                let shared_types = shared_types(&collie.shared_type_values, |_| true, |_| false);
                for group in &mut collie.error_groups {
                    group.exit_code = collie
                        .exit_codes
                        .iter()
                        .find(|row| row.name == group.name)
                        .map(|row| row.code);
                }
                Ok(Self {
                    product,
                    protocol_version: collie.protocol_version,
                    product_version: product_version.to_owned(),
                    operations: collie
                        .operations
                        .into_iter()
                        .filter(Operation::offered)
                        .collect(),
                    events: collie.events,
                    error_groups: collie.error_groups,
                    protocol_failure_groups: protocol_failure_groups(&collie.protocol_failures)?,
                    error_codes: collie.error_codes,
                    plugin_kinds: collie.plugin_kinds,
                    shared_types,
                    definitions,
                    collie_types: BTreeSet::new(),
                    setting_defaults: setting_defaults(&collie.settings),
                    settings: collie.settings,
                })
            }
            Product::Foxhound => Self::foxhound(&collie, foxhound, product_version),
        }
    }

    /// Foxhound's layer: its own entries, and the types they reach that
    /// Collie's projection does not define, those it does taken from
    /// Collie's client package.
    fn foxhound(collie: &Projection, foxhound: &Value, product_version: &str) -> Result<Self> {
        let mut all: Projection =
            Projection::deserialize(foxhound).context("reading Foxhound's projection")?;
        without_meta(&mut all.definitions);
        let own = |entry_product: Product| entry_product == Product::Foxhound;
        let operations: Vec<Operation> = all
            .operations
            .into_iter()
            .filter(|entry| own(entry.product) && entry.offered())
            .collect();
        let events: Vec<Event> = all
            .events
            .into_iter()
            .filter(|entry| own(entry.product))
            .collect();
        let error_codes: Vec<ErrorCode> = all
            .error_codes
            .into_iter()
            .filter(|entry| own(entry.product))
            .collect();
        let plugin_kinds: Vec<PluginKind> = all
            .plugin_kinds
            .into_iter()
            .filter(|entry| own(entry.product))
            .collect();
        let shared_types = shared_types(&all.shared_type_values, own, |name| {
            collie
                .shared_type_values
                .iter()
                .any(|value| value.shared_type == name)
        });
        let handler_requests: Vec<HandlerRequest> = all
            .handler_requests
            .into_iter()
            .filter(|entry| own(entry.product))
            .collect();
        let roots = roots(&operations, &events, &plugin_kinds, &handler_requests);
        let reached = projection::reached(&roots, &all.definitions)?;
        let (collie_types, own_types): (BTreeSet<String>, BTreeSet<String>) = reached
            .into_iter()
            .partition(|name| collie.definitions.contains_key(name));
        Ok(Self {
            product: Product::Foxhound,
            protocol_version: all.protocol_version,
            product_version: product_version.to_owned(),
            operations,
            events,
            error_groups: Vec::new(),
            protocol_failure_groups: BTreeMap::new(),
            error_codes,
            plugin_kinds,
            shared_types,
            definitions: all
                .definitions
                .into_iter()
                .filter(|(name, _)| own_types.contains(name))
                .collect(),
            collie_types,
            setting_defaults: setting_defaults(&all.settings),
            settings: all.settings,
        })
    }

    /// Whether `name` is a type of Collie's client package rather than this
    /// layer's.
    pub(super) fn is_collie_type(&self, name: &str) -> bool {
        self.collie_types.contains(name)
    }

    /// The `$defs` a generator turns into types: this layer's own, with each
    /// reference to one of Collie's types replaced by `replacement(name)`,
    /// the generator's way of naming a type it imports. Also returns the
    /// names it replaced.
    pub(super) fn generator_input(
        &self,
        title: &str,
        replacement: impl Fn(&str) -> (&'static str, String),
    ) -> (Value, BTreeSet<String>) {
        let mut definitions = Value::Object(self.definitions.clone());
        let mut replaced = BTreeSet::new();
        replace_collie_references(
            &mut definitions,
            &self.collie_types,
            &replacement,
            &mut replaced,
        );
        let input = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": title,
            "description": "The root of the layer's types, which holds no value of its own.",
            "$defs": definitions,
        });
        (input, replaced)
    }
}

/// `definitions` without `_meta`, the Trace context the bundle adds to
/// every request's params: an envelope field each SDK's runtime fills in,
/// to which no generator gives a member (ADRs 0099 and 0132).
fn without_meta(definitions: &mut Map<String, Value>) {
    for definition in definitions.values_mut() {
        if let Some(properties) = definition
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.remove(META);
        }
    }
}

/// A value whose `$ref`s are every type the entries name.
fn roots(
    operations: &[Operation],
    events: &[Event],
    plugin_kinds: &[PluginKind],
    handler_requests: &[HandlerRequest],
) -> Value {
    let reference = |name: &TypeName| serde_json::json!({ "$ref": format!("#/$defs/{}", name.0) });
    let operations = operations
        .iter()
        .flat_map(|operation| [reference(&operation.params), reference(&operation.result)]);
    let events = events.iter().map(|event| reference(&event.payload));
    let methods = plugin_kinds
        .iter()
        .flat_map(|kind| &kind.methods)
        .flat_map(|method| [reference(&method.params), reference(&method.result)]);
    let handler_requests = handler_requests
        .iter()
        .flat_map(|request| [reference(&request.params), reference(&request.result)]);
    Value::Array(
        operations
            .chain(events)
            .chain(methods)
            .chain(handler_requests)
            .collect(),
    )
}

fn replace_collie_references(
    value: &mut Value,
    collie_types: &BTreeSet<String>,
    replacement: &impl Fn(&str) -> (&'static str, String),
    replaced: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            let collie_type = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .filter(|name| collie_types.contains(*name))
                .map(str::to_owned);
            if let Some(name) = collie_type {
                object.remove("$ref");
                let (key, value) = replacement(&name);
                object.insert(key.to_owned(), Value::String(value));
                replaced.insert(name);
            }
            for child in object.values_mut() {
                replace_collie_references(child, collie_types, replacement, replaced);
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_collie_references(item, collie_types, replacement, replaced);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::catalog::Product;
    use serde_json::{Value, json};

    use super::Layer;

    /// An operation of a projection, offered on the surfaces `reach` names.
    fn operation(name: &str, product: &str, reach: &str) -> Value {
        json!({
            "name": name,
            "product": product,
            "description": "An operation.",
            "kind": "round_trip",
            "reach": reach,
            "params": { "$ref": "#/$defs/Empty" },
            "result": { "$ref": "#/$defs/Empty" },
            "cancellable": false,
            "stability": "experimental",
            "deprecated": null,
        })
    }

    fn projection(operations: &[Value]) -> Value {
        json!({
            "protocol_version": "0.1",
            "operations": operations,
            "events": [],
            "error_groups": [],
            "error_codes": [],
            "plugin_kinds": [],
            "$defs": { "Empty": { "type": "object" } },
        })
    }

    /// Covers: story:3-75
    #[test]
    fn an_sdk_layer_offers_no_persons_answer() {
        let collie = projection(&[
            operation("widget.spin", "collie", "all_surfaces"),
            operation("widget.lend", "collie", "sdk_only"),
            operation("widget.raise", "collie", "person_only"),
        ]);
        let foxhound = projection(&[
            operation("crawl.spin", "foxhound", "all_surfaces"),
            operation("crawl.accept", "foxhound", "person_only"),
        ]);
        let bundle = json!({ "$defs": {} });

        let offered = |product| {
            Layer::new(&bundle, &collie, &foxhound, product, "0.0.0")
                .unwrap()
                .operations
                .into_iter()
                .map(|operation| operation.name)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            (offered(Product::Collie), offered(Product::Foxhound)),
            (
                vec!["widget.spin".to_owned(), "widget.lend".to_owned()],
                vec!["crawl.spin".to_owned()]
            )
        );
    }

    /// Raising a Limit is a person's answer (ADRs 0029 and 0041): the
    /// committed catalog marks `usage.raise` person-only, and neither
    /// Product's SDK layer offers it, so no program or agent is generated a
    /// way to lift the cap that stopped it.
    ///
    /// Covers: story:12-100
    #[test]
    fn no_sdk_layer_offers_the_raising_of_a_limit() {
        let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/goodfetch-protocol/generated");
        let read = |name: &str| -> Value {
            serde_json::from_str(&std::fs::read_to_string(generated.join(name)).unwrap()).unwrap()
        };
        let (bundle, collie, foxhound) = (
            read("bundle.json"),
            read("collie.json"),
            read("foxhound.json"),
        );
        let reach = collie["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["name"] == "usage.raise")
            .map(|operation| operation["reach"].clone());
        let offers = |product| {
            Layer::new(&bundle, &collie, &foxhound, product, "0.0.0")
                .unwrap()
                .operations
                .iter()
                .any(|operation| operation.name == "usage.raise")
        };

        assert_eq!(
            (reach, offers(Product::Collie), offers(Product::Foxhound)),
            (Some(json!("person_only")), false, false)
        );
    }

    /// A Product's handler requests are the engine's to send and its
    /// program's to answer, so its layer defines the types they take, as it
    /// does its operations'.
    #[test]
    fn the_foxhound_layer_defines_the_types_its_handler_requests_take() {
        let collie = projection(&[]);
        let mut foxhound = projection(&[]);
        foxhound["handler_requests"] = json!([{
            "name": "crawl_request.handle",
            "product": "foxhound",
            "description": "A handler request.",
            "params": { "$ref": "#/$defs/HandleParams" },
            "result": { "$ref": "#/$defs/Handled" },
            "stability": "experimental",
            "deprecated": null,
        }]);
        foxhound["$defs"]["HandleParams"] = json!({ "type": "object" });
        foxhound["$defs"]["Handled"] = json!({ "type": "object" });
        let bundle = json!({ "$defs": {} });

        let layer = Layer::new(&bundle, &collie, &foxhound, Product::Foxhound, "0.0.0").unwrap();

        assert_eq!(
            layer.definitions.keys().cloned().collect::<Vec<_>>(),
            vec!["HandleParams".to_owned(), "Handled".to_owned()]
        );
    }

    /// Covers: story:3-146
    #[test]
    fn no_sdk_layer_gives_a_requests_trace_context_a_member() {
        let params = json!({
            "type": "object",
            "properties": {
                "size": { "type": "integer" },
                "_meta": { "$ref": "#/$defs/TraceContext" },
            },
        });
        let mut collie = projection(&[operation("widget.spin", "collie", "all_surfaces")]);
        collie["$defs"]["Empty"] = params.clone();
        let mut crawl = operation("crawl.spin", "foxhound", "all_surfaces");
        crawl["params"] = json!({ "$ref": "#/$defs/CrawlSpinParams" });
        let mut foxhound = projection(&[crawl]);
        foxhound["$defs"]["CrawlSpinParams"] = params;
        for projection in [&mut collie, &mut foxhound] {
            projection["$defs"]["TraceContext"] = json!({ "type": "object" });
        }
        let bundle = json!({ "$defs": {} });

        let members = |product, name: &str| {
            let layer = Layer::new(&bundle, &collie, &foxhound, product, "0.0.0").unwrap();
            let (input, _) = layer.generator_input("Layer", |name| ("import", name.to_owned()));
            input["$defs"][name]["properties"]
                .as_object()
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
        };

        assert_eq!(
            (
                members(Product::Collie, "Empty"),
                members(Product::Foxhound, "CrawlSpinParams")
            ),
            (Some(vec!["size".to_owned()]), Some(vec!["size".to_owned()]))
        );
    }
}
