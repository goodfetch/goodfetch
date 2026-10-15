//! Collie's AI verbs and Plans (ADR 0091, decided in #473).
//!
//! - `act`, `observe` and `extract` live on the [`Page`], and on a [`Locator`], whose one element
//!   bounds what the model sees and what a step may target. There is no AI runtime object and no
//!   active page (G2).
//! - Each call is one step of the Run's Plan (ADR 0009): replayed without a model when the Plan
//!   holds it and its Target fingerprint still matches, compiled by the model when it is missing,
//!   repaired once on Drift. [`crate::Start::plan`] names the file; [`crate::settings::plan::FROZEN`]
//!   forbids any model call.
//! - Every verb resolves to a [`StepResult`]: its `output`, how the `step` was resolved, and its
//!   `usage`. Failures are errors (ADR 0079); single steps carry no Verdict, which the Run's checks
//!   give (ADR 0029).
//! - [`expect`] runs a Success check under Playwright's assertion names, evaluated live by the engine
//!   and read by the Run's Verdict; a condition still false at its timeout fails with
//!   `success_check_failed`. Negation is [`LocatorExpectations::not`], as Playwright for Java's `not()`.
//! - [`Page::aria_snapshot`] and [`Page::get_by_ref`] are the observation an agent loop reads.
//!
//! Model references and [`ModelUsage`] are ADR 0101's (`models`). Secret placeholders in
//! instructions are ADR 0092's.

use std::fmt;
use std::future::IntoFuture;
use std::marker::PhantomData;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::data::Schema;
use crate::lifecycle::Page;
use crate::models::{ModelReference, ModelUsage, StructuredOutputStrategy};
use crate::page::{Locator, Response};
#[cfg(doc)]
use crate::page::TextMatch;
use crate::results::Error;
use crate::time::{Call, CallFuture, Count};

/// How a step was resolved. A newer engine's value is kept as received (G3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepSource {
    /// Replayed from the Plan, no model call.
    Plan,
    /// Compiled now, since the Plan held no entry for it.
    Model,
    /// Its entry drifted and one Repair recompiled it.
    Repair,
    /// The Effect journal already held this step's Effect, so nothing was done again (ADR 0010).
    Journal,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// What drifted, when a step was repaired.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Drift {
    /// The Target fingerprint the Plan kept, rendered for a person.
    pub expected: String,
    /// What stood there instead, or `None` when nothing matched.
    pub observed: Option<String>,
}

/// How a call was resolved.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StepReport {
    /// Plan, model, Repair or journal.
    pub source: StepSource,
    /// The instruction as the Plan keys it, with `%name%` placeholders and never their values.
    pub instruction: String,
    /// Present when `source` is [`StepSource::Repair`].
    pub drift: Option<Drift>,
}

/// What every AI verb resolves to. A value of `output` is always complete; a failure is an [`Error`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StepResult<T> {
    /// The verb's value.
    pub output: T,
    /// How the step was resolved.
    pub step: StepReport,
    /// The model calls this step made and their price (ADR 0101): none for a step replayed from
    /// the Plan.
    pub usage: ModelUsage,
}

/// The action `act` performed, through the guard like any locator action (ADR 0019).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PerformedAction {
    /// The page API method, such as `click` or `fill`.
    pub method: String,
    /// Its arguments, `%name%` placeholders where a variable was typed.
    pub arguments: Vec<serde_json::Value>,
    /// The target, an ordinary locator resolved again at each use.
    pub target: Locator,
    /// The same call in Rust, as the engine renders it:
    /// `page.get_by_role(AriaRole::Link.name("love")).click().await?;`.
    pub code: String,
}

/// One element `observe` found for its instruction.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Observed {
    /// An ordinary locator, resolved again at each use.
    pub locator: Locator,
    /// The model's description of the element: a signal, never proof.
    pub description: String,
    /// The locator in Rust, as the engine renders it.
    pub code: String,
}

/// How `extract` reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtractMode {
    /// The model writes each field's locators and transform once, kept in the Plan, and values are
    /// copied from the page at every call (ADR 0013).
    #[default]
    Compiled,
    /// The model reads the page at every call, every value checked by Grounding.
    Model,
}

/// The mode of an aria snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AriaSnapshotMode {
    /// Playwright's YAML, as `to_match_aria_snapshot` compares it.
    #[default]
    Default,
    /// With element references, page text marked untrusted (ADR 0014), and bounded by
    /// `observation.max_size`, past which the deepest subtrees collapse into one line keeping their
    /// reference, and the output says so.
    Ai,
}

macro_rules! ai_call {
    ($name:ident $(<$t:ident>)?, $output:ty, $doc:literal) => {
        #[doc = $doc]
        #[must_use = "an AI step does nothing until it is awaited"]
        pub struct $name$(<$t>)? {
            _output: PhantomData<fn() -> $output>,
        }

        impl$(<$t>)? fmt::Debug for $name$(<$t>)? {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(..)"))
            }
        }

        impl$(<$t>)? $name$(<$t>)? {
            /// At most this long, within what remains of any enclosing timeout. Default: `timeout.ai`.
            pub fn timeout(self, timeout: Duration) -> Self {
                let _ = timeout;
                unimplemented!()
            }

            /// No timeout of its own; an enclosing timeout still applies.
            pub fn no_timeout(self) -> Self {
                unimplemented!()
            }

            /// The model for this call alone, a Model reference: `ai.model` at the run layer
            /// (ADRs 0083, 0101).
            pub fn model(self, model: impl Into<ModelReference>) -> Self {
                let _ = model.into();
                unimplemented!()
            }
        }

        impl$(<$t: Send + 'static>)? IntoFuture for $name$(<$t>)? {
            type Output = Result<StepResult<$output>, Error>;
            type IntoFuture = CallFuture<StepResult<$output>>;

            fn into_future(self) -> Self::IntoFuture {
                unimplemented!()
            }
        }
    };
}

ai_call!(Act, PerformedAction, "One action described in words, not sent until awaited.");
ai_call!(Observe, Vec<Observed>, "Finding the elements an instruction describes, not sent until awaited.");
ai_call!(Extract<T>, T, "Reading typed data, not sent until awaited.");

impl Act {
    /// A plain value for the instruction's `%name%` placeholder. A placeholder no variable fills is a
    /// Secret placeholder, bound by the setting `secrets.<name>` (ADR 0092). The model, the Plan, logs
    /// and the Run trace see only `%name%`; the engine types the value at input (ADR 0014). A
    /// placeholder neither fills, a variable no placeholder uses, or a name that is both fails with
    /// `variables_mismatch`.
    pub fn variable(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }
}

impl<T> Extract<T> {
    /// `extract.mode` for this call alone. Default: [`ExtractMode::Compiled`].
    pub fn mode(self, mode: ExtractMode) -> Self {
        let _ = mode;
        unimplemented!()
    }
}

/// An aria snapshot, not sent until awaited.
#[must_use = "a snapshot is not taken until it is awaited"]
pub struct AriaSnapshot {
    _private: (),
}

impl fmt::Debug for AriaSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AriaSnapshot(..)")
    }
}

impl AriaSnapshot {
    /// Default or AI mode.
    pub fn mode(self, mode: AriaSnapshotMode) -> Self {
        let _ = mode;
        unimplemented!()
    }

    /// How deep to go below the root.
    pub fn depth(self, depth: u32) -> Self {
        let _ = depth;
        unimplemented!()
    }

    /// Adds each element's box, `[box=x,y,width,height]`.
    pub fn boxes(self) -> Self {
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for AriaSnapshot {
    type Output = Result<String, Error>;
    type IntoFuture = CallFuture<String>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

macro_rules! ai_verbs {
    ($ty:ty) => {
        impl $ty {
            /// Performs one action described in words. Fails with `target_not_found` when nothing on
            /// the page fits it.
            pub fn act(&self, instruction: &str) -> Act {
                let _ = instruction;
                unimplemented!()
            }

            /// Finds the elements an instruction describes, without acting. None is an answer: an
            /// empty `Vec`.
            pub fn observe(&self, instruction: &str) -> Observe {
                let _ = instruction;
                unimplemented!()
            }

            /// Reads typed data in the schema's type. A required field not on the page fails with
            /// `extraction_incomplete`.
            pub fn extract<T: DeserializeOwned + Send + 'static>(&self, instruction: &str, schema: Schema<T>) -> Extract<T> {
                let _ = (instruction, schema);
                unimplemented!()
            }

            /// Playwright's accessibility snapshot. Secret values are masked and cookies never
            /// appear, in both modes.
            pub fn aria_snapshot(&self) -> AriaSnapshot {
                unimplemented!()
            }
        }
    };
}

ai_verbs!(Page);
ai_verbs!(Locator);

impl Page {
    /// The element an AI snapshot gave this reference, such as `e12`. References last as long as
    /// their element, whichever snapshot showed them; one whose element is gone fails with
    /// `target_not_found` at once.
    pub fn get_by_ref(&self, reference: &str) -> Locator {
        let _ = reference;
        unimplemented!()
    }
}

/// What became of the Run's Plan: the Run report's `plan` (ADR 0102).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanReport {
    /// The file the Run named, or `None` when the Plan lives only in its Run trace.
    pub file: Option<std::path::PathBuf>,
    /// The version the Run read, or `None` for a new Plan.
    pub version_used: Option<String>,
    /// What happened to it.
    pub status: PlanStatus,
    /// The instructions of the steps compiled or repaired in this Run.
    pub changed_steps: Vec<String>,
}

/// What happened to a Run's Plan.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanStatus {
    /// Nothing changed.
    Unchanged,
    /// Written, after a `verified` Run.
    Promoted,
    /// A new version a person must accept: from a Run that was not `verified`, or a Plan with
    /// Effects (ADR 0009).
    PendingReview,
    /// `plan.frozen` was set.
    Frozen,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

// Success checks.

/// What a text or URL check expects, as in Playwright's assertions: a `&str` matches the whole value
/// (`to_have_url`'s exactly, text after whitespace is normalized), [`Expected::regex`] a pattern run
/// in the engine. Unlike a locator's [`TextMatch`], a string is never a substring here.
#[derive(Debug, Clone)]
pub struct Expected {
    _private: (),
}

impl Expected {
    /// A regular expression, run in the engine.
    pub fn regex(pattern: &str) -> Self {
        let _ = pattern;
        unimplemented!()
    }
}

impl From<&str> for Expected {
    fn from(text: &str) -> Self {
        let _ = text;
        unimplemented!()
    }
}

/// What [`expect`] takes: a page, a locator or a response.
pub trait Expectable {
    /// The assertions offered on it.
    type Expectations;
}

impl Expectable for Page {
    type Expectations = PageExpectations;
}

impl Expectable for Locator {
    type Expectations = LocatorExpectations;
}

impl Expectable for Response {
    type Expectations = ResponseExpectations;
}

/// Runs a Success check: the engine evaluates the condition live, waiting until it holds or its
/// timeout (`timeout.wait`, 30 s) passes, records it in the Run trace, and the Run's Verdict reads its
/// latest evaluation (ADR 0029). Still false at the timeout: `success_check_failed`. To ask without
/// counting, use `is_visible()` or `count()`; to wait, ADR 0080's waits.
pub fn expect<T: Expectable>(target: &T) -> T::Expectations {
    let _ = target;
    unimplemented!()
}

/// Playwright's page assertions, each a Success check.
#[derive(Debug)]
#[must_use = "an expectation does nothing until one of its checks is awaited"]
pub struct PageExpectations {
    _private: (),
}

impl PageExpectations {
    /// Negates the next check.
    pub fn not(self) -> Self {
        unimplemented!()
    }

    /// The page's URL: a string must equal it, as in Playwright.
    pub fn to_have_url(self, url: impl Into<Expected>) -> Call<()> {
        let _ = url.into();
        unimplemented!()
    }

    /// The page's title matches.
    pub fn to_have_title(self, title: impl Into<Expected>) -> Call<()> {
        let _ = title.into();
        unimplemented!()
    }

    /// The page's aria snapshot matches.
    pub fn to_match_aria_snapshot(self, expected: &str) -> Call<()> {
        let _ = expected;
        unimplemented!()
    }
}

/// Playwright's locator assertions, all but `to_have_screenshot`, each a Success check.
#[derive(Debug)]
#[must_use = "an expectation does nothing until one of its checks is awaited"]
pub struct LocatorExpectations {
    _private: (),
}

macro_rules! no_arg_checks {
    ($($name:ident: $doc:literal),* $(,)?) => {
        $(
            #[doc = $doc]
            pub fn $name(self) -> Call<()> {
                unimplemented!()
            }
        )*
    };
}

impl LocatorExpectations {
    /// Negates the next check.
    pub fn not(self) -> Self {
        unimplemented!()
    }

    no_arg_checks! {
        to_be_attached: "Attached to the page.",
        to_be_checked: "Checked.",
        to_be_disabled: "Disabled.",
        to_be_editable: "Editable.",
        to_be_empty: "Empty.",
        to_be_enabled: "Enabled.",
        to_be_focused: "Focused.",
        to_be_hidden: "Hidden.",
        to_be_in_viewport: "In the viewport.",
        to_be_visible: "Visible.",
    }

    /// The text contains each expected part.
    pub fn to_contain_text(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// The class list contains these classes.
    pub fn to_contain_class(self, expected: &str) -> Call<()> {
        let _ = expected;
        unimplemented!()
    }

    /// The accessible description matches.
    pub fn to_have_accessible_description(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// The accessible error message matches.
    pub fn to_have_accessible_error_message(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// The accessible name matches.
    pub fn to_have_accessible_name(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// The attribute is present, with this value when given.
    pub fn to_have_attribute(self, name: &str, value: Option<Expected>) -> Call<()> {
        let _ = (name, value);
        unimplemented!()
    }

    /// The class attribute matches.
    pub fn to_have_class(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// This many elements match.
    pub fn to_have_count(self, count: Count) -> Call<()> {
        let _ = count;
        unimplemented!()
    }

    /// The computed style matches.
    pub fn to_have_css(self, name: &str, value: impl Into<Expected>) -> Call<()> {
        let _ = (name, value.into());
        unimplemented!()
    }

    /// The id matches.
    pub fn to_have_id(self, id: impl Into<Expected>) -> Call<()> {
        let _ = id.into();
        unimplemented!()
    }

    /// The JavaScript property equals this JSON value.
    pub fn to_have_js_property(self, name: &str, value: serde_json::Value) -> Call<()> {
        let _ = (name, value);
        unimplemented!()
    }

    /// The ARIA role matches.
    pub fn to_have_role(self, role: crate::page::AriaRole) -> Call<()> {
        let _ = role;
        unimplemented!()
    }

    /// The text matches.
    pub fn to_have_text(self, expected: impl Into<Expected>) -> Call<()> {
        let _ = expected.into();
        unimplemented!()
    }

    /// The input value matches.
    pub fn to_have_value(self, value: impl Into<Expected>) -> Call<()> {
        let _ = value.into();
        unimplemented!()
    }

    /// The selected values match.
    pub fn to_have_values(self, values: Vec<Expected>) -> Call<()> {
        let _ = values;
        unimplemented!()
    }

    /// The aria snapshot matches.
    pub fn to_match_aria_snapshot(self, expected: &str) -> Call<()> {
        let _ = expected;
        unimplemented!()
    }
}

/// A response assertion, a Success check.
#[derive(Debug)]
#[must_use = "an expectation does nothing until one of its checks is awaited"]
pub struct ResponseExpectations {
    _private: (),
}

impl ResponseExpectations {
    /// Negates the next check.
    pub fn not(self) -> Self {
        unimplemented!()
    }

    /// A status from 200 to 299.
    pub fn to_be_ok(self) -> Call<()> {
        unimplemented!()
    }
}

// Details of the AI verbs' Error codes, in `goodfetch-protocol`.

/// Details of `target_not_found`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TargetNotFound {
    /// The instruction `act` found nothing for.
    pub instruction: Option<String>,
    /// The element reference whose element is gone.
    pub reference: Option<String>,
}

/// Why a drifted step was not repaired.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WhyNoRepair {
    /// `plan.frozen` was set.
    Frozen,
    /// No model is configured.
    NoModel,
    /// The call's one Repair drifted too.
    RepairDrifted,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Details of `plan_drift`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanDrift {
    /// The step's instruction.
    pub instruction: String,
    /// What drifted.
    pub drift: Drift,
    /// Why no Repair was made.
    pub why_no_repair: WhyNoRepair,
}

/// A required field not found on the page.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MissingField {
    /// Where in the value.
    pub path: Vec<crate::data::PathSegment>,
    /// `absent`, or `no_match` when the compiled locator matched nothing.
    pub reason: String,
}

/// Details of `extraction_incomplete`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExtractionIncomplete {
    /// The fields that were found, never typed as complete.
    pub found: serde_json::Value,
    /// Each missing field and why.
    pub missing: Vec<MissingField>,
}

/// Details of `extraction_invalid`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExtractionInvalid {
    /// How many answers the model gave.
    pub attempts: u32,
    /// The model that gave them.
    pub model: ModelReference,
    /// The strategy of the last attempt (ADR 0101).
    pub strategy: StructuredOutputStrategy,
    /// The last attempt's problems, per field.
    pub issues: Vec<crate::data::SchemaIssue>,
}

/// Details of `success_check_failed`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SuccessCheckFailed {
    /// The Success check in Rust: `expect(&page).to_have_url(Expected::regex("^https://quotes\\.toscrape\\.com/author/"))`.
    pub check: String,
    /// What it expected.
    pub expected: String,
    /// The last value seen, or `None` when no element matched.
    pub observed: Option<String>,
    /// The timeout that passed.
    pub timeout: Duration,
}

/// Details of `variables_mismatch`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VariablesMismatch {
    /// Placeholders with no value.
    pub missing: Vec<String>,
    /// Values with no placeholder.
    pub unused: Vec<String>,
    /// Names both a variable and a bound Secret placeholder.
    pub conflicting: Vec<String>,
}

/// Details of `plan_step_missing`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanStepMissing {
    /// The call's instruction.
    pub instruction: String,
    /// The Plan file, or `None` when the Run named none.
    pub file: Option<std::path::PathBuf>,
}

/// Details of `plan_invalid`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanInvalid {
    /// The Plan file.
    pub file: std::path::PathBuf,
    /// What is wrong with it.
    pub problem: String,
}

/// Details of `model_not_configured`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ModelNotConfigured {
    /// The call that needed a model.
    pub instruction: String,
}

/// Details of `effect_uncertain`: an AI action's instruction or a declared Effect's key
/// (ADR 0106). Never retried: a person reconciles it (ADR 0029).
pub type EffectUncertain = crate::effects::UncertainEffect;

/// Details of `effect_not_confirmed`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectNotConfirmed {
    /// The step's instruction.
    pub instruction: String,
    /// Its ADR 0014 category.
    pub category: String,
    /// `person` when declined, `timeout` when the Human pause expired.
    pub by: String,
}
