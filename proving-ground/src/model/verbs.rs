//! What the AI verbs ask a model for, as a task declares it: `observe`'s
//! candidates, `act`'s one action, compiled `extract`'s locators,
//! `page.agent`'s steps and the vision rung's point on a screenshot.
//!
//! A request asks for one of these forms by its schema (the structured
//! output's, or the parameters of the tool it calls): the form whose
//! answer's field is a top-level property of it, `candidates`, `action`,
//! `locators`, `actions` or `coordinate`. An element is named as the
//! request's prompt offers it: by the reference an `ai` aria snapshot gives
//! it (`- button "Book the demo" [ref=e15]`), else by its selector. A point
//! is `[x, y]`, as Anthropic's computer-use tool gives one, and only a
//! request that carries an image gets one.

use serde_json::{Map, Value, json};

use super::request::Ask;

/// An element of a task's page: its ARIA role and accessible name, as an
/// `ai` aria snapshot shows them beside its reference, and its CSS selector
/// on the page at the time step.
#[derive(Clone)]
pub(crate) struct Element {
    pub(crate) role: &'static str,
    pub(crate) name: String,
    pub(crate) selector: String,
}

impl Element {
    /// The element as `strings` offer it: the reference of the first
    /// snapshot line that shows its role and name, else its selector.
    fn named(&self, strings: &[String]) -> String {
        strings
            .iter()
            .flat_map(|text| text.lines())
            .find_map(|line| self.reference(line))
            .map_or_else(|| self.selector.clone(), str::to_owned)
    }

    /// The reference `line` gives the element, when it is the snapshot line
    /// of an element of its role and name.
    fn reference<'a>(&self, line: &'a str) -> Option<&'a str> {
        let rest = line
            .trim_start()
            .strip_prefix("- ")?
            .strip_prefix(self.role)?
            .strip_prefix(" \"")?
            .strip_prefix(self.name.as_str())?
            .strip_prefix('"')?;
        let (_, after) = rest.split_once("[ref=")?;
        after.split_once(']').map(|(reference, _)| reference)
    }
}

/// An `observe` candidate: an element that fits the instruction, and what
/// it is.
#[derive(Clone)]
pub(crate) struct Candidate {
    pub(crate) element: Element,
    pub(crate) description: &'static str,
}

/// One action on the page: `act`'s answer, and each of an Agent step's.
/// `done` and `give_up` name no element.
#[derive(Clone)]
pub(crate) struct Action {
    pub(crate) method: &'static str,
    pub(crate) element: Option<Element>,
    pub(crate) arguments: Vec<Value>,
}

/// The element compiled `extract` reads one field from.
#[derive(Clone)]
pub(crate) struct Locator {
    pub(crate) field: &'static str,
    pub(crate) element: Element,
}

/// One step of `page.agent`: the model's message and its actions.
#[derive(Clone)]
pub(crate) struct Step {
    pub(crate) message: String,
    pub(crate) actions: Vec<Action>,
}

/// What a task answers each AI verb at one time step. The default is a
/// page where nothing fits the task: no candidate, no action, no locator,
/// and an Agent that gives up.
#[derive(Clone, Default)]
pub(crate) struct Verbs {
    /// None when nothing on the page fits.
    pub(crate) observe: Vec<Candidate>,
    /// `None` when nothing fits.
    pub(crate) act: Option<Action>,
    /// One for each field the page shows.
    pub(crate) locators: Vec<Locator>,
    /// The steps toward the task's goal, the last ending it with `done`;
    /// none when the Agent gives up at once.
    pub(crate) steps: Vec<Step>,
    /// Where the vision rung clicks: the middle of the task's target in
    /// the page's CSS pixels, as a screenshot of the unscrolled viewport at
    /// a device pixel ratio of 1 shows it; `None` when nothing is drawn to
    /// point at.
    pub(crate) point: Option<(u64, u64)>,
}

/// Why an Agent with no step toward its task gives up.
const NOTHING_FITS: &str = "Nothing on the page fits the task.";

/// The form of an AI verb's answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Form {
    Observe,
    Act,
    Locators,
    Step,
    Coordinate,
}

impl Form {
    /// The form `schema` asks for: the one whose answer's field is one of
    /// its top-level properties.
    pub(crate) fn asked(schema: &Value) -> Option<Self> {
        let properties = schema["properties"].as_object()?;
        [
            (Self::Observe, "candidates"),
            (Self::Act, "action"),
            (Self::Locators, "locators"),
            (Self::Step, "actions"),
            (Self::Coordinate, "coordinate"),
        ]
        .into_iter()
        .find(|(_, field)| properties.contains_key(*field))
        .map(|(form, _)| form)
    }
}

impl Verbs {
    /// The answer of `form` to `ask`, its elements named as its strings
    /// offer them. An Agent step is the one after the steps the request's
    /// turns hold, or the last once all are taken; with no step, it gives
    /// up. A point is answered only to a request that carries an image.
    pub(crate) fn answer(&self, form: Form, ask: &Ask) -> Value {
        let strings = &ask.strings;
        let taken = ask.model_turns;
        match form {
            Form::Coordinate => json!({
                "coordinate": self.point.filter(|_| ask.images > 0).map(|(x, y)| [x, y]),
            }),
            Form::Observe => json!({ "candidates": self
                .observe
                .iter()
                .map(|candidate| json!({
                    "element": candidate.element.named(strings),
                    "description": candidate.description,
                }))
                .collect::<Vec<_>>() }),
            Form::Act => json!({
                "action": self.act.as_ref().map(|action| action.value(strings)),
            }),
            Form::Locators => json!({ "locators": self
                .locators
                .iter()
                .map(|locator| json!({
                    "field": locator.field,
                    "element": locator.element.named(strings),
                }))
                .collect::<Vec<_>>() }),
            Form::Step => self.steps.get(taken).or(self.steps.last()).map_or_else(
                || {
                    json!({
                        "message": NOTHING_FITS,
                        "actions": [{ "method": "give_up", "arguments": [NOTHING_FITS] }],
                    })
                },
                |step| {
                    json!({
                        "message": step.message,
                        "actions": step
                            .actions
                            .iter()
                            .map(|action| action.value(strings))
                            .collect::<Vec<_>>(),
                    })
                },
            ),
        }
    }
}

impl Action {
    fn value(&self, strings: &[String]) -> Value {
        let mut fields = Map::new();
        fields.insert("method".to_owned(), Value::from(self.method));
        if let Some(element) = &self.element {
            fields.insert("element".to_owned(), Value::from(element.named(strings)));
        }
        fields.insert("arguments".to_owned(), Value::from(self.arguments.clone()));
        Value::Object(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Element {
        Element {
            role: "button",
            name: "Book the demo".to_owned(),
            selector: "#submit".to_owned(),
        }
    }

    #[test]
    fn an_element_is_named_by_the_reference_its_snapshot_line_gives() {
        let strings = [
            "- button \"Book\" [ref=e1]\n  - button \"Book the demo\" [ref=e15] [cursor=pointer]"
                .to_owned(),
        ];

        assert_eq!(book().named(&strings), "e15");
    }

    #[test]
    fn an_element_of_another_role_or_name_is_not_it() {
        let strings = [
            "- link \"Book the demo\" [ref=e2]\n- button \"Book the demo now\" [ref=e3]".to_owned(),
        ];

        assert_eq!(book().named(&strings), "#submit");
    }
}
