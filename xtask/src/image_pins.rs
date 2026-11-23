//! The digest-pin check (ADR 0067): every container image a workflow runs is
//! pinned by digest, as zizmor's hash-pin policy pins every Action by SHA. A
//! tag can move, so an image named only by its tag could change what runs.
//!
//! It reads where an image can enter a run:
//! - each workflow job's container and service containers, each step's
//!   `uses: docker://…`, and each literal image a `run:` script names;
//! - each composite or Docker action under `.github/actions/`;
//! - each `FROM` of a Containerfile or Dockerfile, which the jobs build;
//! - each `image.toml` under `ci/`, the pin files scripts read their images
//!   from (`repository`, `tag`, `digest`);
//! - each literal image a script under `ci/` names.
//!
//! In scripts, an image is recognised by its registry host, or as the image
//! argument of `docker` or `podman`'s `run`, `pull` or `create`, so a Docker
//! Hub short name (`postgres:16`) is checked too; an image held in a variable
//! is checked where the variable's value is written: a pin file.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::check::Finding;
use crate::{tree, workflow};

const KIND: &str = "unpinned-image";

/// The registries a literal image in a script is recognised by.
const REGISTRIES: [&str; 8] = [
    "docker.io/",
    "ghcr.io/",
    "mcr.microsoft.com/",
    "quay.io/",
    "gcr.io/",
    "registry.k8s.io/",
    "public.ecr.aws/",
    "registry.gitlab.com/",
];

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let subject = format!("{}#{}", workflow.file, job.id);
            let mut unpinned: Vec<String> = job
                .images
                .iter()
                .filter(|image| !is_pinned(image))
                .cloned()
                .collect();
            for step in &job.steps {
                if let Some(image) = step.uses.as_deref().and_then(docker_image)
                    && !is_pinned(image)
                {
                    unpinned.push(image.to_owned());
                }
                if let Some(script) = &step.run {
                    unpinned.extend(unpinned_in_script(script));
                }
            }
            if !unpinned.is_empty() {
                findings.push(finding(subject, &unpinned));
            }
        }
    }
    for file in tree::files(root, ".github/actions")? {
        if file.ends_with("/action.yml") || file.ends_with("/action.yaml") {
            let unpinned = action_images(&root.join(&file))?;
            if !unpinned.is_empty() {
                findings.push(finding(file, &unpinned));
            }
        }
    }
    for file in tree::files(root, "")? {
        let name = file.rsplit('/').next().unwrap_or(&file);
        let unpinned = if is_container_build_file(name) {
            from_images(&read(root, &file)?)
        } else if file.starts_with("ci/") && name == "image.toml" {
            pin_file(&read(root, &file)?, &file)?
        } else if file.starts_with("ci/")
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "sh")
        {
            unpinned_in_script(&read(root, &file)?)
        } else {
            continue;
        };
        if !unpinned.is_empty() {
            findings.push(finding(file, &unpinned));
        }
    }
    Ok(findings)
}

fn finding(subject: impl Into<String>, unpinned: &[String]) -> Finding {
    Finding::new(
        KIND,
        subject,
        format!(
            "pin {} by digest (`<image>@sha256:<digest>`)",
            unpinned.join(", ")
        ),
    )
}

fn read(root: &Path, file: &str) -> Result<String> {
    std::fs::read_to_string(root.join(file)).with_context(|| format!("reading {file}"))
}

/// Whether `image` names its digest: `…@sha256:` and 64 hexadecimal digits.
fn is_pinned(image: &str) -> bool {
    image
        .rsplit_once("@sha256:")
        .is_some_and(|(_, digest)| is_sha256(digest))
}

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The image of `uses: docker://<image>`.
fn docker_image(uses: &str) -> Option<&str> {
    uses.trim().strip_prefix("docker://")
}

/// The images of an action's `runs.image` (a Docker action) and its steps'
/// `uses: docker://…` (a composite action) that are not pinned.
fn action_images(path: &Path) -> Result<Vec<String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let action: Value =
        serde_saphyr::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let Some(runs) = action.get("runs") else {
        return Ok(Vec::new());
    };
    let mut images = Vec::new();
    if let Some(Value::String(image)) = runs.get("image")
        && let Some(image) = docker_image(image)
    {
        images.push(image.to_owned());
    }
    if let Some(Value::Array(steps)) = runs.get("steps") {
        for step in steps {
            if let Some(Value::String(uses)) = step.get("uses")
                && let Some(image) = docker_image(uses)
            {
                images.push(image.to_owned());
            }
            if let Some(Value::String(script)) = step.get("run") {
                images.extend(unpinned_in_script(script));
            }
        }
    }
    images.retain(|image| !is_pinned(image));
    Ok(images)
}

fn is_container_build_file(name: &str) -> bool {
    ["Containerfile", "Dockerfile"]
        .iter()
        .any(|kind| name == *kind || name.ends_with(&format!(".{kind}")))
}

/// Each `FROM` image that is not pinned, past `scratch` and the names of the
/// file's own earlier stages.
fn from_images(text: &str) -> Vec<String> {
    let mut stages = Vec::new();
    let mut unpinned = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("FROM"))
        {
            continue;
        }
        let mut words = words.skip_while(|word| word.starts_with("--"));
        let Some(image) = words.next() else {
            continue;
        };
        let earlier_stage = stages.iter().any(|stage| stage == image);
        if words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("AS"))
            && let Some(stage) = words.next()
        {
            stages.push(stage.to_owned());
        }
        if image != "scratch" && !earlier_stage && !is_pinned(image) {
            unpinned.push(image.to_owned());
        }
    }
    unpinned
}

/// A pin file's image when its `digest` is missing or not a SHA-256 digest.
fn pin_file(text: &str, file: &str) -> Result<Vec<String>> {
    let table: toml::Table = toml::from_str(text).with_context(|| format!("parsing {file}"))?;
    let digest = table.get("digest").and_then(|digest| digest.as_str());
    if digest
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .is_some_and(is_sha256)
    {
        return Ok(Vec::new());
    }
    let repository = table
        .get("repository")
        .and_then(|repository| repository.as_str())
        .unwrap_or("its image");
    Ok(vec![repository.to_owned()])
}

/// The container engines whose image argument a script's command names.
const ENGINES: [&str; 2] = ["docker", "podman"];

/// The engine subcommands that take an image.
const IMAGE_COMMANDS: [&str; 3] = ["run", "pull", "create"];

/// The options of those subcommands that take a value as the next word,
/// which is no image; any other option is read as a flag, and a value written
/// `--option=value` is one word.
const VALUE_OPTIONS: [&str; 33] = [
    "--add-host",
    "--cap-add",
    "--cap-drop",
    "--cgroupns",
    "--cpus",
    "--device",
    "--dns",
    "--entrypoint",
    "-e",
    "--env",
    "--env-file",
    "--gpus",
    "-h",
    "--hostname",
    "-l",
    "--label",
    "-m",
    "--memory",
    "--mount",
    "--name",
    "--network",
    "-p",
    "--platform",
    "--publish",
    "--pull",
    "--security-opt",
    "--shm-size",
    "--tmpfs",
    "-u",
    "--user",
    "-v",
    "--volume",
    "--workdir",
];

/// Each literal image a script names that is not pinned: a reference with
/// no digest, or with a digest that is not a SHA-256 one. An image is
/// recognised by its registry host, or as the image argument of `docker` or
/// `podman`'s `run`, `pull` or `create`, which may be a Docker Hub short
/// name (`postgres:16`). A digest from a variable (`<repository>@$digest`),
/// or an image argument that is a variable, is read from a pin file, which
/// this check reads too. Comments and URLs (`https://ghcr.io/v2/…`) are not
/// images.
fn unpinned_in_script(script: &str) -> Vec<String> {
    let mut unpinned = Vec::new();
    let script = script.replace("\\\n", " ");
    for line in script.lines() {
        let code = line.trim_start();
        if code.starts_with('#') {
            continue;
        }
        for word in code.split(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | '`' | '(' | ')' | '=')
        }) {
            let from_variable = word
                .split_once('@')
                .is_some_and(|(_, digest)| digest.starts_with('$'));
            if REGISTRIES.iter().any(|registry| word.starts_with(registry))
                && !from_variable
                && !is_pinned(word)
            {
                unpinned.push(word.to_owned());
            }
        }
        for image in engine_images(code) {
            let named_by_registry = REGISTRIES
                .iter()
                .any(|registry| image.starts_with(registry));
            if !named_by_registry && !image.starts_with('$') && !is_pinned(image) {
                unpinned.push(image.to_owned());
            }
        }
    }
    unpinned
}

/// The image argument of each `docker` or `podman` `run`, `pull` or `create`
/// on a line: the first word after the subcommand that is no option or
/// option value.
fn engine_images(code: &str) -> Vec<&str> {
    let words: Vec<&str> = code
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | '`' | '(' | ')' | ';')
        })
        .filter(|word| !word.is_empty())
        .collect();
    let mut images = Vec::new();
    for (index, pair) in words.windows(2).enumerate() {
        let engine = pair[0].rsplit('/').next().unwrap_or(pair[0]);
        if !ENGINES.contains(&engine) || !IMAGE_COMMANDS.contains(&pair[1]) {
            continue;
        }
        let mut rest = words[index + 2..].iter();
        while let Some(word) = rest.next() {
            if !word.starts_with('-') {
                images.push(*word);
                break;
            }
            if VALUE_OPTIONS.contains(word) {
                rest.next();
            }
        }
    }
    images
}
