//! GitHub Actions workflow files, parsed into the parts the checks read:
//! each workflow's triggers and top-level permissions, and each job's runner,
//! matrix, reusable workflow and steps.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// What marks a test step: a step whose name starts with it (TESTING.md, No
/// retries). "all green" refuses a run whose earlier attempt failed one, the
/// egress check puts one after the deny, and `xtask no-retry` refuses a step
/// that runs a test runner under another name.
const TEST_STEP_PREFIX: &str = "Test";

/// Whether a step named `name` is a test step, the one rule every check of
/// test steps reads: of the workflow files, and of GitHub's record of a run.
pub(crate) fn is_test_step(name: &str) -> bool {
    name.starts_with(TEST_STEP_PREFIX)
}

pub(crate) struct Workflow {
    /// The file name under `.github/workflows/`, such as `pull-request.yml`.
    pub(crate) file: String,
    pub(crate) events: BTreeSet<String>,
    /// The workflow's top-level `permissions:`.
    pub(crate) permissions: Permissions,
    pub(crate) jobs: Vec<Job>,
}

/// A workflow's top-level `permissions:`, which every job's token starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Permissions {
    /// None written, so the token gets the repository's default permissions.
    Default,
    /// `permissions: {}`: no permission, each job granting its own.
    Empty,
    /// Any other value, such as a scope's grant or `read-all`.
    Granted,
}

pub(crate) struct Job {
    pub(crate) id: String,
    /// The reusable workflow a calling job runs, such as
    /// `./.github/workflows/<file>` or `<owner>/<repo>/<path>@<ref>`.
    pub(crate) uses: Option<String>,
    pub(crate) runner: Runner,
    /// The images of its job container and its service containers, as written.
    pub(crate) images: Vec<String>,
    pub(crate) steps: Vec<Step>,
}

pub(crate) struct Step {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    /// The Action the step runs, such as `actions/upload-artifact@<sha>`.
    pub(crate) uses: Option<String>,
    /// The Action's scalar inputs.
    pub(crate) with: BTreeMap<String, String>,
    /// The step's `run:` script; nothing for a step that `uses:` an Action.
    pub(crate) run: Option<String>,
    pub(crate) condition: Option<String>,
}

/// Where a job may run, as far as its `runs-on` can be read without running it.
pub(crate) enum Runner {
    /// A reusable-workflow call, which names no runner of its own.
    None,
    /// Every label set the job may run on (one per matrix value).
    Labels(Vec<Vec<String>>),
    /// A runner group, which holds self-hosted or larger runners.
    Group,
    /// An expression this check cannot resolve.
    Unresolved,
}

/// Reads every `*.yml` and `*.yaml` file of `<root>/.github/workflows`, sorted by name.
pub(crate) fn read_all(root: &Path) -> Result<Vec<Workflow>> {
    let folder = root.join(".github/workflows");
    let mut files = Vec::new();
    if folder.is_dir() {
        for entry in
            std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))?
        {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "yml" || extension == "yaml")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files.iter().map(|path| read(path)).collect()
}

fn read(path: &Path) -> Result<Workflow> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let document: Value =
        serde_saphyr::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("a workflow file name is UTF-8")?
        .to_owned();
    parse(file, &document).with_context(|| format!("reading {}", path.display()))
}

fn parse(file: String, document: &Value) -> Result<Workflow> {
    let events = match document.get("on") {
        Some(Value::String(event)) => BTreeSet::from([event.clone()]),
        Some(Value::Array(events)) => events.iter().map(scalar).collect::<Result<_>>()?,
        Some(Value::Object(events)) => events.keys().cloned().collect(),
        _ => bail!("the workflow has no `on` triggers"),
    };
    let permissions = match document.get("permissions") {
        None => Permissions::Default,
        Some(Value::Object(scopes)) if scopes.is_empty() => Permissions::Empty,
        Some(_) => Permissions::Granted,
    };
    let mut jobs = Vec::new();
    if let Some(Value::Object(entries)) = document.get("jobs") {
        for (id, job) in entries {
            jobs.push(Job {
                id: id.clone(),
                uses: job.get("uses").map(scalar).transpose()?,
                runner: runner(job)?,
                images: images(job)?,
                steps: steps(job)?,
            });
        }
    }
    Ok(Workflow {
        file,
        events,
        permissions,
        jobs,
    })
}

fn steps(job: &Value) -> Result<Vec<Step>> {
    let Some(Value::Array(steps)) = job.get("steps") else {
        return Ok(Vec::new());
    };
    steps
        .iter()
        .map(|step| {
            let mut with = BTreeMap::new();
            if let Some(Value::Object(inputs)) = step.get("with") {
                for (input, value) in inputs {
                    with.insert(input.clone(), scalar(value)?);
                }
            }
            Ok(Step {
                id: step.get("id").map(scalar).transpose()?,
                name: step.get("name").map(scalar).transpose()?,
                uses: step.get("uses").map(scalar).transpose()?,
                with,
                run: step.get("run").map(scalar).transpose()?,
                condition: step.get("if").map(scalar).transpose()?,
            })
        })
        .collect()
}

/// A job container's image (`container: <image>` or `container: { image }`)
/// and each service container's.
fn images(job: &Value) -> Result<Vec<String>> {
    let image = |container: &Value| match container {
        Value::Object(fields) => fields.get("image").map(scalar).transpose(),
        other => scalar(other).map(Some),
    };
    let mut images = Vec::new();
    if let Some(container) = job.get("container") {
        images.extend(image(container)?);
    }
    if let Some(Value::Object(services)) = job.get("services") {
        for service in services.values() {
            images.extend(image(service)?);
        }
    }
    Ok(images)
}

fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        _ => bail!("expected a scalar, found {value}"),
    }
}

fn runner(job: &Value) -> Result<Runner> {
    let Some(runs_on) = job.get("runs-on") else {
        return Ok(Runner::None);
    };
    match runs_on {
        Value::Object(_) => Ok(Runner::Group),
        Value::String(text) if text.contains("${{") => Ok(match matrix_key(text) {
            Some(key) => matrix_labels(job, key)?.map_or(Runner::Unresolved, Runner::Labels),
            None => Runner::Unresolved,
        }),
        other => {
            Ok(labels(other)?.map_or(Runner::Unresolved, |labels| Runner::Labels(vec![labels])))
        }
    }
}

/// `${{ matrix.<key> }}` gives `<key>`; any other expression gives nothing.
fn matrix_key(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix("${{")?.strip_suffix("}}")?.trim();
    let key = inner.strip_prefix("matrix.")?;
    key.chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
        .then_some(key)
}

/// The labels of a string or a list of strings; nothing when one is an expression.
fn labels(value: &Value) -> Result<Option<Vec<String>>> {
    let labels = match value {
        Value::Array(items) => items.iter().map(scalar).collect::<Result<Vec<_>>>()?,
        other => vec![scalar(other)?],
    };
    Ok((!labels.iter().any(|label| label.contains("${{"))).then_some(labels))
}

/// Every label set `matrix.<key>` takes, from the matrix's own values and its
/// `include` entries; nothing when the matrix cannot be read without running.
fn matrix_labels(job: &Value, key: &str) -> Result<Option<Vec<Vec<String>>>> {
    let Some(Value::Object(matrix)) = job
        .get("strategy")
        .and_then(|strategy| strategy.get("matrix"))
    else {
        return Ok(None);
    };
    let mut values = Vec::new();
    if let Some(Value::Array(items)) = matrix.get(key) {
        values.extend(items.iter());
    }
    if let Some(Value::Array(includes)) = matrix.get("include") {
        values.extend(includes.iter().filter_map(|include| include.get(key)));
    }
    if values.is_empty() {
        return Ok(None);
    }
    let mut sets = Vec::new();
    for value in values {
        match labels(value)? {
            Some(set) => sets.push(set),
            None => return Ok(None),
        }
    }
    Ok(Some(sets))
}
