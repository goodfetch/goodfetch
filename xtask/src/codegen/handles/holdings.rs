//! Which handle a handle holds (ADRs 0078, 0092): one whose ID every answer
//! that builds it requires, an opener's result or a listener's event's
//! payload, so the handle answers it without a call, as a page answers its
//! Session. Each opener and listener that builds the handle is told how it
//! gets the held one, and a handle whose opener keeps held handles is told
//! which.

use anyhow::{Result, bail};

use super::{Form, Handle, Holding, Holds, names};
use crate::codegen::layer::Layer;

/// One answer that builds a handle: the handle whose opener or listener
/// gives it, its type in the bundle, and the entry that gives it, as a
/// refusal names it.
struct Answer {
    builder: usize,
    type_name: String,
    entry: String,
    /// Whether a listener's event gives it, rather than an opener.
    listened: bool,
    /// Whether a member of a sub-object gives it.
    nested: bool,
}

/// Reads the handle each of `handles` holds, and how each opener and
/// listener that builds it gets that handle; fails when an answer that
/// builds a handle lacks the ID the others name, when they name two
/// handles, when a member already has the held handle's name, or when a
/// listener of another handle builds it.
pub(super) fn holdings(layer: &Layer, handles: &mut [Handle<'_>]) -> Result<()> {
    for built in 0..handles.len() {
        if handles[built].id.value {
            continue;
        }
        let answers = answers(handles, built);
        let Some(holds) = held(layer, handles, built, &answers)? else {
            continue;
        };
        let namespace = handles[built].namespace;
        if named(&handles[built], holds.namespace) {
            bail!(
                "a `{namespace}` answers its `{}` without a call, by a member of that name, which the `{namespace}` operations already give (ADR 0078)",
                holds.namespace
            );
        }
        for answer in &answers {
            if answer.nested {
                bail!(
                    "{}: a sub-object's member opens a `{namespace}`, which holds a `{}`, so only a handle's own members open one (ADR 0078)",
                    answer.entry,
                    holds.namespace
                );
            }
            let builder = &mut handles[answer.builder];
            let itself = builder.namespace == holds.namespace;
            if answer.listened {
                if !itself {
                    bail!(
                        "{}: it builds a `{namespace}`, which holds a `{}`, so only that `{}`'s listener builds one (ADR 0078)",
                        answer.entry,
                        holds.namespace,
                        holds.namespace
                    );
                }
                mark_listened(builder, namespace);
                continue;
            }
            let holding = if itself {
                Holding::Builder(holds.clone())
            } else {
                if !builder
                    .keeps
                    .iter()
                    .any(|kept| kept.namespace == holds.namespace)
                {
                    builder.keeps.push(holds.clone());
                }
                Holding::Kept(holds.clone())
            };
            mark_opener(builder, namespace, &holding);
        }
        handles[built].holds = Some(holds);
    }
    for handle in handles.iter() {
        for kept in &handle.keeps {
            let reached = handles
                .iter()
                .find(|other| other.namespace == kept.namespace);
            if reached.is_some_and(|other| {
                other.reads.is_some() || other.holds.is_some() || !other.keeps.is_empty()
            }) {
                bail!(
                    "a `{}` keeps the `{}` its answers name, built from its ID alone, so a `{}` has no reads and holds and keeps no handle (ADR 0078)",
                    handle.namespace,
                    kept.namespace,
                    kept.namespace
                );
            }
        }
    }
    Ok(())
}

/// Each answer that builds `handles[built]`: its openers' results and its
/// builders' listeners' payloads, in the handles' order.
fn answers(handles: &[Handle<'_>], built: usize) -> Vec<Answer> {
    let namespace = handles[built].namespace;
    let mut answers = Vec::new();
    for (builder, handle) in handles.iter().enumerate() {
        let members = handle.members.iter().map(|member| (member, false)).chain(
            handle
                .sub_objects
                .iter()
                .flat_map(|held| held.members.iter().map(|member| (member, true))),
        );
        for (member, nested) in members {
            if matches!(&member.form, Form::Opens(opened) if opened == namespace) {
                answers.push(Answer {
                    builder,
                    type_name: member.operation.result.0.clone(),
                    entry: format!("operation `{}`", member.operation.name),
                    listened: false,
                    nested,
                });
            }
        }
        for listener in handle.listener.iter().chain(&handle.published_listener) {
            for listened in &listener.events {
                if listened.builds == Some(namespace) {
                    answers.push(Answer {
                        builder,
                        type_name: listened.payload.clone(),
                        entry: format!("event `{}.{}`", handle.namespace, listened.wire),
                        listened: true,
                        nested: false,
                    });
                }
            }
        }
    }
    answers
}

/// The handle every one of `answers` names by its ID, which
/// `handles[built]` holds; none when no answer names one.
fn held(
    layer: &Layer,
    handles: &[Handle<'_>],
    built: usize,
    answers: &[Answer],
) -> Result<Option<Holds>> {
    let namespace = handles[built].namespace;
    let mut holds: Option<Holds> = None;
    let others = handles
        .iter()
        .enumerate()
        .filter(|(index, handle)| *index != built && !handle.id.value)
        .map(|(_, handle)| handle);
    for other in others {
        let names_it = |answer: &Answer| {
            layer
                .definitions
                .get(&answer.type_name)
                .is_some_and(|schema| names(schema, "required").contains(&other.id.param))
        };
        if !answers.iter().any(names_it) {
            continue;
        }
        if let Some(lacking) = answers.iter().find(|answer| !names_it(answer)) {
            bail!(
                "{}: its answer `{}` lacks `{}`, which every other answer that builds a `{namespace}` requires, so each `{namespace}` holds its `{}` (ADR 0078)",
                lacking.entry,
                lacking.type_name,
                other.id.param,
                other.namespace
            );
        }
        if let Some(first) = &holds {
            bail!(
                "a `{namespace}` holds one handle, but the answers that build it name both a `{}` and a `{}` (ADR 0078)",
                first.namespace,
                other.namespace
            );
        }
        holds = Some(Holds {
            namespace: other.namespace,
            id: other.id.param.clone(),
            type_name: other.id.type_name.clone(),
        });
    }
    Ok(holds)
}

/// Whether `handle` already has a member, sub-object, read or builder named
/// `name`.
fn named(handle: &Handle<'_>, name: &str) -> bool {
    handle.members.iter().any(|member| member.name == name)
        || handle.sub_objects.iter().any(|held| held.name == name)
        || handle.reads().iter().any(|read| read.name == name)
        || handle.builders.iter().any(|builder| builder.builds == name)
}

/// Marks each event of `builder`'s listeners that builds a `built` as
/// building it with `builder`.
fn mark_listened(builder: &mut Handle<'_>, built: &str) {
    for listener in builder
        .listener
        .iter_mut()
        .chain(builder.published_listener.iter_mut())
    {
        for listened in &mut listener.events {
            if listened.builds == Some(built) {
                listened.holds = true;
            }
        }
    }
}

/// Marks each opener of `builder` that opens a `built` with `holding`.
fn mark_opener(builder: &mut Handle<'_>, built: &str, holding: &Holding) {
    for member in &mut builder.members {
        if matches!(&member.form, Form::Opens(opened) if opened == built) {
            member.holding = Some(holding.clone());
        }
    }
}
