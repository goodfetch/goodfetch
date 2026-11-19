//! The handles a program holds (ADR 0099), read from each operation's kind,
//! its namespace and its handle ID, for every language's generator: which
//! operations are a handle's members and in which form, which parameters
//! are positional, which values a handle builds without a call and which
//! sub-objects it holds. Nothing here is per entry: a rule the catalog
//! breaks fails `xtask codegen` with the entry named.
//!
//! - A handle's ID is the required parameter `<namespace>_id`, or the one
//!   named as the namespace whose type is the value itself (a locator).
//!   An operation of the namespace that takes no such parameter is no
//!   member: it is how a program gets the value (`run.start`).
//! - A round trip, a query and a waiter are members; a query declares no
//!   `timeout`, and a waiter's name starts with `wait_for`. A round trip
//!   named `new_<value>` opens that value, whose ID its result carries, and
//!   a handle's own `close` that takes no required parameter closes it. An
//!   event named `on` of a namespace in [`LISTENERS`], whose one required
//!   parameter is the event listened to, is the handle's listener (ADR
//!   0081): each value of that parameter names the event
//!   `<namespace>.<value>`, whose payload the listener gets. A read, a sequence and another event get their forms
//!   with their own tickets (issue 1233), as does an opener, which opens its
//!   own namespace's value.
//! - A prefix has one meaning: `new_` starts a round trip or an opener,
//!   `wait_for` a waiter and every waiter, `is_` a query, `on` an event or
//!   a sequence, and `get_by_` no operation, since it builds without I/O.
//! - Required parameters are positional in the catalog's order, but a
//!   boolean, which is named like every optional one.
//! - A handle whose ID is a value that holds another handle's ID is built
//!   by that handle, without a call, from the value's other fields.
//! - `<handle>.<name>.<member>` is a member of the handle's sub-object
//!   `<name>`, which takes the handle's ID.
//! - A result property marked `typed_by` a parameter holds the values of
//!   the fields that parameter declares (ADR 0147), so a member types it
//!   from the fields its caller passes. The parameter is a required one of
//!   the operation, and a result marks one property at most.
//! - A round trip, a query or a waiter whose result has exactly one
//!   property answers that property's value, so `page.goto` answers the
//!   document's response rather than a wrapper of it (issue 2517); an
//!   optional property answers the language's none when it is absent. The
//!   operation's own function, the wire and the result type are unchanged.

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value};

use super::layer::{Kind, Layer, Operation};

/// The namespaces whose handles the generators write so far; each other
/// one joins with the ticket that builds its value.
pub(super) const HANDLES: [&str; 4] = ["run", "session", "page", "locator"];

/// The namespaces whose `on` the generators write as a listener (ADR 0081),
/// which gets its events on its own call. `session.on` publishes on the
/// shared event stream instead, so it joins with its own ticket.
pub(super) const LISTENERS: [&str; 1] = ["page"];

/// The parameter that names a handle in its operations' params.
pub(super) struct HandleId {
    /// The parameter, such as `page_id`.
    pub(super) param: String,
    /// Its type in the bundle, such as `PageId`.
    pub(super) type_name: String,
    /// Whether the type is the handle's own value rather than an ID the
    /// engine or the program gave it.
    pub(super) value: bool,
}

/// A member's or a builder's parameters but the handle's ID, as a caller
/// passes them.
#[derive(Default)]
pub(super) struct Parameters {
    /// The required ones that are no boolean, in the catalog's order.
    pub(super) positional: Vec<String>,
    /// The required booleans, which are named.
    pub(super) named: Vec<String>,
    /// The optional ones, which are named.
    pub(super) optional: Vec<String>,
}

/// The form a member takes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Form {
    RoundTrip,
    Query,
    Waiter,
    /// A round trip that opens a value of this namespace, whose ID its
    /// result carries as `<namespace>_id`.
    Opens(String),
    /// The handle's own `close` (ADR 0078): idempotent, it answers nothing,
    /// and every other call on the value then fails with `closed`.
    Closes,
}

/// A handle's `on`, which registers a listener for one of its events (ADR
/// 0081): its call stays in flight and gets them. TypeScript writes it so
/// far, Python and Rust with issues 2932 and 2933.
pub(super) struct Listener<'a> {
    pub(super) operation: &'a Operation,
    /// The parameter that names the event listened to, such as `event`.
    pub(super) param: String,
    /// The events it listens to, in the order its parameter's type lists them.
    pub(super) events: Vec<Listened>,
}

/// An event a [`Listener`] gets.
pub(super) struct Listened {
    /// The value its call's parameter names it by, such as `page_error`.
    pub(super) wire: String,
    /// Its payload's type in the bundle, such as `WebError`.
    pub(super) payload: String,
    /// Its description in the catalog.
    pub(super) description: String,
}

impl Listened {
    /// Its name as a program passes it: Playwright's, its words joined in
    /// lowercase, such as `pageerror` (ADR 0081).
    pub(super) fn joined(&self) -> String {
        self.wire.replace('_', "")
    }
}

/// The schema keyword that marks a result property typed by a parameter.
const TYPED_BY: &str = "typed_by";

/// The schema keyword that names the Duration setting a call's own
/// `timeout` sets for that call alone (ADR 0080).
const SETTING: &str = "setting";

/// A result property whose names and types follow the fields a parameter
/// declares (ADR 0147).
pub(super) struct TypedBy {
    /// The positional parameter that declares the fields, such as `fields`.
    pub(super) param: String,
    /// The result's property that holds their values, such as `values`.
    pub(super) property: String,
}

/// The one property of a member's result, which the member answers in
/// place of the result.
pub(super) struct Unwrapped {
    /// The property, as the wire names it, such as `response`.
    pub(super) property: String,
    /// Whether the result always carries it; an optional one answers the
    /// language's none when it is absent.
    pub(super) required: bool,
}

/// An operation as a member of a handle or of one of its sub-objects.
pub(super) struct Member<'a> {
    pub(super) operation: &'a Operation,
    /// Its `snake_case` name on the handle, such as `wait_for_url`.
    pub(super) name: &'a str,
    pub(super) form: Form,
    pub(super) parameters: Parameters,
    /// The result property its declared fields type, if any.
    pub(super) typed_by: Option<TypedBy>,
    /// The Duration setting its own `timeout` sets for the call alone, such
    /// as `timeout.wait`; none when it takes no `timeout`.
    pub(super) timeout_setting: Option<String>,
    /// The one property of its result, which it answers in place of the
    /// result; none when the result has no property or several.
    pub(super) unwrapped: Option<Unwrapped>,
}

/// A value a handle builds without a call.
pub(super) struct Builder {
    /// The namespace of the value built, which names the method.
    pub(super) builds: String,
    /// The value's type in the bundle.
    pub(super) type_name: String,
    /// The value's description in the bundle.
    pub(super) description: String,
    pub(super) parameters: Parameters,
}

/// A sub-object of a handle: its name and its members.
pub(super) struct SubObject<'a> {
    pub(super) name: &'a str,
    pub(super) members: Vec<Member<'a>>,
}

/// One namespace's handle.
pub(super) struct Handle<'a> {
    pub(super) namespace: &'static str,
    pub(super) id: HandleId,
    pub(super) members: Vec<Member<'a>>,
    pub(super) builders: Vec<Builder>,
    pub(super) sub_objects: Vec<SubObject<'a>>,
    /// Its `on`, when its namespace is one of [`LISTENERS`].
    pub(super) listener: Option<Listener<'a>>,
}

impl Handle<'_> {
    /// The handle of `namespace`, named by `id`, before its members.
    fn of(namespace: &'static str, id: HandleId) -> Self {
        Self {
            namespace,
            id,
            members: Vec::new(),
            builders: Vec::new(),
            sub_objects: Vec::new(),
            listener: None,
        }
    }

    /// Whether a program closes the value: its catalog has a `close` in
    /// the closing form.
    pub(super) fn closes(&self) -> bool {
        self.members
            .iter()
            .any(|member| member.form == Form::Closes)
    }
}

fn names(schema: &Value, key: &str) -> Vec<String> {
    schema[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn referenced(property: &Value) -> Option<&str> {
    property["$ref"].as_str()?.strip_prefix("#/$defs/")
}

/// The parameter of `operation`'s params, `schema`, that names a handle of
/// `namespace`, if it takes one.
fn handle_id(namespace: &str, operation: &Operation, schema: &Value) -> Result<Option<HandleId>> {
    let required = names(schema, "required");
    let scalar = format!("{namespace}_id");
    let (param, value) = if required.contains(&scalar) {
        (scalar, false)
    } else if required.iter().any(|name| name == namespace) {
        (namespace.to_owned(), true)
    } else {
        return Ok(None);
    };
    let Some(type_name) = referenced(&schema["properties"][&param]) else {
        bail!(
            "operation `{}`: its handle ID `{param}` has a type of its own in the bundle, which the handle's class is typed by (ADR 0099)",
            operation.name
        );
    };
    Ok(Some(HandleId {
        type_name: type_name.to_owned(),
        param,
        value,
    }))
}

/// Fails when the last segment of `operation`'s name starts with a prefix
/// whose one meaning its kind does not have (ADR 0099).
fn check_prefix(operation: &Operation, name: &str) -> Result<()> {
    let starts = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'))
    };
    let kind = operation.kind;
    let broken = if starts("wait_for") != (kind == Kind::Waiter) {
        Some("`wait_for` starts the name of a waiter, and of nothing else")
    } else if starts("new") && !matches!(kind, Kind::RoundTrip | Kind::Opener) {
        Some("`new_` opens a value the caller closes, which a round trip or an opener does")
    } else if starts("is") && kind != Kind::Query {
        Some("`is_` answers yes or no and never waits, which a query does")
    } else if starts("on") && !matches!(kind, Kind::Event | Kind::Sequence) {
        Some("`on` observes side activity, which an event or a sequence does")
    } else if starts("get_by") {
        Some("`get_by_` builds a value without I/O, which no operation does")
    } else {
        None
    };
    match broken {
        Some(why) => bail!("operation `{}`: {why} (ADR 0099)", operation.name),
        None => Ok(()),
    }
}

/// The parameters of `schema` but `id`, as a caller passes them.
fn parameters(schema: &Value, id: &str) -> Parameters {
    let required = names(schema, "required");
    let mut parameters = Parameters::default();
    for name in required.iter().filter(|name| *name != id) {
        if schema["properties"][name]["type"] == "boolean" {
            parameters.named.push(name.clone());
        } else {
            parameters.positional.push(name.clone());
        }
    }
    parameters.optional = schema["properties"]
        .as_object()
        .into_iter()
        .flat_map(Map::keys)
        .filter(|name| *name != id && !required.contains(name))
        .cloned()
        .collect();
    parameters
}

/// The property of `operation`'s result marked `typed_by` one of its
/// `parameters`, if it has one.
fn typed_by(
    layer: &Layer,
    operation: &Operation,
    form: &Form,
    parameters: &Parameters,
) -> Result<Option<TypedBy>> {
    let Some(result) = layer.definitions.get(&operation.result.0) else {
        return Ok(None);
    };
    let mut marked = result["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(property, schema)| Some((property, schema.get(TYPED_BY)?)));
    let Some((property, param)) = marked.next() else {
        return Ok(None);
    };
    if !matches!(form, Form::RoundTrip | Form::Query | Form::Waiter) {
        bail!(
            "operation `{}`: its result's `{property}` is `{TYPED_BY}`, which only a round trip, a query or a waiter answers: an opened or closed value answers no fields (ADR 0147)",
            operation.name
        );
    }
    if marked.next().is_some() {
        bail!(
            "operation `{}`: its result marks more than one property `{TYPED_BY}`, where a call declares one set of fields (ADR 0147)",
            operation.name
        );
    }
    let Some(param) = param
        .as_str()
        .filter(|param| parameters.positional.iter().any(|name| name == param))
    else {
        bail!(
            "operation `{}`: its result's `{property}` is `{TYPED_BY}` {param}, which names no required parameter of it, so no call declares its fields (ADR 0147)",
            operation.name
        );
    };
    Ok(Some(TypedBy {
        param: param.to_owned(),
        property: property.clone(),
    }))
}

/// The one property of `operation`'s result, which its member in `form`
/// answers in place of the result: a round trip's, a query's or a waiter's
/// whose result has exactly one property. A property `typed_by` the call's
/// fields is refused there, since its values' type is the member's own.
fn unwrapped(
    layer: &Layer,
    operation: &Operation,
    form: &Form,
    typed_by: Option<&TypedBy>,
) -> Result<Option<Unwrapped>> {
    if !matches!(form, Form::RoundTrip | Form::Query | Form::Waiter) {
        return Ok(None);
    }
    let Some(result) = layer.definitions.get(&operation.result.0) else {
        return Ok(None);
    };
    let Some(properties) = result["properties"].as_object() else {
        return Ok(None);
    };
    let mut only = properties.keys();
    let (Some(property), None) = (only.next(), only.next()) else {
        return Ok(None);
    };
    if let Some(typed) = typed_by {
        bail!(
            "operation `{}`: its result's one property `{}` is `{TYPED_BY}`, which a member answers in place of the result, so the fields' values need a result of two properties or more (ADR 0147)",
            operation.name,
            typed.property
        );
    }
    Ok(Some(Unwrapped {
        required: names(result, "required").contains(property),
        property: property.clone(),
    }))
}

/// The Duration setting `operation`'s own `timeout` sets, which its params
/// type, `schema`, names with `setting`; none when it takes no `timeout`.
fn timeout_setting(operation: &Operation, schema: &Value) -> Result<Option<String>> {
    let Some(timeout) = schema["properties"].get("timeout") else {
        return Ok(None);
    };
    let Some(setting) = timeout.get(SETTING).and_then(Value::as_str) else {
        bail!(
            "operation `{}`: its `timeout` names no `{SETTING}`, the Duration setting it sets for the call alone, which a refusal of `0` names (ADR 0080)",
            operation.name
        );
    };
    Ok(Some(setting.to_owned()))
}

/// The form `operation` takes as the member `name`, or `None` for a kind
/// that has no form yet.
fn form(operation: &Operation, name: &str, schema: &Value) -> Result<Option<Form>> {
    Ok(match operation.kind {
        Kind::RoundTrip => Some(
            name.strip_prefix("new_")
                .map_or(Form::RoundTrip, |opened| Form::Opens(opened.to_owned())),
        ),
        Kind::Query => {
            if schema["properties"].get("timeout").is_some() {
                bail!(
                    "operation `{}`: a query never waits, so it declares no `timeout` (ADR 0099)",
                    operation.name
                );
            }
            Some(Form::Query)
        }
        Kind::Waiter => Some(Form::Waiter),
        Kind::Read | Kind::Opener | Kind::Sequence | Kind::Event => None,
    })
}

/// `operation`, the `on` of `namespace`, as its listener: it gets one
/// event for each value of its one required parameter but the handle's ID,
/// the event listened to, which names the event `<namespace>.<value>`.
fn listener<'a>(
    layer: &Layer,
    namespace: &str,
    operation: &'a Operation,
    schema: &Value,
) -> Result<Listener<'a>> {
    let id = format!("{namespace}_id");
    let required = names(schema, "required");
    let [event] = required
        .iter()
        .filter(|name| **name != id)
        .collect::<Vec<_>>()[..]
    else {
        bail!(
            "operation `{}`: a listener's `on` takes one required parameter but its handle's ID, the event listened to (ADR 0081)",
            operation.name
        );
    };
    let values = referenced(&schema["properties"][event])
        .and_then(|type_name| layer.definitions.get(type_name))
        .and_then(|definition| definition["oneOf"].as_array())
        .and_then(|variants| {
            variants
                .iter()
                .map(|variant| variant["const"].as_str())
                .collect::<Option<Vec<_>>>()
        });
    let Some(values) = values.filter(|values| !values.is_empty()) else {
        bail!(
            "operation `{}`: its `{event}` is a type of its own whose values name the events listened to (ADR 0081)",
            operation.name
        );
    };
    let events = values
        .into_iter()
        .map(|wire| {
            let name = format!("{namespace}.{wire}");
            let Some(found) = layer.events.iter().find(|held| held.name == name) else {
                bail!(
                    "operation `{}`: its `{event}` `{wire}` names the event `{name}`, which the catalog lacks (ADR 0081)",
                    operation.name
                );
            };
            Ok(Listened {
                wire: wire.to_owned(),
                payload: found.payload.0.clone(),
                description: found.description.clone(),
            })
        })
        .collect::<Result<_>>()?;
    Ok(Listener {
        operation,
        param: event.clone(),
        events,
    })
}

/// The handles of `layer`'s operations, in [`HANDLES`]' order; a namespace
/// with no member has none.
pub(super) fn handles(layer: &Layer) -> Result<Vec<Handle<'_>>> {
    let mut handles: Vec<Handle<'_>> = Vec::new();
    for namespace in HANDLES {
        let mut found: Option<Handle<'_>> = None;
        for operation in &layer.operations {
            let Some((path, name)) = operation.name.rsplit_once('.') else {
                continue;
            };
            let (owner, sub_object) = match path.split_once('.') {
                None => (path, None),
                Some((owner, sub_object)) => (owner, Some(sub_object)),
            };
            if owner != namespace {
                continue;
            }
            check_prefix(operation, name)?;
            let schema = layer
                .definitions
                .get(&operation.params.0)
                .with_context(|| format!("operation `{}` has no params type", operation.name))?;
            let Some(id) = handle_id(namespace, operation, schema)? else {
                continue;
            };
            if operation.kind == Kind::Event
                && sub_object.is_none()
                && name == "on"
                && LISTENERS.contains(&namespace)
            {
                let listener = listener(layer, namespace, operation, schema)?;
                found
                    .get_or_insert_with(|| Handle::of(namespace, id))
                    .listener = Some(listener);
                continue;
            }
            let Some(form) = form(operation, name, schema)? else {
                continue;
            };
            let parameters = parameters(schema, &id.param);
            let typed_by = typed_by(layer, operation, &form, &parameters)?;
            let timeout_setting = timeout_setting(operation, schema)?;
            // A handle's own `close` that takes nothing a caller must pass
            // and cannot be cancelled is how its program closes it.
            let closes = sub_object.is_none()
                && name == "close"
                && form == Form::RoundTrip
                && !operation.cancellable
                && parameters.positional.is_empty()
                && parameters.named.is_empty();
            let form = if closes { Form::Closes } else { form };
            let unwrapped = unwrapped(layer, operation, &form, typed_by.as_ref())?;
            let member = Member {
                operation,
                name,
                form,
                parameters,
                typed_by,
                timeout_setting,
                unwrapped,
            };
            let handle = found.get_or_insert_with(|| Handle::of(namespace, id));
            match sub_object {
                None => handle.members.push(member),
                Some(sub_object) => {
                    match handle
                        .sub_objects
                        .iter_mut()
                        .find(|held| held.name == sub_object)
                    {
                        Some(held) => held.members.push(member),
                        None => handle.sub_objects.push(SubObject {
                            name: sub_object,
                            members: vec![member],
                        }),
                    }
                }
            }
        }
        handles.extend(found);
    }
    check_opened_values(layer, &handles)?;
    let builders = builders(layer, &handles)?;
    for (parent, builder) in builders {
        handles[parent].builders.push(builder);
    }
    Ok(handles)
}

/// Fails when a `new_<value>` member opens no handle, or its result does
/// not carry the value's ID.
fn check_opened_values(layer: &Layer, handles: &[Handle<'_>]) -> Result<()> {
    let members = handles.iter().flat_map(|handle| {
        handle
            .members
            .iter()
            .chain(handle.sub_objects.iter().flat_map(|held| &held.members))
    });
    for member in members {
        let Form::Opens(opened) = &member.form else {
            continue;
        };
        let carried = layer
            .definitions
            .get(&member.operation.result.0)
            .is_some_and(|result| names(result, "required").contains(&format!("{opened}_id")));
        let held = handles
            .iter()
            .any(|handle| handle.namespace == opened && !handle.id.value);
        if !carried || !held {
            bail!(
                "operation `{}`: `new_` opens a value the caller closes (ADR 0099), so `{opened}` is a handle and the result carries its `{opened}_id`",
                member.operation.name
            );
        }
    }
    Ok(())
}

/// Each value a handle builds without a call, with the index of the handle
/// that builds it: the one whose ID the value holds.
fn builders(layer: &Layer, handles: &[Handle<'_>]) -> Result<Vec<(usize, Builder)>> {
    let mut builders = Vec::new();
    for built in handles.iter().filter(|handle| handle.id.value) {
        let schema = layer
            .definitions
            .get(&built.id.type_name)
            .with_context(|| format!("the bundle has no type `{}`", built.id.type_name))?;
        let required = names(schema, "required");
        let Some(parent) = handles
            .iter()
            .position(|handle| !handle.id.value && required.contains(&handle.id.param))
        else {
            bail!(
                "the value `{}` of the `{}` operations holds no handle's ID, so no handle builds it",
                built.id.type_name,
                built.namespace
            );
        };
        builders.push((
            parent,
            Builder {
                builds: built.namespace.to_owned(),
                type_name: built.id.type_name.clone(),
                description: schema["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                parameters: parameters(schema, &handles[parent].id.param),
            },
        ));
    }
    Ok(builders)
}
