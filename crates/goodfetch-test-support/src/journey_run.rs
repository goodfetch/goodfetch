//! The Rust Journey helper (ADRs 0063, 0066, 0069 and 0094): `JourneyRun`
//! runs one variant of a Journey, a `cli`, `foxhound-cli`, `sdk-rust` or
//! `http-listener` test, on the executable its caller passes. It reads the
//! Journey file as plain TOML values (its ID, Product, surfaces, Proving
//! ground seed and tasks, steps and checks), starts the variant's home and,
//! for a Journey with a `[proving_ground]` table, its Proving ground on the
//! Journey's own seed, which `GOODFETCH_TEST_SEED` never changes. It records
//! each step's and check's outcome and each named task's oracle answer in
//! the file's order, and `finish` hands what it recorded, in the result
//! document's field names, to the constructor the test passes
//! (`goodfetch-protocol`'s), then writes the document it returns. It knows
//! no Goodfetch type: the constructor derives the outcome, and every reader
//! compares the document with its Journey.
//!
//! The rules it records by:
//! - steps run in the file's order and by its wording; a `manual` step is
//!   recorded `manual` without running, whenever the run passes it;
//! - a step called out of order, twice, or with a wording the file does not
//!   hold is recorded `failed` without running;
//! - after the first failure, nothing more runs: every later step and check
//!   is `not_reached`, and no result goes to the oracle;
//! - at `finish`, each step never called is recorded, after those called, as
//!   `not_reached` (`manual` for a manual one), so a run that skips a step
//!   writes a document every reader refuses;
//! - a check the file scopes to other variants' surfaces is not the
//!   variant's: it is neither run nor written;
//! - checks and tasks are written in the file's order, a check never called
//!   `not_reached` and a task never judged with no answer;
//! - each check naming a task is judged on its own submission, so a task two
//!   checks name is written once per check, with that check's answer.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the Journey file, runs the executable and writes the document outside any async worker (ADR 0062)"
)]

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::env::{TestEnv, command};
use crate::proving_ground::{ProvingGround, ProvingGroundError};
use crate::proving_ground_format::ROUTES;
use crate::variables::{ENGINE_VARIABLE, RESULT_DOCUMENTS_VARIABLE, Variables};

/// The runtime a Rust variant records.
const RUNTIME: &str = "rust";

/// The surface of a Collie Journey's `cli` variant run on Foxhound's
/// executable (ADR 0094).
const FOXHOUND_CLI: &str = "foxhound-cli";

/// The executable a Journey variant drives: the one `GOODFETCH_TEST_ENGINE`
/// names when it is set, for a skew run of `just journeys --engine <path>`
/// (ADR 0066), else `built`, the commit's own: `CARGO_BIN_EXE_goodfetch-collie`
/// or `-foxhound` in a Product's executable crate, or the test tool
/// manifest's entry. Every `cli` variant takes its executable through it,
/// and [`JourneyRun`] passes the executable it is given through it.
pub fn journey_engine(built: impl AsRef<Path>) -> PathBuf {
    Variables::from_process()
        .get(ENGINE_VARIABLE)
        .map_or_else(|| built.as_ref().to_owned(), PathBuf::from)
}

/// The catalog's folder, `journeys/` at the repository's root.
fn catalog() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../journeys")
}

/// What the helper reads of a Journey file; the catalog check holds the
/// rest to the Journey type.
#[derive(serde::Deserialize)]
struct JourneyFile {
    id: String,
    product: String,
    #[serde(default)]
    proving_ground: Option<ProvingGroundTable>,
    surfaces: BTreeMap<String, toml::Value>,
    steps: Vec<StepEntry>,
    #[serde(default)]
    expect: Vec<CheckEntry>,
}

#[derive(serde::Deserialize)]
struct ProvingGroundTable {
    seed: u64,
    #[serde(default)]
    tasks: Vec<TaskEntry>,
}

#[derive(serde::Deserialize)]
struct TaskEntry {
    id: String,
}

#[derive(serde::Deserialize)]
struct StepEntry {
    text: String,
    #[serde(default)]
    manual: Option<String>,
}

#[derive(serde::Deserialize)]
struct CheckEntry {
    text: String,
    #[serde(default)]
    task: Option<String>,
    /// The surfaces of the variants the check belongs to; every variant's
    /// when empty.
    #[serde(default)]
    surfaces: Vec<String>,
}

impl CheckEntry {
    /// Whether the check belongs to the variant on `surface`, a
    /// `foxhound-cli` variant taking the `cli` variant's checks.
    fn belongs_to(&self, surface: &str) -> bool {
        self.surfaces.is_empty()
            || self
                .surfaces
                .iter()
                .any(|listed| listed == surface || (surface == FOXHOUND_CLI && listed == "cli"))
    }
}

impl JourneyFile {
    fn tasks(&self) -> impl Iterator<Item = &str> {
        self.proving_ground
            .iter()
            .flat_map(|table| table.tasks.iter().map(|task| task.id.as_str()))
    }

    /// Whether the Journey has a variant on `surface`: one of its surfaces,
    /// or `foxhound-cli` for a Collie Journey that lists `cli`.
    fn lists(&self, surface: &str) -> bool {
        self.surfaces.contains_key(surface)
            || (surface == FOXHOUND_CLI
                && self.product == "collie"
                && self.surfaces.contains_key("cli"))
    }
}

/// One recorded step or check, in the result document's words.
struct Recorded {
    text: String,
    task: Option<String>,
    outcome: &'static str,
}

/// One variant run of a Journey, from its start to the document it writes.
/// A test calls `step` for each step of the file, in order, then `check`
/// and `expect_task` for its checks, then `finish`. Dropping it unfinished
/// fails the test with an [`UnfinishedJourneyRun`].
pub struct JourneyRun<C> {
    // Dropped first, so an unfinished run fails before its home goes.
    guard: Guard,
    journey: JourneyFile,
    surface: String,
    engine: PathBuf,
    product: String,
    version: String,
    surface_package: Option<(String, String)>,
    /// The steps recorded, in the order they were.
    steps: Vec<Recorded>,
    /// Which of the file's steps are recorded.
    taken: Vec<bool>,
    /// The file's checks, each once recorded.
    checks: Vec<Option<Recorded>>,
    /// Checks recorded that the file does not hold.
    extra_checks: Vec<Recorded>,
    /// The oracle's answer to each check judged, by the check's place in
    /// the file.
    answers: BTreeMap<usize, Value>,
    failures: Vec<String>,
    construct: C,
    agent: ureq::Agent,
    // Before the home, so it stops before the home's teardown runs.
    proving_ground: Option<ProvingGround>,
    env: TestEnv,
}

impl<C> JourneyRun<C> {
    /// Starts the variant of `journeys/<id>.toml` on `surface` (`cli`,
    /// `foxhound-cli`, `sdk-rust`, `http-listener`), driving `engine`, the
    /// executable of the Journey's Product: `CARGO_BIN_EXE_goodfetch-collie`
    /// or `-foxhound` in that Product's executable crate, the test tool
    /// manifest's entry in its Rust crate's `sdk-rust` variants and
    /// Foxhound's entry for a `foxhound-cli` variant, or any path for a skew
    /// run. `construct` turns what the run recorded into the document.
    ///
    /// # Errors
    ///
    /// As [`JourneyRun::start_file`], and [`JourneyRunError::OtherJourney`]
    /// when the file holds another ID.
    ///
    /// # Panics
    ///
    /// When the test's [`TestEnv`] cannot start.
    pub fn start(
        id: &str,
        surface: &str,
        engine: &Path,
        construct: C,
    ) -> Result<Self, JourneyRunError> {
        let run = Self::start_file(
            &catalog().join(format!("{id}.toml")),
            surface,
            engine,
            construct,
        )?;
        if run.journey.id == id {
            Ok(run)
        } else {
            Err(JourneyRunError::OtherJourney {
                asked: id.to_owned(),
                file: run.journey.id.clone(),
            })
        }
    }

    /// Starts the variant on `surface` of the Journey `file` holds, as
    /// [`JourneyRun::start`] does for a catalog file.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or parsed, does not list `surface`, the
    /// executable does not report its Product and version, or its Proving
    /// ground does not start.
    ///
    /// # Panics
    ///
    /// When the test's [`TestEnv`] cannot start.
    pub fn start_file(
        file: &Path,
        surface: &str,
        engine: &Path,
        construct: C,
    ) -> Result<Self, JourneyRunError> {
        let text = std::fs::read_to_string(file).map_err(|source| JourneyRunError::Unreadable {
            path: file.to_owned(),
            source,
        })?;
        let mut journey: JourneyFile =
            toml::from_str(&text).map_err(|source| JourneyRunError::Invalid {
                path: file.to_owned(),
                source: Box::new(source),
            })?;
        // The variant's own checks: those scoped to other variants' surfaces
        // are not run, and not recorded.
        journey.expect.retain(|check| check.belongs_to(surface));
        if !journey.lists(surface) {
            return Err(JourneyRunError::UnlistedSurface {
                journey: journey.id,
                surface: surface.to_owned(),
            });
        }
        let engine = &journey_engine(engine);
        let env = TestEnv::new();
        let (product, version) = reported_version(&env, engine)?;
        // The Journey's own seed, never the test's (ADR 0063).
        let proving_ground = journey
            .proving_ground
            .as_ref()
            .map(|table| ProvingGround::start(&env, table.seed))
            .transpose()?;
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            guard: Guard {
                variant: format!("{}@{surface}", journey.id),
                armed: true,
            },
            taken: vec![false; journey.steps.len()],
            checks: journey.expect.iter().map(|_| None).collect(),
            journey,
            surface: surface.to_owned(),
            engine: engine.to_owned(),
            product,
            version,
            surface_package: None,
            steps: Vec::new(),
            extra_checks: Vec::new(),
            answers: BTreeMap::new(),
            failures: Vec::new(),
            construct,
            agent,
            proving_ground,
            env,
        })
    }

    /// The variant's home and cleared environment.
    pub fn env(&self) -> &TestEnv {
        &self.env
    }

    /// The executable the variant drives.
    pub fn engine(&self) -> &Path {
        &self.engine
    }

    /// The executable's command line under the variant's environment: how a
    /// step pinned `on = "cli"` runs in any variant.
    pub fn command(&self) -> assert_cmd::Command {
        command(&self.env, &self.engine)
    }

    /// The Proving ground started on the Journey's seed, for a Journey with
    /// a `[proving_ground]` table.
    pub fn proving_ground(&self) -> Option<&ProvingGround> {
        self.proving_ground.as_ref()
    }

    /// Records the surface's package, for a surface that has one, such as
    /// an `sdk-rust` variant's crate and its version.
    pub fn surface_package(&mut self, name: &str, version: &str) {
        self.surface_package = Some((name.to_owned(), version.to_owned()));
    }

    /// Runs the step the file words `wording`, when it is the next one and
    /// nothing failed before it, and records its outcome: `failed` when
    /// `body` panics or returns an error. A step pinned `on` another surface
    /// drives that surface in its body, through that surface's own helper
    /// (the CLI through [`JourneyRun::command`]).
    pub fn step<R: BodyResult>(&mut self, wording: &str, body: impl FnOnce(&Self) -> R) {
        let found = self.untaken_step(wording);
        let manual_reached = found.is_some_and(|index| {
            (0..=index)
                .all(|before| self.taken[before] || self.journey.steps[before].manual.is_some())
        });
        self.take_manual_steps();
        if manual_reached {
            // A manual step the run reached: recorded `manual` just above.
            return;
        }
        let next = self.taken.iter().position(|taken| !taken);
        let misuse = match (found, next) {
            (Some(index), Some(next)) if index == next => None,
            (Some(index), _) => {
                self.taken[index] = true;
                Some(format!(
                    "is called before step {} `{}`",
                    next.map_or(0, |next| next + 1),
                    next.map_or("", |next| self.journey.steps[next].text.as_str())
                ))
            }
            (None, _) if self.journey.steps.iter().any(|step| step.text == wording) => {
                Some("is called a second time".to_owned())
            }
            (None, _) => Some(format!("is not a step of {}", self.journey.id)),
        };
        if let Some(misuse) = misuse {
            self.fail(format!("step `{wording}` {misuse}"));
            self.record_step(wording, "failed");
            return;
        }
        let index = found.expect("invariant: an in-order step was found");
        self.taken[index] = true;
        if self.failed() {
            self.record_step(wording, "not_reached");
        } else if let Some(failure) = self.run_body(body) {
            self.fail(format!("step {} `{wording}` failed: {failure}", index + 1));
            self.record_step(wording, "failed");
        } else {
            self.record_step(wording, "passed");
        }
    }

    /// Runs the check the file words `wording`, one that names no task, when
    /// nothing failed before it, and records its outcome: `failed` when
    /// `body` panics or returns an error. A check that names a task is
    /// recorded by [`JourneyRun::expect_task`].
    pub fn check<R: BodyResult>(&mut self, wording: &str, body: impl FnOnce(&Self) -> R) {
        let index = self
            .journey
            .expect
            .iter()
            .position(|check| check.text == wording && check.task.is_none());
        let Some(index) = index.filter(|index| self.checks[*index].is_none()) else {
            let misuse = if index.is_some() {
                "is called a second time".to_owned()
            } else {
                format!("is not a check of {} that names no task", self.journey.id)
            };
            self.fail(format!("check `{wording}` {misuse}"));
            self.extra_checks.push(Recorded {
                text: wording.to_owned(),
                task: None,
                outcome: "failed",
            });
            return;
        };
        let outcome = if self.failed() {
            "not_reached"
        } else if let Some(failure) = self.run_body(body) {
            self.fail(format!("check `{wording}` failed: {failure}"));
            "failed"
        } else {
            "passed"
        };
        self.checks[index] = Some(Recorded {
            text: wording.to_owned(),
            task: None,
            outcome,
        });
    }

    /// Submits the tool's result for the check the file words `check`, one
    /// that names a task, to its Proving ground's oracle for that task, when
    /// nothing failed before, and records the check as `passed` when the
    /// answer is `verified` and `failed` otherwise. Each check naming a task
    /// is judged on its own submission, so two checks naming one task (two
    /// Crawls of one site) take two answers. `submission` is the oracle's
    /// submission without its `task`: the tool's `status`, its `records`
    /// and any `files`. Returns the answer.
    pub fn expect_task(&mut self, check: &str, submission: &Value) -> Option<&Value> {
        let index = self
            .journey
            .expect
            .iter()
            .position(|entry| entry.text == check && entry.task.is_some());
        let Some(index) = index.filter(|index| self.checks[*index].is_none()) else {
            let misuse = if index.is_some() {
                "is judged a second time".to_owned()
            } else {
                format!("is not a check of {} that names a task", self.journey.id)
            };
            self.fail(format!("check `{check}` {misuse}"));
            self.extra_checks.push(Recorded {
                text: check.to_owned(),
                task: None,
                outcome: "failed",
            });
            return None;
        };
        if self.failed() {
            return None;
        }
        let task = self.journey.expect[index]
            .task
            .clone()
            .expect("invariant: the check found names a task");
        let answer = match self.judge(&task, submission) {
            Ok(answer) => answer,
            Err(failure) => {
                self.fail(format!("check `{check}`, task `{task}`: {failure}"));
                return None;
            }
        };
        let outcome = answer
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned();
        let verified = outcome == "verified";
        if !verified {
            self.fail(format!(
                "the oracle answers `{outcome}` for task `{task}` of check `{check}`"
            ));
        }
        self.checks[index] = Some(Recorded {
            text: check.to_owned(),
            task: Some(task),
            outcome: if verified { "passed" } else { "failed" },
        });
        self.answers.insert(index, answer);
        self.answers.get(&index)
    }

    /// Hands what the run recorded to the constructor and writes the
    /// document it returns as `<id>@<surface>.json`, into the test's
    /// folder, which lives as long as the returned value, and into the folder
    /// `GOODFETCH_TEST_RESULT_DOCUMENTS` names when it is set (`just test`
    /// sets it). The facts are a JSON object in the result document's field
    /// names, deserialized into the constructor's argument.
    ///
    /// # Errors
    ///
    /// When the constructor's argument does not accept the facts, its
    /// document does not serialize, or the file cannot be written.
    pub fn finish<F, D>(self) -> Result<Finished<D>, JourneyRunError>
    where
        C: FnOnce(F) -> D,
        F: DeserializeOwned,
        D: Serialize,
    {
        let mut run = self;
        run.take_manual_steps();
        for index in 0..run.journey.steps.len() {
            if !run.taken[index] {
                let outcome = if run.journey.steps[index].manual.is_some() {
                    "manual"
                } else {
                    "not_reached"
                };
                run.taken[index] = true;
                let text = run.journey.steps[index].text.clone();
                run.record_step(&text, outcome);
            }
        }
        let facts = run.facts();
        let Self {
            mut guard,
            construct,
            proving_ground,
            env,
            failures,
            ..
        } = run;
        guard.armed = false;
        drop(proving_ground);
        let variant = guard.variant.clone();
        let argument = serde_json::from_value(facts).map_err(JourneyRunError::Facts)?;
        let document = construct(argument);
        let value = serde_json::to_value(&document).map_err(JourneyRunError::Document)?;
        let file = format!("{variant}.json");
        let mut bytes = serde_json::to_vec_pretty(&value).map_err(JourneyRunError::Document)?;
        bytes.push(b'\n');
        // The test's own copy is the one `Finished` reads: in the suite's
        // folder, another test running the same variant may write after it.
        let path = env.folder().join(&file);
        let suite_copy = Variables::from_process()
            .get(RESULT_DOCUMENTS_VARIABLE)
            .map(|folder| PathBuf::from(folder).join(&file));
        for written in std::iter::once(&path).chain(&suite_copy) {
            written
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(written, &bytes))
                .map_err(|source| JourneyRunError::Write {
                    path: written.clone(),
                    source,
                })?;
        }
        Ok(Finished {
            outcome: value
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::to_owned),
            document,
            path,
            variant,
            failures,
            _env: env,
        })
    }

    /// The first step of the file worded `wording` that is not recorded yet.
    fn untaken_step(&self, wording: &str) -> Option<usize> {
        self.journey
            .steps
            .iter()
            .enumerate()
            .position(|(index, step)| step.text == wording && !self.taken[index])
    }

    fn failed(&self) -> bool {
        !self.failures.is_empty()
    }

    fn fail(&mut self, failure: String) {
        self.failures.push(failure);
    }

    fn record_step(&mut self, text: &str, outcome: &'static str) {
        self.steps.push(Recorded {
            text: text.to_owned(),
            task: None,
            outcome,
        });
    }

    /// Records each manual step the run has reached: those before the next
    /// automated step not yet recorded.
    fn take_manual_steps(&mut self) {
        while let Some(index) = self.taken.iter().position(|taken| !taken) {
            if self.journey.steps[index].manual.is_none() {
                break;
            }
            self.taken[index] = true;
            let text = self.journey.steps[index].text.clone();
            self.record_step(&text, "manual");
        }
    }

    fn run_body<R: BodyResult>(&self, body: impl FnOnce(&Self) -> R) -> Option<String> {
        match catch_unwind(AssertUnwindSafe(|| body(self))) {
            Ok(result) => result.failure(),
            Err(payload) => Some(panic_message(payload.as_ref())),
        }
    }

    fn judge(&self, task: &str, submission: &Value) -> Result<Value, String> {
        let Some(proving_ground) = &self.proving_ground else {
            return Err("the Journey starts no Proving ground".to_owned());
        };
        let Value::Object(fields) = submission else {
            return Err(format!("the submission is not a JSON object: {submission}"));
        };
        let mut body = Map::new();
        body.insert("task".to_owned(), json!(task));
        body.extend(fields.clone());
        let url = format!("http://{}{}", proving_ground.address(), ROUTES.oracle);
        let mut response = self
            .agent
            .post(&url)
            .send_json(Value::Object(body))
            .map_err(|error| format!("the oracle cannot be reached: {error}"))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("the oracle's answer cannot be read: {error}"))?;
        if status != 200 {
            return Err(format!("the oracle answers {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|error| format!("the oracle's answer: {error}"))
    }

    /// What the run recorded, in the result document's field names.
    fn facts(&self) -> Value {
        let recorded = |recorded: &Recorded| {
            let mut entry = json!({ "text": recorded.text, "outcome": recorded.outcome });
            if let Some(task) = &recorded.task {
                entry["task"] = json!(task);
            }
            entry
        };
        let checks: Vec<Value> = self
            .journey
            .expect
            .iter()
            .zip(&self.checks)
            .map(|(check, done)| match done {
                Some(done) => recorded(done),
                None => recorded(&Recorded {
                    text: check.text.clone(),
                    task: check.task.clone(),
                    outcome: "not_reached",
                }),
            })
            .chain(self.extra_checks.iter().map(recorded))
            .collect();
        // Each named task, once per check naming it with that check's
        // answer in the checks' order, or once when no check names it.
        let tasks: Vec<Value> = self
            .journey
            .tasks()
            .flat_map(|task| {
                let judged: Vec<Value> = self
                    .journey
                    .expect
                    .iter()
                    .enumerate()
                    .filter(|(_, check)| check.task.as_deref() == Some(task))
                    .map(|(index, _)| match self.answers.get(&index) {
                        Some(answer) => json!({ "id": task, "answer": answer }),
                        None => json!({ "id": task }),
                    })
                    .collect();
                if judged.is_empty() {
                    vec![json!({ "id": task })]
                } else {
                    judged
                }
            })
            .collect();
        let mut facts = json!({
            "journey_id": self.journey.id,
            "surface": self.surface,
            "executable": { "product": self.product, "version": self.version },
            "platform": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
            "runtime": RUNTIME,
            "steps": self.steps.iter().map(recorded).collect::<Vec<_>>(),
            "checks": checks,
            "tasks": tasks,
        });
        if let Some((name, version)) = &self.surface_package {
            facts["surface_package"] = json!({ "name": name, "version": version });
        }
        if let Some(proving_ground) = &self.proving_ground {
            let description = proving_ground.description();
            facts["proving_ground"] =
                json!({ "version": description.version, "seed": description.seed });
        }
        facts
    }
}

/// The Product and Product version an executable reports through
/// `--version`, `goodfetch-<product> <version>`.
fn reported_version(env: &TestEnv, engine: &Path) -> Result<(String, String), JourneyRunError> {
    let output = command(env, engine)
        .arg("--version")
        .output()
        .map_err(|source| JourneyRunError::Engine {
            path: engine.to_owned(),
            source,
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .split_once(' ')
        .and_then(|(binary, version)| {
            let product = binary.strip_prefix("goodfetch-")?;
            ["collie", "foxhound"]
                .contains(&product)
                .then(|| (product.to_owned(), version.to_owned()))
        })
        .filter(|_| output.status.success())
        .ok_or_else(|| JourneyRunError::EngineVersion {
            path: engine.to_owned(),
            output: stdout.into_owned(),
        })
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the step panicked".to_owned())
}

/// What a step's or check's body returns: nothing, which passes unless it
/// panics, or a `Result`, whose error fails it.
pub trait BodyResult {
    /// Why the body failed, if it did.
    fn failure(self) -> Option<String>;
}

impl BodyResult for () {
    fn failure(self) -> Option<String> {
        None
    }
}

impl<T, E: fmt::Debug> BodyResult for Result<T, E> {
    fn failure(self) -> Option<String> {
        self.err().map(|error| format!("{error:?}"))
    }
}

/// A finished run: the document its constructor returned and where it was
/// written. The test's folder, which holds the file when no results folder
/// is set, lives as long as it does.
#[must_use = "a finished run proves nothing until `assert_proved` is called"]
pub struct Finished<D> {
    document: D,
    path: PathBuf,
    variant: String,
    outcome: Option<String>,
    failures: Vec<String>,
    _env: TestEnv,
}

impl<D> Finished<D> {
    /// The document written.
    pub fn document(&self) -> &D {
        &self.document
    }

    /// Where it was written in the test's folder: this run's document,
    /// though another run of the variant writes the suite's folder after it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The variant's outcome as its document states it.
    pub fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }

    /// Each failure the run recorded: a failed step or check, an answer that
    /// is not `verified`, or a step, check or task the Journey does not
    /// hold in that place.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// Fails the test, with a [`VariantNotProved`], unless the document
    /// states `proved` and the run recorded no failure.
    pub fn assert_proved(&self) {
        if self.outcome.as_deref() == Some("proved") && self.failures.is_empty() {
            return;
        }
        let not_proved = VariantNotProved {
            variant: self.variant.clone(),
            outcome: self.outcome.clone().unwrap_or_default(),
            failures: self.failures.clone(),
            document: self.path.clone(),
        };
        #[expect(
            clippy::print_stderr,
            reason = "a typed panic payload prints nothing, so the failure is listed first, as TestEnv's teardown does"
        )]
        {
            eprintln!("{not_proved}");
        }
        std::panic::panic_any(not_proved);
    }
}

/// The panic payload of [`Finished::assert_proved`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VariantNotProved {
    /// The variant, `<id>@<surface>`.
    pub variant: String,
    /// The outcome its document states, empty when it states none.
    pub outcome: String,
    /// The failures the run recorded.
    pub failures: Vec<String>,
    /// Where the document was written.
    pub document: PathBuf,
}

impl fmt::Display for VariantNotProved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Journey variant {} is `{}`, not `proved` ({}):",
            self.variant,
            self.outcome,
            self.document.display()
        )?;
        for failure in &self.failures {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

/// The panic payload of a [`JourneyRun`] dropped without `finish`, which
/// would leave its variant with no document.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnfinishedJourneyRun {
    /// The variant, `<id>@<surface>`.
    pub variant: String,
}

impl fmt::Display for UnfinishedJourneyRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the run of Journey variant {} was dropped without `finish`, so it wrote no document",
            self.variant
        )
    }
}

struct Guard {
    variant: String,
    armed: bool,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.armed || std::thread::panicking() {
            return;
        }
        let unfinished = UnfinishedJourneyRun {
            variant: self.variant.clone(),
        };
        #[expect(
            clippy::print_stderr,
            reason = "a typed panic payload prints nothing, so the failure is listed first, as TestEnv's teardown does"
        )]
        {
            eprintln!("{unfinished}");
        }
        std::panic::panic_any(unfinished);
    }
}

/// Why a Journey variant run could not start or finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JourneyRunError {
    /// The Journey file cannot be read.
    #[error("the Journey file {} cannot be read: {source}", path.display())]
    Unreadable {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The Journey file is not one.
    #[error("the Journey file {} is invalid: {source}", path.display())]
    Invalid {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: Box<toml::de::Error>,
    },
    /// The catalog's file of one ID holds another.
    #[error("journeys/{asked}.toml holds Journey `{file}`")]
    OtherJourney {
        /// The ID asked for.
        asked: String,
        /// The ID the file holds.
        file: String,
    },
    /// The Journey has no variant on the surface.
    #[error("Journey {journey} lists no surface `{surface}`")]
    UnlistedSurface {
        /// The Journey.
        journey: String,
        /// The surface asked for.
        surface: String,
    },
    /// The executable cannot be run.
    #[error("the executable {} cannot be run: {source}", path.display())]
    Engine {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The executable's `--version` names no Product and version.
    #[error("the executable {} reports no `goodfetch-<product> <version>` through `--version`: `{output}`", path.display())]
    EngineVersion {
        /// Its path.
        path: PathBuf,
        /// What it printed.
        output: String,
    },
    /// The Journey's Proving ground did not start.
    #[error(transparent)]
    ProvingGround(#[from] ProvingGroundError),
    /// The constructor's argument does not accept the recorded facts.
    #[error("the constructor does not accept the recorded facts: {0}")]
    Facts(serde_json::Error),
    /// The constructor's document does not serialize.
    #[error("the document does not serialize: {0}")]
    Document(serde_json::Error),
    /// The document cannot be written.
    #[error("the document {} cannot be written: {source}", path.display())]
    Write {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
}
