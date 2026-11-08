//! The catalog file's sections (ADR 0038): each entry of every module's
//! [`Entries`] as the catalog lists it, marked with its Product (ADR 0095),
//! its types referring to the schema bundle's. The emitter fills one
//! [`Sections`] with every module's entries and writes the catalog from it.

use schemars::SchemaGenerator;
use serde_json::{Value, json};

use super::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, HandlerRequest, HookStage,
    Metric, Operation, PROTOCOL_VERSION, PluginKind, Product, Setting, SharedTypeValue,
};

/// The catalog's sections, filled one module's [`Entries`] at a time.
#[derive(Default)]
pub struct Sections {
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
    /// Adds every entry of `entries`, marked with the Product of the module
    /// that defines it, generating its types' schemas with `generator`.
    pub fn add(&mut self, entries: &Entries, generator: &mut SchemaGenerator) {
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
    pub fn request_params(&self) -> impl Iterator<Item = &Value> {
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

    /// The catalog file, with ADR 0069's exit-code table's `exit_codes`
    /// rows.
    pub fn into_catalog(self, exit_codes: Vec<Value>) -> Value {
        json!({
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "operations": sorted(self.operations),
            "events": sorted(self.events),
            "handler_requests": sorted(self.handler_requests),
            "error_groups": ErrorGroup::ALL.iter().map(|group| error_group_entry(*group)).collect::<Vec<_>>(),
            "error_codes": sorted(self.error_codes),
            "exit_codes": exit_codes,
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
