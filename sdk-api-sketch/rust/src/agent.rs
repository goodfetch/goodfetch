//! The Agent in Collie's SDK API (ADR 0100, decided in #474).
//!
//! - [`Page::agent`] runs ADR 0013's built-in Agent on one task, to its end: a thin, journaled loop
//!   over the Operation catalog's page operations, the program's Agent tools and a `note` action.
//!   It starts on the page, may use the tabs it opens, and carries nothing into the next call (G2).
//!   The whole task is one gesture of the page (ADR 0085); it gives up the page's turn while an
//!   Agent tool or its Success check runs.
//! - The program states the Success check with [`crate::expect`] (ADR 0091) in
//!   [`AgentCall::success_check`]. The engine runs it each time the model reports done; a failure
//!   goes back to the model, and the Verdict reads its latest evaluation (ADR 0029).
//! - The call resolves to an [`AgentResult`]: ADR 0079's three separate fields, the steps, their
//!   [`ModelUsage`] (ADR 0101) and the page the task ended on. A step cap, a stall rule, the task's
//!   timeout, a Limit and an abort are values: [`Ended::Stopped`] with its reason, and a Grounded
//!   `partial`.
//! - [`AgentCall::start`] gives an [`AgentTask`] whose [`AgentTask::steps`] is a lossless
//!   [`EventStream`] that starts at the task's first step (ADR 0081).
//! - With the Run's Plan ([`crate::Start::plan`], ADR 0091), the task is one Plan entry, replayed
//!   whole with no model call (before each Effect, every target reachable without it is checked).
//!   On Drift, or a failed Success check after the replay, the loop takes over live from there: no
//!   Repair (amends ADR 0009).

use std::fmt;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::ai::Drift;
use crate::concurrency::EventStream;
use crate::data::Schema;
use crate::lifecycle::Page;
use crate::models::{ModelReference, ModelUsage};
use crate::page::Locator;
use crate::results::{Ended, Error, PartialOutput, Reported, Verdict};
use crate::time::CallFuture;

// Steps.

/// Where a step came from. A newer engine's value is kept as received (G3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentStepOrigin {
    /// Decided by the model now.
    Model,
    /// Replayed from the Plan, no model call.
    Plan,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// How one action of a step ended. A failure goes back to the model; it never fails the call.
#[derive(Debug)]
#[non_exhaustive]
pub enum AgentActionOutcome {
    /// It was done.
    Done,
    /// It failed, and the model was told.
    Failed(Error),
    /// The Effect journal already held this Effect, so nothing was done again (ADR 0010).
    Journal,
    /// An outcome this SDK does not know, as received.
    Unknown(String),
}

/// One action a step took.
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentAction {
    /// The catalog operation, such as `locator.click` or `page.goto`; `tool:<name>` for an Agent
    /// tool; `note` for the built-in action that notes the answer so far.
    pub operation: String,
    /// Its arguments, `%name%` placeholders where a variable or a secret was typed, never the value.
    pub arguments: Vec<serde_json::Value>,
    /// The target of a page action, an ordinary locator resolved again at each use.
    pub target: Option<Locator>,
    /// A page action in Rust, as the engine renders it (ADR 0091), so a person can turn a task into
    /// plain page API calls.
    pub code: Option<String>,
    /// How it ended.
    pub outcome: AgentActionOutcome,
}

/// One step: one model decision and the actions it took, or one step replayed from the Plan.
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentStep {
    /// From 1.
    pub index: u32,
    /// Model or Plan.
    pub origin: AgentStepOrigin,
    /// On the first live step after a replay drifted, or after its Success check failed: what the
    /// Plan expected and what the page showed (ADR 0091). The replay ended there and the loop took
    /// over.
    pub drift: Option<Drift>,
    /// The model's own words for this step: a signal, never proof. Empty for a replayed step.
    pub message: String,
    /// What it did.
    pub actions: Vec<AgentAction>,
    /// The URL of the page the task is on after the step.
    pub url: String,
    /// Its actions failed, or repeated earlier ones on targets whose page had not changed since.
    pub stalled: bool,
    /// This step's model calls and their price: none for a replayed step.
    pub usage: ModelUsage,
}

// The result and the task.

/// What a task resolves to: ADR 0079's three separate fields, the output and what the task did.
/// `output` is a valid `T` whose values Grounding found on the pages the task observed or in its
/// Agent tools' results; a value it could not find leaves the Verdict at needs-review, naming the
/// field.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the Verdict says whether the output is proven"]
pub struct AgentResult<T> {
    /// How the task ended: finished, or stopped with its reason.
    pub ended: Ended,
    /// Whether the Success check and the other checks prove it.
    pub verdict: Verdict,
    /// What the model claimed; `None` when the task stopped before it claimed anything.
    pub reported: Option<Reported>,
    /// Present only once the task finished with a complete `T`.
    pub output: Option<T>,
    /// What was recorded before an early end, set only when `output` is not.
    pub partial: Option<PartialOutput>,
    /// Every step, in order.
    pub steps: Vec<AgentStep>,
    /// The whole task's model calls and their price (ADR 0101).
    pub usage: ModelUsage,
    /// The page the task ended on: its own, or a tab it opened, which the program now holds. The
    /// other tabs it opened are closed as `page.close()` closes them: on a Borrowed browser,
    /// released.
    pub page: Page,
}

impl<T> AgentResult<T> {
    /// The result as the engine sent it, with the fields this SDK does not know (G3).
    pub fn raw(&self) -> &serde_json::Value {
        unimplemented!()
    }

    /// Returns the output when the Verdict is `Verified`; otherwise the error `not_verified`, with
    /// every reason.
    pub fn require_verified(self) -> Result<T, Error> {
        unimplemented!()
    }
}

/// A running task, from [`AgentCall::start`]. Dropping it before it ends cancels it (ADR 0078's
/// `Drop` rule).
#[must_use = "dropping a task cancels it"]
pub struct AgentTask<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for AgentTask<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentTask(..)")
    }
}

impl<T> AgentTask<T> {
    /// The task's steps, lossless, from step 1 whenever it is called, ending when the task ends and
    /// yielding the task's error when it failed. Each call is a reader of its own; an unread open
    /// stream slows the task (ADR 0081).
    pub fn steps(&self) -> EventStream<AgentStep> {
        unimplemented!()
    }

    /// Waits for the task's end. An early end is a value ([`Ended::Stopped`]); a failure is an
    /// [`Error`].
    pub async fn result(self) -> Result<AgentResult<T>, Error> {
        unimplemented!()
    }
}

// Agent tools.

/// ADR 0014's categories of Effects that ask a person for confirmation unless the run allows them;
/// area 4's spec spells them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfirmationCategory {
    /// A payment.
    Payment,
    /// Creating an account.
    AccountCreation,
    /// Accepting terms.
    AcceptingTerms,
    /// Sending a message.
    SendingMessages,
    /// Deleting something.
    Deletion,
    /// A category this SDK does not know.
    Unknown(String),
}

/// The error an Agent tool returns: any error of the program. A [`crate::Error`] boxed in it ends the
/// task with its own code; any other ends it with `agent_tool_failed`, which keeps it as its
/// `source()`.
pub type ToolError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A function of the program that the Agent may call as an action: an Agent tool.
pub struct AgentTool {
    _private: (),
}

impl fmt::Debug for AgentTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentTool(..)")
    }
}

impl AgentTool {
    /// A tool whose input is `I`, a type deriving `JsonSchema` and `Deserialize` (ADR 0082), checked
    /// before `run` is called with it and the task's current page. Its JSON is what the model reads,
    /// bounded by `observation.max_size`, and a source Grounding accepts; the task gives up the
    /// page's turn while it runs. The name must be unique within a task and no built-in action's,
    /// or the task fails with `agent_tool_invalid`. It is an Effect unless
    /// [`AgentTool::read_only`]: journaled and never redone (ADR 0010).
    pub fn new<I, F, Fut>(name: &str, description: &str, run: F) -> Self
    where
        I: JsonSchema + DeserializeOwned + Send + 'static,
        F: Fn(I, Page) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value, ToolError>> + Send + 'static,
    {
        let _ = (name, description, run);
        unimplemented!()
    }

    /// The tool changes nothing: no Effect.
    pub fn read_only(self) -> Self {
        unimplemented!()
    }

    /// Asks a person for confirmation before every call (ADR 0014), since the run allowances of
    /// these categories name domains and a tool has none.
    pub fn category(self, category: ConfirmationCategory) -> Self {
        let _ = category;
        unimplemented!()
    }
}

// The call.

/// A task for the Agent, not sent until awaited or started. Awaited, it resolves to the result;
/// [`AgentCall::start`] gives the running task, whose steps can be read as they happen.
#[must_use = "a task does not start until it is awaited or started"]
pub struct AgentCall<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for AgentCall<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentCall(..)")
    }
}

impl<T> AgentCall<T> {
    /// The output's schema (ADR 0082); without one, the output is `()` and the model's message is
    /// in `reported`.
    pub fn schema<U: DeserializeOwned + Send + 'static>(self, schema: Schema<U>) -> AgentCall<U> {
        let _ = schema;
        unimplemented!()
    }

    /// The Success check, stated with [`crate::expect`], run each time the model reports done: each
    /// `expect` in it is a Success check the engine evaluates live. Without one, the Verdict is
    /// needs-review (ADR 0013). An error other than `success_check_failed` ends the task with its own
    /// code.
    pub fn success_check<F, Fut>(self, check: F) -> Self
    where
        F: Fn(Page) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let _ = check;
        unimplemented!()
    }

    /// `agent.max_steps` for this task: 20 by default. Reaching it stops only the task.
    pub fn max_steps(self, steps: u32) -> Self {
        let _ = steps;
        unimplemented!()
    }

    /// `agent.domains` for this task: by default the registrable domain of the page it starts on.
    /// An entry matches its host and their subdomains; `"*"` allows wherever the run's domain
    /// policy does. It bounds the task's navigations, never the subresources a page loads.
    pub fn domains<I, S>(self, domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let _ = domains.into_iter().map(Into::into).collect::<Vec<String>>();
        unimplemented!()
    }

    /// Adds an Agent tool.
    pub fn tool(self, tool: AgentTool) -> Self {
        let _ = tool;
        unimplemented!()
    }

    /// A plain value for the task's `%name%` placeholder, as `act` takes them (ADR 0091). A
    /// placeholder no variable fills is a Secret placeholder bound by `secrets.<name>` (ADR 0092).
    pub fn variable(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }

    /// The model for this task alone: `ai.model` at the run layer (ADR 0101).
    pub fn model(self, model: impl Into<ModelReference>) -> Self {
        let _ = model.into();
        unimplemented!()
    }

    /// `timeout.agent` for this task: 10 minutes by default. Reaching it cancels the step in flight
    /// and stops the task with [`crate::StopReason::Timeout`]; it is no error. An enclosing call's
    /// timeout fails it with `deadline_exceeded` naming that call; `timeout.ai` does not apply.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Starts the task and returns it running, to read its steps as they happen.
    pub async fn start(self) -> Result<AgentTask<T>, Error> {
        unimplemented!()
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for AgentCall<T> {
    type Output = Result<AgentResult<T>, Error>;
    type IntoFuture = CallFuture<AgentResult<T>>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

impl Page {
    /// Runs one task, to its end, with the built-in Agent, starting on this page.
    pub fn agent(&self, task: &str) -> AgentCall<()> {
        let _ = task;
        unimplemented!()
    }
}

// The Agent's Error details (ADR 0079's naming rule).

/// Details of `agent_tool_failed`: a tool returned an error that is not a [`crate::Error`], and the
/// task ended. The tool's error is the error's `source()`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentToolFailed {
    /// The tool's name.
    pub tool: String,
    /// The tool's error, as its `Display` wrote it.
    pub message: String,
    /// The tool was not read-only, so its Effect is left uncertain for a person to reconcile
    /// (ADR 0029).
    pub effect_uncertain: bool,
}

/// Details of `agent_tool_invalid`: refused before the task starts.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentToolInvalid {
    /// The tool's name.
    pub tool: String,
    /// `duplicate` or `built_in_name`.
    pub problem: String,
}

/// Details of `success_check_threw`: the Success check returned an error other than
/// `success_check_failed` that is not a Goodfetch code, kept as the error's `source()`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SuccessCheckThrew {
    /// The error, as its `Display` wrote it.
    pub message: String,
}

/// On an error `page.agent` fails with after the task took steps (ADR 0100).
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentTaskProgress {
    /// The steps taken.
    pub steps: Vec<AgentStep>,
    /// The partial answer as JSON: only values valid against their part of the schema and Grounded.
    pub partial: Option<serde_json::Value>,
}
