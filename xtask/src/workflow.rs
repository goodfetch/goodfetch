//! GitHub Actions workflow files, parsed into the parts the checks read:
//! each workflow's triggers and each job's runner, condition and matrix.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

pub(crate) struct Workflow {
    /// The file name under `.github/workflows/`, such as `pull-request.yml`.
    pub(crate) file: String,
    pub(crate) events: BTreeSet<String>,
    pub(crate) jobs: Vec<Job>,
}

pub(crate) struct Job {
    pub(crate) id: String,
    pub(crate) condition: Option<String>,
    pub(crate) runner: Runner,
}

/// Where a job may run, as far as its `runs-on` can be read without running it.
pub(crate) enum Runner {
    /// A reusable-workflow call, which names no runner of its own.
    None,
    /// Every label set the job may run on (one per matrix value).
    Labels(Vec<Vec<String>>),
    /// A runner group, which may hold self-hosted runners.
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
    let mut jobs = Vec::new();
    if let Some(Value::Object(entries)) = document.get("jobs") {
        for (id, job) in entries {
            jobs.push(Job {
                id: id.clone(),
                condition: job.get("if").map(scalar).transpose()?,
                runner: runner(job)?,
            });
        }
    }
    Ok(Workflow { file, events, jobs })
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
