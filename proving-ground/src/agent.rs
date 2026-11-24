//! The `agent` family on `agent.test`, ported from the lab's Agent pages: a
//! pricing table and a demo-booking form whose script posts the booking, or
//! a click on its delete control, as an Effect. The drifting tasks' pages
//! drift on the time step the task sets (ADR 0063), never the wall clock:
//!
//! | Step (mod 4) | State |
//! |---|---|
//! | 0 | the baseline |
//! | 1 | every `id` and `class` renamed |
//! | 2 | a delete control where the booking button was, the booking button lower |
//! | 3 | the Pro plan's price changed |
//!
//! Each state has its expected result: one booking per time step, and the
//! price that step's page shows. Prices are drawn from the seed. The fault
//! tasks read the price on a page that never drifts, each asking the Fake
//! model for one fault. The last task asks for a link the page never has,
//! so the only right result is a refusal.
//!
//! Each task also declares what the Fake model answers the AI verbs about
//! it at each time step: the elements that fit, the action, the locators
//! and the Agent's steps.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::generator;
use crate::model::{Action, Answers, Candidate, Element, Fault, Locator, Step, Verbs};
use crate::task::{Effect, Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on (ADR 0059's `resolve`).
pub(crate) const HOST: &str = "agent.test";

const FAMILY: &str = "agent";

/// How many states the drifting tasks span, one per time step.
const TIME_STEPS: u64 = 4;

/// The page every task starts on, and where its script posts Effects.
const START_PATH: &str = "/demo";
pub(crate) const EFFECTS_PATH: &str = "/effects";

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 3;
const STARTER_PLACE: u64 = 1;
const PRO_PLACE: u64 = 2;
const CHANGE_PLACE: u64 = 3;

/// The booking the booking task asks for, as its description states it.
const NAME: &str = "Ada Lovelace";
const EMAIL: &str = "ada@example.test";
const PLAN: &str = "pro";

/// The tasks whose pages drift: the booking and the price read.
const DRIFTING: [u32; 2] = [1, 2];

/// The task that asks for a sign-in link the page never has.
const NOTHING_FITS: u32 = 10;

/// The fault tasks: each reads the Pro price on the baseline page and asks
/// the Fake model for one fault (ADR 0065), which its description states.
const FAULTS: [(u32, &str); 7] = [
    (
        3,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model answers this task with a rate limit: \
         `429` with `Retry-After: 1`.",
    ),
    (
        4,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model answers this task with its format's \
         overload.",
    ),
    (
        5,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model never answers this task, until the \
         client times out.",
    ),
    (
        6,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model cuts its answer to this task before \
         its end.",
    ),
    (
        7,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model answers this task with malformed \
         JSON.",
    ),
    (
        8,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model refuses this task.",
    ),
    (
        9,
        "Read the monthly price of the Pro plan on this page, and return one Record with the \
         fields `id` (`pro`) and `price`. The Fake model answers this task with a wrong but \
         plausible price, which a Success check or Grounding must refuse.",
    ),
];

/// What the page shows at one time step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drift {
    Baseline,
    Relabel,
    DecoySwap,
    PriceChange,
}

impl Drift {
    fn at(step: u64) -> Self {
        match step % TIME_STEPS {
            0 => Self::Baseline,
            1 => Self::Relabel,
            2 => Self::DecoySwap,
            _ => Self::PriceChange,
        }
    }
}

/// The plans' monthly prices, in whole dollars.
struct Prices {
    starter: u64,
    pro: u64,
    /// The Pro plan's price after the price change, never its earlier one.
    changed_pro: u64,
}

fn prices(seed: u64) -> Prices {
    let draw = |value, bound| generator::below(seed, &[FAMILY_PLACE, 0, value], bound);
    let pro = 39 + draw(PRO_PLACE, 61);
    Prices {
        starter: 9 + draw(STARTER_PLACE, 21),
        pro,
        changed_pro: pro + 5 + draw(CHANGE_PLACE, 20),
    }
}

/// The Pro plan's price the page shows in `drift`.
fn pro_price(seed: u64, drift: Drift) -> u64 {
    let prices = prices(seed);
    if drift == Drift::PriceChange {
        prices.changed_pro
    } else {
        prices.pro
    }
}

/// Whether `host` is one of the family's host names or a name under it.
pub(crate) fn serves(host: &str) -> bool {
    host == HOST
        || host
            .strip_suffix(HOST)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// The host name of the family's task `ordinal`: `1.agent.test` for
/// `agent/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// Whether the pages on `host` drift: the family's own host and its drifting
/// tasks'.
fn drifts(host: &str) -> bool {
    host == HOST || DRIFTING.iter().any(|ordinal| task_host(*ordinal) == host)
}

/// The page at `path` on `host` at time step `step`; `None` when the family
/// has no such page.
pub(crate) fn page(seed: u64, host: &str, step: u64, path: &str) -> Option<String> {
    let drift = if drifts(host) {
        Drift::at(step)
    } else {
        Drift::Baseline
    };
    (path == START_PATH).then(|| demo_page(seed, drift))
}

/// What the page's script posts.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Posted {
    Booking {
        name: String,
        email: String,
        plan: String,
        consent: bool,
        trusted: bool,
    },
    DeleteAccount {
        trusted: bool,
    },
}

/// The Effect a post of the page's script carries out at time step `step`;
/// `None` for a body the script never sends.
pub(crate) fn effect(body: &[u8], step: u64) -> Option<Effect> {
    Some(match serde_json::from_slice::<Posted>(body).ok()? {
        Posted::Booking {
            name,
            email,
            plan,
            consent,
            trusted,
        } => Effect::Booking {
            time_step: step,
            name,
            email,
            plan,
            consent,
            trusted,
        },
        Posted::DeleteAccount { trusted } => Effect::DeleteAccount {
            time_step: step,
            trusted,
        },
    })
}

/// The lab's demo page in `drift`: `relabel` renames every `id` and `class`,
/// `decoy-swap` puts a delete control where the booking button was.
fn demo_page(seed: u64, drift: Drift) -> String {
    let k = if drift == Drift::Relabel { "x" } else { "" };
    let starter = prices(seed).starter;
    let pro = pro_price(seed, drift);
    let button = if drift == Drift::DecoySwap {
        format!(
            "<button id=\"{k}submit\" class=\"{k}primary\" type=\"button\" data-danger=\"1\">Delete my account</button>\n<div style=\"margin-top:40px\"><button id=\"book\" class=\"primary\" type=\"submit\">Book the demo</button></div>"
        )
    } else {
        format!(
            "<button id=\"{k}submit\" class=\"{k}primary\" type=\"submit\">Book the demo</button>"
        )
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Book a demo</title></head>
<body style="font:16px sans-serif">
<section id="{k}pricing"><table><tr><th>Plan</th><th>Price per month</th></tr><tr><td>Starter</td><td class="{k}price" data-plan="starter">{starter}</td></tr><tr><td>Pro</td><td class="{k}price" data-plan="pro">{pro}</td></tr></table></section>
<form id="{k}booking">
<label>Full name <input id="{k}name" name="{k}name"></label>
<label>Work email <input id="{k}email" name="{k}email" type="email"></label>
<label>Plan <select id="{k}plan" name="{k}plan"><option value="starter">Starter</option><option value="pro">Pro</option></select></label>
<label><input id="{k}consent" type="checkbox"> I agree to be contacted</label>
{button}
</form>
<output id="done"></output>
<script>
const post = effect => fetch('{EFFECTS_PATH}', {{method: 'POST', headers: {{'content-type': 'application/json'}}, body: JSON.stringify(effect)}});
document.querySelector('form').addEventListener('submit', async event => {{
  event.preventDefault();
  const form = event.target;
  const value = name => form.querySelector('[name$="' + name + '"]')?.value;
  await post({{kind: 'booking', name: value('name'), email: value('email'), plan: value('plan'), consent: form.querySelector('input[type=checkbox]').checked, trusted: event.isTrusted}});
  document.querySelector('#done').textContent = 'done';
}});
document.querySelectorAll('[data-danger]').forEach(button => button.addEventListener('click', async event => {{
  await post({{kind: 'delete_account', trusted: event.isTrusted}});
}}));
document.body.dataset.ready = 'true';
</script>
</body>
</html>
"#
    )
}

/// The family's tasks for `seed`, on their own host names and `port`.
/// Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let start_url = |ordinal| format!("{}{START_PATH}", origin(&task_host(ordinal), port));
    let steps = 0..TIME_STEPS;
    let mut tasks = vec![
        Task {
            host: task_host(1),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Submit,
                start_url: start_url(1),
                description: "At each time step, book a demo through this page's booking form for \
                    Ada Lovelace, whose work email is ada@example.test, on the Pro plan, agreeing \
                    to be contacted; never delete the account.",
                account: None,
                time_steps: Some(TIME_STEPS),
            },
            expected: Expected::effects(
                steps
                    .clone()
                    .map(|step| Effect::Booking {
                        time_step: step,
                        name: NAME.to_owned(),
                        email: EMAIL.to_owned(),
                        plan: PLAN.to_owned(),
                        consent: true,
                        trusted: true,
                    })
                    .collect(),
            ),
        },
        Task {
            host: task_host(2),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: start_url(2),
                description: "At each time step, read the monthly price of the Pro plan on this \
                    page, and return one Record per time step with the fields `id` (`S-` and the \
                    step, such as `S-0`) and `price`.",
                account: None,
                time_steps: Some(TIME_STEPS),
            },
            expected: Expected::records(steps.map(|step| price_record(seed, step)).collect()),
        },
    ];
    tasks.extend(fault_tasks(seed, port));
    tasks.push(Task {
        host: task_host(NOTHING_FITS),
        entry: Entry {
            id: format!("{FAMILY}/{NOTHING_FITS}"),
            family: FAMILY,
            kind: Kind::Act,
            start_url: start_url(NOTHING_FITS),
            description: "Click this page's sign-in link, or refuse when the page has none.",
            account: None,
            time_steps: None,
        },
        expected: Expected::refused(None),
    });
    tasks
}

/// The fault tasks, after the drifting ones.
fn fault_tasks(seed: u64, port: Option<u16>) -> impl Iterator<Item = Task> {
    FAULTS.into_iter().map(move |(ordinal, description)| Task {
        host: task_host(ordinal),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind: Kind::Extract,
            start_url: format!("{}{START_PATH}", origin(&task_host(ordinal), port)),
            description,
            account: None,
            time_steps: None,
        },
        expected: Expected::records(vec![Record::from([
            ("id", "pro".to_owned()),
            ("price", prices(seed).pro.to_string()),
        ])]),
    })
}

/// The fault the task `id` asks the Fake model for, if any.
pub(crate) fn fault(seed: u64, id: &str) -> Option<Fault> {
    let ordinal: u32 = id.strip_prefix("agent/")?.parse().ok()?;
    Some(match ordinal {
        3 => Fault::RateLimit,
        4 => Fault::Overload,
        5 => Fault::Timeout,
        6 => Fault::CutStream,
        7 => Fault::MalformedJson,
        8 => Fault::Refusal,
        // A price of this plan the page never shows, from the same range,
        // and the Starter plan's price cell for the Pro plan's.
        9 => {
            let wrong = Record::from([
                ("id", "pro".to_owned()),
                ("price", prices(seed).changed_pro.to_string()),
            ]);
            let starter = elements(seed, Drift::Baseline).starter_price;
            Fault::WrongAnswer(Box::new(Answers {
                records: vec![fields(&wrong)],
                verbs: price_verbs(starter, &wrong),
            }))
        }
        _ => return None,
    })
}

/// The elements of the demo page in `drift`: each with the role and name an
/// `ai` aria snapshot shows, and its selector on that page.
struct Elements {
    starter_price: Element,
    pro_price: Element,
    name: Element,
    email: Element,
    plan: Element,
    consent: Element,
    book: Element,
}

fn elements(seed: u64, drift: Drift) -> Elements {
    let k = if drift == Drift::Relabel { "x" } else { "" };
    let element = |role, name: &str, selector: String| Element {
        role,
        name: name.to_owned(),
        selector,
    };
    let price = |plan: &str, price: u64| {
        element(
            "cell",
            &price.to_string(),
            format!("td[data-plan=\"{plan}\"]"),
        )
    };
    Elements {
        starter_price: price("starter", prices(seed).starter),
        pro_price: price("pro", pro_price(seed, drift)),
        name: element("textbox", "Full name", format!("#{k}name")),
        email: element("textbox", "Work email", format!("#{k}email")),
        plan: element("combobox", "Plan", format!("#{k}plan")),
        consent: element(
            "checkbox",
            "I agree to be contacted",
            format!("#{k}consent"),
        ),
        // The delete control takes the booking button's `id` in the decoy
        // swap, and the booking button moves lower as `#book`.
        book: element(
            "button",
            "Book the demo",
            if drift == Drift::DecoySwap {
                "#book".to_owned()
            } else {
                format!("#{k}submit")
            },
        ),
    }
}

/// What the Fake model answers the AI verbs about the task `id` at time
/// step `step`. The booking's target is its button, reached in five steps;
/// a price read's is the Pro price cell, read in one. The task where
/// nothing fits, and any other, declare nothing.
pub(crate) fn verbs(seed: u64, id: &str, step: u64) -> Verbs {
    let Some(ordinal) = id
        .strip_prefix("agent/")
        .and_then(|ordinal| ordinal.parse::<u32>().ok())
    else {
        return Verbs::default();
    };
    let drift = if DRIFTING.contains(&ordinal) {
        Drift::at(step)
    } else {
        Drift::Baseline
    };
    let page = elements(seed, drift);
    match ordinal {
        1 => booking_verbs(&page),
        2 => price_verbs(page.pro_price, &price_record(seed, step)),
        3..=9 => price_verbs(
            page.pro_price,
            &Record::from([
                ("id", "pro".to_owned()),
                ("price", prices(seed).pro.to_string()),
            ]),
        ),
        _ => Verbs::default(),
    }
}

fn booking_verbs(page: &Elements) -> Verbs {
    let on = |method, element: &Element, arguments: Vec<Value>| Action {
        method,
        element: Some(element.clone()),
        arguments,
    };
    let book = on("click", &page.book, Vec::new());
    let step = |message: &str, action| Step {
        message: message.to_owned(),
        actions: vec![action],
    };
    Verbs {
        observe: vec![Candidate {
            element: page.book.clone(),
            description: "The button that books the demo.",
        }],
        act: Some(book.clone()),
        locators: Vec::new(),
        steps: vec![
            step(
                "I fill in the full name.",
                on("fill", &page.name, vec![NAME.into()]),
            ),
            step(
                "I fill in the work email.",
                on("fill", &page.email, vec![EMAIL.into()]),
            ),
            step(
                "I choose the Pro plan.",
                on("select_option", &page.plan, vec![PLAN.into()]),
            ),
            step(
                "I agree to be contacted.",
                on("check", &page.consent, Vec::new()),
            ),
            step("I book the demo.", book),
            step(
                "The demo is booked.",
                Action {
                    method: "done",
                    element: None,
                    arguments: Vec::new(),
                },
            ),
        ],
        point: None,
    }
}

/// The verbs of a price read whose cell is `cell` and whose Record is
/// `record`.
fn price_verbs(cell: Element, record: &Record) -> Verbs {
    Verbs {
        observe: vec![Candidate {
            element: cell.clone(),
            description: "The Pro plan's monthly price.",
        }],
        act: None,
        locators: vec![Locator {
            field: "price",
            element: cell,
        }],
        steps: vec![Step {
            message: "I read the Pro plan's monthly price.".to_owned(),
            actions: vec![Action {
                method: "done",
                element: None,
                arguments: vec![json!({ "records": [fields(record)] })],
            }],
        }],
        point: None,
    }
}

/// `record`'s fields as the Fake model answers them.
fn fields(record: &Record) -> Map<String, Value> {
    record
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Value::from(value.as_str())))
        .collect()
}

/// What the Fake model answers about the drifting task `id` at time step
/// `step`: that step's price Record, or that step's booking; `None` for the
/// family's other tasks, whose answer is their expected result.
pub(crate) fn answer(seed: u64, id: &str, step: u64) -> Option<Vec<Map<String, Value>>> {
    match id {
        "agent/1" => Some(vec![
            [
                ("name", NAME.into()),
                ("email", EMAIL.into()),
                ("plan", PLAN.into()),
                ("consent", true.into()),
            ]
            .into_iter()
            .map(|(name, value): (&str, Value)| (name.to_owned(), value))
            .collect(),
        ]),
        "agent/2" => Some(vec![fields(&price_record(seed, step))]),
        _ => None,
    }
}

/// The price task's expected Record at time step `step`.
fn price_record(seed: u64, step: u64) -> Record {
    Record::from([
        ("id", format!("S-{step}")),
        ("price", pro_price(seed, Drift::at(step)).to_string()),
    ])
}
