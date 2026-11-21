//! A handle's reads (issue 2798): which operations answer a value of the
//! answer that opened their handle, which the handle holds, so a read sends
//! nothing, and the rules a read keeps for that answer to be the one it
//! reads.

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{Form, Handle, HandleId, Member, names};
use crate::codegen::layer::{Kind, Layer, Operation};

/// A handle's reads: the handle is built with the answer that opened it.
pub(in crate::codegen) struct Reads<'a> {
    /// The type each opener of the handle answers, such as `RunStarted`,
    /// which the handle is built with.
    pub(in crate::codegen) opened: String,
    /// Its reads, in the catalog's order.
    pub(in crate::codegen) members: Vec<Read<'a>>,
}

/// One read of a handle.
pub(in crate::codegen) struct Read<'a> {
    pub(in crate::codegen) operation: &'a Operation,
    /// The last segment of its name, such as `started_as`.
    pub(in crate::codegen) name: &'a str,
    /// The one property of its result, which the opened answer holds with
    /// the same type, as the wire names it.
    pub(in crate::codegen) property: String,
}

/// `operation`, the read `name` of `namespace`'s handle, whose params are
/// `schema` and whose handle `id` names: it is the handle's own, takes
/// nothing but the ID, and its result's one property, required, is one the
/// answer of each opener of the namespace holds with the same type, which
/// the read answers without a message. Answers that type with the read.
pub(super) fn read<'a>(
    layer: &Layer,
    namespace: &str,
    operation: &'a Operation,
    name: &'a str,
    sub_object: Option<&str>,
    schema: &Value,
    id: &HandleId,
) -> Result<(String, Read<'a>)> {
    let refuse = |why: &str| -> Result<(String, Read<'a>)> {
        bail!(
            "operation `{}`: a read answers a value of the answer that opened its handle, without a message, so {why} (issue 2798)",
            operation.name
        )
    };
    if sub_object.is_some() {
        return refuse("it is its handle's own member, not a sub-object's");
    }
    let taken = schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .any(|(param, _)| *param != id.param);
    if taken {
        return refuse(&format!("it takes nothing but `{}`", id.param));
    }
    let result = layer
        .definitions
        .get(&operation.result.0)
        .with_context(|| format!("operation `{}` has no result type", operation.name))?;
    let mut only = result["properties"].as_object().into_iter().flatten();
    let (Some((property, answered)), None) = (only.next(), only.next()) else {
        return refuse("its result has one property, the value it answers");
    };
    if !names(result, "required").contains(property) {
        return refuse(&format!("its result's `{property}` is required"));
    }
    let mut opened = layer
        .operations
        .iter()
        .filter(|held| {
            held.kind == Kind::Opener
                && held
                    .name
                    .split_once('.')
                    .is_some_and(|(owner, _)| owner == namespace)
        })
        .map(|opener| opener.result.0.as_str())
        .collect::<Vec<_>>();
    opened.sort_unstable();
    opened.dedup();
    let [opened] = opened[..] else {
        return refuse(&format!(
            "`{namespace}` has an opener, and each of its openers answers the same type"
        ));
    };
    let holds = layer.definitions.get(opened).is_some_and(|answer| {
        names(answer, "required").contains(property)
            && typed(&answer["properties"][property]) == typed(answered)
    });
    if !holds {
        return refuse(&format!(
            "`{opened}`, which opens `{namespace}`, holds its result's `{property}`, required and of the same type"
        ));
    }
    Ok((
        opened.to_owned(),
        Read {
            operation,
            name,
            property: property.clone(),
        },
    ))
}

/// `property`'s schema without the keywords that say nothing of its type:
/// its description and the version it arrived in.
fn typed(property: &Value) -> Value {
    let mut typed = property.clone();
    if let Some(keywords) = typed.as_object_mut() {
        keywords.remove("description");
        keywords.remove("arrived_in");
    }
    typed
}

/// Fails when a handle with reads is built another way than by its own
/// opener, whose answer its reads answer from: by another handle's
/// `new_<value>`, as a value, or by a listener.
pub(super) fn check_opened(handles: &[Handle<'_>]) -> Result<()> {
    for handle in handles {
        let Some(reads) = &handle.reads else {
            continue;
        };
        let namespace = handle.namespace;
        let opens = |member: &Member<'_>| matches!(&member.form, Form::Opens(opened) if opened == namespace);
        let built = handle.id.value
            || handles.iter().any(|other| {
                other.members.iter().any(opens)
                    || other
                        .sub_objects
                        .iter()
                        .any(|held| held.members.iter().any(opens))
                    || other
                        .listener
                        .iter()
                        .chain(&other.published_listener)
                        .any(|listener| {
                            listener
                                .events
                                .iter()
                                .any(|listened| listened.builds == Some(namespace))
                        })
            });
        if let Some(read) = reads.members.first().filter(|_| built) {
            bail!(
                "operation `{}`: a read answers a value of the answer that opened its handle, so `{namespace}` is built only by its opener, which answers `{}` (issue 2798)",
                read.operation.name,
                reads.opened
            );
        }
    }
    Ok(())
}
