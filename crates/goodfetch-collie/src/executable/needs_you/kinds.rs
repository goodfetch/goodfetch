//! The prompts of the engine's and Collie's kinds of "Needs you" items, and
//! what an item of any kind offers: the answers its `actions` let a person
//! give here, and what they can do about it elsewhere.

use goodfetch_protocol::catalog::{Operation, Product, Reach};
use goodfetch_protocol::checkpoint::RUN_RESUME;
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::needs_you::{
    CHANGED_CERTIFICATE, NEEDS_YOU_DISMISS, NeedsYouDismissParams, NeedsYouItem,
    RUN_STOPPED_BY_LIMIT, RUN_STOPPED_BY_STORAGE, STORAGE_LEVEL,
};
use goodfetch_protocol::run::ReachedLimit;
use goodfetch_protocol::tls::{CERTS_ACCEPT, CERTS_FORGET, CertsAcceptParams, CertsForgetParams};
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};
use serde_json::json;

use super::super::catalog_commands::command_words;
use super::super::{certs, usage};
use super::{Confirmation, ItemAnswer, ItemPrompt};

/// The prompts of the engine's and Collie's kinds.
const COLLIES: &[ItemPrompt] = &[
    ItemPrompt {
        kind: STORAGE_LEVEL,
        answers: &[],
        outside: Some(|item| format!("free space on {}", folder(item))),
    },
    ItemPrompt {
        kind: RUN_STOPPED_BY_STORAGE,
        answers: &[],
        outside: Some(|item| {
            format!(
                "free space on {}, then start the Run again with its Run ID",
                folder(item)
            )
        }),
    },
    ItemPrompt {
        kind: RUN_STOPPED_BY_LIMIT,
        answers: &[ItemAnswer {
            label: "Raise the Limit",
            operation: &USAGE_RAISE,
            input: Some(|item| match limit(item) {
                Ok((params, _)) => {
                    let unit = params
                        .currency
                        .as_ref()
                        .map(|currency| format!(", in millionths of {currency}"))
                        .unwrap_or_default();
                    format!(
                        "The {} allows {}. Its new amount{unit}:",
                        usage::limit(&params),
                        usage::amount(params.amount, params.currency.as_deref())
                    )
                }
                Err(_) => "Its new amount:".to_owned(),
            }),
            confirmation: raise,
        }],
        outside: None,
    },
    ItemPrompt {
        kind: CHANGED_CERTIFICATE,
        answers: &[
            ItemAnswer {
                label: "Accept the changed key",
                operation: &CERTS_ACCEPT,
                input: None,
                confirmation: |item, _| {
                    let host = detail(item, "host")?;
                    let changed = detail(item, "presented_spki_sha256")?;
                    let pinned = detail(item, "pinned_spki_sha256").ok();
                    Ok(Confirmation {
                        question: certs::accept_question(host, pinned, changed),
                        warning: None,
                        params: json!(CertsAcceptParams {
                            host: host.to_owned(),
                            spki_sha256: changed.to_owned(),
                        }),
                        then: None,
                    })
                },
            },
            ItemAnswer {
                label: "Forget the pin",
                operation: &CERTS_FORGET,
                input: None,
                confirmation: |item, _| {
                    let host = detail(item, "host")?;
                    let pinned = detail(item, "pinned_spki_sha256").ok();
                    Ok(Confirmation {
                        question: certs::forget_question(host, pinned),
                        warning: None,
                        params: json!(CertsForgetParams {
                            host: host.to_owned(),
                        }),
                        then: None,
                    })
                },
            },
        ],
        outside: None,
    },
];

/// The answer of every item whose source lets a person dismiss it.
const DISMISS: ItemAnswer = ItemAnswer {
    label: "Dismiss it",
    operation: &NEEDS_YOU_DISMISS,
    input: None,
    confirmation: |item, _| {
        Ok(Confirmation {
            question: "Dismiss this item? It does not come back.".to_owned(),
            warning: None,
            params: json!(NeedsYouDismissParams {
                item: item.id.clone(),
            }),
            then: None,
        })
    },
};

/// The detail `name` of `item`, or that it lacks it, as an item of a newer
/// engine may.
fn detail<'item>(item: &'item NeedsYouItem, name: &str) -> Result<&'item str, String> {
    item.details
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("This item does not say its `{name}`, which this answer needs."))
}

/// The folder of `item`, whose filesystem lacks space.
fn folder(item: &NeedsYouItem) -> &str {
    detail(item, "folder").unwrap_or("its filesystem")
}

/// The raise of the Limit that stopped `item`'s Run, at the amount it
/// allows now, which that gives too.
fn limit(item: &NeedsYouItem) -> Result<(UsageRaiseParams, u64), String> {
    let lacks = |name: &str| {
        format!(
            "This item does not say its `{name}` as this executable reads it: raise the Limit with `usage raise`."
        )
    };
    let reached = item
        .details
        .get("limit")
        .and_then(|limit| serde_json::from_value::<ReachedLimit>(limit.clone()).ok())
        .ok_or_else(|| lacks("limit"))?;
    let amount = item
        .details
        .get("amount")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| lacks("amount"))?;
    let params = UsageRaiseParams {
        scope: reached.scope.kind,
        scope_id: Some(reached.scope.id).filter(|id| !id.is_empty()),
        usage_kind: reached.usage_kind,
        currency: reached.currency,
        period: reached.period,
        amount,
    };
    Ok((params, amount))
}

/// The raise, to the amount the person typed, of the Limit that stopped
/// `item`'s Run. A Recipe's Run is then resumed by its own command.
fn raise(item: &NeedsYouItem, typed: &str) -> Result<Confirmation, String> {
    let (params, _) = limit(item)?;
    let amount = typed
        .parse()
        .map_err(|_| format!("`{typed}` is no amount: type a whole number, such as 1000."))?;
    let params = UsageRaiseParams { amount, ..params };
    let resumed = item
        .run
        .as_ref()
        .filter(|_| offers(item, &RUN_RESUME))
        .map(|run| {
            format!(
                "Resume the Run with `{} {run}`.",
                command_words(RUN_RESUME.name)
            )
        });
    Ok(Confirmation {
        question: usage::question(&params),
        warning: None,
        params: json!(params),
        then: resumed,
    })
}

/// Whether `item` lists `operation` among what answers it.
fn offers(item: &NeedsYouItem, operation: &Operation) -> bool {
    item.actions.iter().any(|action| action == operation.name)
}

/// The kinds an executable prompts for: Collie's and its own.
#[derive(Clone, Copy)]
pub(super) struct Kinds {
    /// The executable's Product, whose catalog its commands come from.
    pub(super) product: Product,
    /// The prompts its own kinds get.
    pub(super) own: &'static [ItemPrompt],
}

impl Kinds {
    /// The prompt of `kind`, if it has one.
    fn prompt(self, kind: &str) -> Option<&'static ItemPrompt> {
        COLLIES
            .iter()
            .chain(self.own)
            .find(|prompt| prompt.kind == kind)
    }

    /// The answers a person gives `item` here: its kind's whose operation it
    /// lists, then its dismissal, where its source lets a person dismiss it.
    pub(super) fn answers(self, item: &NeedsYouItem) -> Vec<&'static ItemAnswer> {
        self.prompt(&item.kind)
            .into_iter()
            .flat_map(|prompt| prompt.answers)
            .chain([&DISMISS])
            .filter(|answer| offers(item, answer.operation))
            .collect()
    }

    /// What can be done about `item` beside its answers here: outside the
    /// engine, by a command, from an SDK, or in the Kennel, each as a
    /// sentence; that its summary says so, for an item nothing answers.
    pub(super) fn elsewhere(self, item: &NeedsYouItem) -> Vec<String> {
        let answers = self.answers(item);
        let mut said = Vec::new();
        if let Some(outside) = self.prompt(&item.kind).and_then(|prompt| prompt.outside) {
            said.push(format!("To do: {}.", outside(item)));
        }
        let unprompted = item.actions.iter().filter(|action| {
            answers
                .iter()
                .all(|answer| answer.operation.name != **action)
        });
        for action in unprompted {
            let words = command_words(action);
            let reach = operations_of(self.product)
                .find(|operation| operation.name == action)
                .map(|operation| operation.reach);
            said.push(match reach {
                Some(Reach::AllSurfaces) => format!("By a command: `{words}`."),
                Some(Reach::SdkOnly) => format!("From an SDK: `{action}`."),
                Some(Reach::PersonOnly) => format!("In the Kennel: {words}."),
                None => format!("With the Product that lists it: `{action}`."),
            });
        }
        if answers.is_empty() && said.is_empty() {
            said.push("Nothing here answers it: its summary says what does.".to_owned());
        }
        said
    }
}
