//! Fault plans (ADR 0060): the explicit list of faults and Client
//! operations one Simulation or fault test applies at named simulated
//! moments, its file format, and the glue that generates plans from a
//! proptest-state-machine reference machine, shrinks a failing one and
//! writes it to the test's folder with the command that replays it. A
//! committed plan sits in `fixtures/fault_plans/` beside its scenario, with
//! its `.source.toml`, and still replays after the code changes, where a
//! bare seed replays only on its own commit.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads and writes a plan outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use proptest::collection::SizeRange;
use proptest::test_runner::{Config, TestCaseError, TestError, TestRunner};
use proptest_state_machine::ReferenceStateMachine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::env::TestEnv;
use crate::runtime::proptest_config_for;

/// The file a failing generator writes its shrunk plan to, in the test's
/// folder.
pub const FAULT_PLAN_FILE: &str = "fault-plan.json";

/// The `.source.toml` written beside it, to commit with it (`TESTING.md`,
/// Fixtures and data files).
pub const FAULT_PLAN_SOURCE_FILE: &str = "fault-plan.source.toml";

/// A Fault plan: the seed and generator version it came from, and its steps
/// in the order they apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultPlan {
    /// The test seed the plan was generated from.
    pub seed: u64,
    /// The version of the scenario's generator, raised when its reference
    /// machine changes what it generates.
    pub generator_version: u32,
    /// The steps, in the order they apply.
    pub steps: Vec<FaultPlanStep>,
}

/// One step of a Fault plan: an action on one host at a simulated moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultPlanStep {
    /// When it applies, in simulated milliseconds from the scenario's start.
    pub at: u64,
    /// The simulated host it acts on.
    pub host: String,
    /// What it does.
    pub action: FaultPlanAction,
}

/// A fault, or a Client operation, that a step applies to its host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultPlanAction {
    /// The host's process crashes.
    Crash,
    /// The crashed host starts again.
    Restart,
    /// The link between the host and `peer` stops carrying messages.
    Partition {
        /// The host on the link's other end.
        peer: String,
    },
    /// The link between the host and `peer` carries messages again.
    Heal {
        /// The host on the link's other end.
        peer: String,
    },
    /// The link between the host and `peer` delays each message.
    Latency {
        /// The host on the link's other end.
        peer: String,
        /// The delay, in simulated milliseconds.
        milliseconds: u64,
    },
    /// The messages in flight from the host to `peer` are lost.
    Drop {
        /// The host they were sent to.
        peer: String,
    },
    /// The host's wall clock jumps, forwards or backwards; its monotonic
    /// clock does not.
    JumpWall {
        /// The jump, in milliseconds.
        milliseconds: i64,
    },
    /// A Client operation the host's Client sends, as the scenario reads
    /// it.
    Operation(serde_json::Value),
}

/// Why a Fault plan file cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum FaultPlanError {
    /// The file cannot be read.
    #[error("the Fault plan {} cannot be read: {error}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// Why.
        error: io::Error,
    },
    /// The file is not a Fault plan: a field or an action it does not know,
    /// or one it needs missing.
    #[error("the Fault plan {} is not one: {error}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// Why.
        error: serde_json::Error,
    },
}

impl FaultPlan {
    /// The plan in this file. A field or an action the format does not
    /// know is refused.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, or is not a Fault plan.
    pub fn read(path: &Path) -> Result<Self, FaultPlanError> {
        let text = std::fs::read_to_string(path).map_err(|error| FaultPlanError::Unreadable {
            path: path.to_owned(),
            error,
        })?;
        serde_json::from_str(&text).map_err(|error| FaultPlanError::Invalid {
            path: path.to_owned(),
            error,
        })
    }

    /// Writes the plan as indented JSON, one field a line, to be read and
    /// committed.
    ///
    /// # Errors
    ///
    /// If the file cannot be written.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(path, text)
    }
}

/// Generates a scenario's Fault plans from its reference machine, whose
/// transitions are the plan's steps, and on a failure shrinks the plan and
/// writes it to the test's folder.
pub struct FaultPlanGenerator {
    scenario: String,
    generator_version: u32,
    seed: u64,
    folder: PathBuf,
    config: Config,
}

impl FaultPlanGenerator {
    /// The generator of `scenario`'s plans at `generator_version`, from the
    /// test's seed, with the case count of the running suite.
    pub fn new(env: &TestEnv, scenario: &str, generator_version: u32) -> Self {
        Self {
            scenario: scenario.to_owned(),
            generator_version,
            seed: env.seed(),
            folder: env.folder().to_owned(),
            config: Config {
                // The written Fault plan is the regression to commit.
                failure_persistence: None,
                ..proptest_config_for(env.suite(), env.seed())
            },
        }
    }

    /// The same generator with another proptest configuration, for another
    /// case count or proptest's regression file beside the scenario.
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Runs `run` on each generated plan of `steps` steps. The reference
    /// machine's initial state must be fixed, since a plan holds only its
    /// steps. On a failure, proptest shrinks the plan to a smaller one that
    /// still fails, which is written to [`FAULT_PLAN_FILE`] in the test's
    /// folder with its [`FAULT_PLAN_SOURCE_FILE`].
    ///
    /// # Errors
    ///
    /// The shrunk plan with its file and replay command, or why proptest
    /// stopped generating.
    pub fn check<M, E>(
        &self,
        steps: impl Into<SizeRange>,
        run: impl Fn(&FaultPlan) -> Result<(), E>,
    ) -> Result<(), FaultPlanFailure>
    where
        M: ReferenceStateMachine<Transition = FaultPlanStep>,
        M::State: Clone + fmt::Debug,
        E: fmt::Display,
    {
        let mut runner = TestRunner::new(self.config.clone());
        let outcome = runner.run(&M::sequential_strategy(steps), |(_, steps, seen)| {
            // The whole plan runs, so every step counts as seen; otherwise
            // proptest-state-machine drops the unseen ones before shrinking.
            if let Some(seen) = seen {
                seen.store(steps.len(), Ordering::SeqCst);
            }
            run(&self.plan(steps)).map_err(|error| TestCaseError::fail(error.to_string()))
        });
        match outcome {
            Ok(()) => Ok(()),
            Err(TestError::Abort(reason)) => Err(FaultPlanFailure::Aborted {
                scenario: self.scenario.clone(),
                reason: reason.to_string(),
            }),
            Err(TestError::Fail(reason, (_, steps, _))) => {
                Err(self.shrunk(self.plan(steps), reason.to_string()))
            }
        }
    }

    fn plan(&self, steps: Vec<FaultPlanStep>) -> FaultPlan {
        FaultPlan {
            seed: self.seed,
            generator_version: self.generator_version,
            steps,
        }
    }

    fn shrunk(&self, plan: FaultPlan, reason: String) -> FaultPlanFailure {
        let file = self.folder.join(FAULT_PLAN_FILE);
        let command = replay_command(&self.scenario, &file);
        let written = plan.write(&file).and_then(|()| {
            std::fs::write(
                self.folder.join(FAULT_PLAN_SOURCE_FILE),
                self.source(&command)?,
            )
        });
        let plan = Box::new(plan);
        match written {
            Ok(()) => FaultPlanFailure::Shrunk {
                scenario: self.scenario.clone(),
                plan,
                reason,
                file,
                command,
            },
            Err(error) => FaultPlanFailure::Unwritten {
                scenario: self.scenario.clone(),
                plan,
                reason,
                file,
                error,
            },
        }
    }

    /// The `.source.toml` of a written plan: its generator, version, seed
    /// and replay command (`TESTING.md`, Fixtures and data files). A JSON
    /// string is a TOML basic string, with the same escapes.
    fn source(&self, command: &str) -> io::Result<String> {
        let generator = serde_json::to_string(&format!(
            "proptest-state-machine, scenario {}",
            self.scenario
        ))?;
        let command = serde_json::to_string(command)?;
        Ok(format!(
            "generator = {generator}\nversion = {}\nseed = {}\ncommand = {command}\n",
            self.generator_version, self.seed
        ))
    }
}

/// `just simulate <scenario> --plan <file>`, the file quoted for a POSIX
/// shell, since a test's folder holds a space.
fn replay_command(scenario: &str, file: &Path) -> String {
    let quoted = file.display().to_string().replace('\'', r"'\''");
    format!("just simulate {scenario} --plan '{quoted}'")
}

/// A scenario that failed on a generated Fault plan.
#[derive(Debug, thiserror::Error)]
pub enum FaultPlanFailure {
    /// The shrunk plan that still fails, written with its replay command.
    #[error(
        "the scenario {scenario} fails on its shrunk Fault plan, written to {}: {reason}\nreplay it with: {command}",
        file.display()
    )]
    Shrunk {
        /// The scenario.
        scenario: String,
        /// The shrunk plan.
        plan: Box<FaultPlan>,
        /// Why the scenario failed on it.
        reason: String,
        /// The file it was written to.
        file: PathBuf,
        /// The command that replays it.
        command: String,
    },
    /// The shrunk plan, which could not be written.
    #[error(
        "the scenario {scenario} fails on its shrunk Fault plan, which cannot be written to {}: {error}; the scenario failed with: {reason}; the plan: {plan:?}",
        file.display()
    )]
    Unwritten {
        /// The scenario.
        scenario: String,
        /// The shrunk plan.
        plan: Box<FaultPlan>,
        /// Why the scenario failed on it.
        reason: String,
        /// The file it could not be written to.
        file: PathBuf,
        /// Why.
        error: io::Error,
    },
    /// proptest stopped generating plans, its reference machine's
    /// preconditions refusing too many.
    #[error("proptest stopped generating the scenario {scenario}'s Fault plans: {reason}")]
    Aborted {
        /// The scenario.
        scenario: String,
        /// Why.
        reason: String,
    },
}

#[cfg(test)]
mod tests;
