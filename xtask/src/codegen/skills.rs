//! The agent skills' renderings (ADRs 0065, 0095 and 0097): each skill of
//! `skills/` is one source per topic, written once with the docs' markers
//! and rendered with each carrying Product's names into its executable's
//! `generated/skills/<name>/`, which the executable embeds through the list
//! of those files beside them, `generated/skills.rs`. Collie's topics,
//! directly in `skills/`, render for both Products; Foxhound's, in
//! `skills/foxhound/`, for Foxhound alone. A source folder is named as its
//! first carrier's rendering names the skill.
//!
//! The generator fails a rendering that breaks a rule, naming the source's
//! line:
//! - the Agent Skills front matter: a `name` of at most 64 lowercase
//!   letters, digits and single hyphens, which is its folder's, and a
//!   `description` of 1 to 1,024 characters (`skill-name`,
//!   `skill-description`, `skill-front-matter`);
//! - a code span `<namespace>.<member>` whose namespace is the catalog's is
//!   an operation, which the Product's catalog must hold and offer beyond
//!   the SDKs (`skill-operation`);
//! - a command, a code span or a `console` block's `$ ` line starting with
//!   the executable's name, must be on the Product's command line
//!   (`skill-command`);
//! - a person's answer (ADR 0011's person-only entries) is never an example
//!   to run, and the line that names it asks the agent to ask for it
//!   (`skill-person-only`);
//! - the text passes the wording policy (`wording`, ADR 0035).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::check::Finding;
use crate::docs_samples::render;
use crate::features::{CommandLines, CommandTree};
use crate::products::{self, Product};
use crate::{product_version, tree, wording};

/// The folder of the skills' sources.
const SOURCES: &str = "skills";
/// The folder, in [`SOURCES`], of Foxhound's own topics.
const FOXHOUND_TOPICS: &str = "foxhound";
/// The file that makes a folder a skill.
const SKILL_FILE: &str = "SKILL.md";

/// Where Collie's executable embeds its renderings.
pub(super) const COLLIE_FOLDER: &str = concat!(products::collie_folder!(), "/generated/skills");
/// Where Foxhound's executable embeds its renderings.
pub(super) const FOXHOUND_FOLDER: &str = concat!(products::foxhound_folder!(), "/generated/skills");

/// The list of the files Collie's executable embeds, beside its renderings.
const COLLIE_LIST: &str = concat!(products::collie_folder!(), "/generated/skills.rs");
/// The list of the files Foxhound's executable embeds.
const FOXHOUND_LIST: &str = concat!(products::foxhound_folder!(), "/generated/skills.rs");

/// Where each executable crate's `generated/` folder, a Claude Code plugin
/// root, holds its manifest (story 11-239).
const PLUGIN_MANIFEST: &str = "generated/.claude-plugin/plugin.json";

/// The Agent Skills specification's limits, in characters.
const NAME_LIMIT: usize = 64;
const DESCRIPTION_LIMIT: usize = 1_024;

/// Rendered files' contents, by path from the root.
type Files = BTreeMap<String, Vec<u8>>;

/// A `SKILL.md`'s front matter, before rendering.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    name: String,
    description: String,
}

/// What a rendering of a skill may name, for one Product.
struct Reachable<'a> {
    product: Product,
    /// The catalog's namespaces, whichever Product carries them.
    namespaces: BTreeSet<&'a str>,
    /// The Product's operations, by name, with their reach.
    operations: BTreeMap<&'a str, &'a str>,
    /// The Product's command line; none in a workspace without executables.
    commands: Option<&'a CommandTree>,
}

/// Every rendering `xtask codegen` writes, by path from the root, or the
/// findings that keep a skill from rendering.
pub(super) fn files(
    root: &Path,
    catalog: &Value,
    command_lines: Option<&CommandLines>,
) -> Result<Files> {
    let (files, findings) = rendered(root, catalog, command_lines)?;
    if !findings.is_empty() {
        let lines: Vec<String> = findings
            .iter()
            .map(|finding| {
                format!(
                    "error[{}] {}: {}",
                    finding.kind, finding.subject, finding.message
                )
            })
            .collect();
        bail!("the skills do not render:\n{}", lines.join("\n"));
    }
    let lists = embedded(&files);
    Ok(files
        .into_iter()
        .chain(lists)
        .chain(plugins(root)?)
        .filter(|(path, _)| carried(root, path))
        .collect())
}

/// Each executable crate's Claude Code plugin manifest (story 11-239),
/// which makes its `generated/` folder, its renderings in `skills/`, a
/// plugin named as the executable: its skills load as
/// `<executable>:<skill>`. The version is the Product's, the licence and
/// repository the workspace's.
fn plugins(root: &Path) -> Result<Files> {
    let version = product_version::read(root)?;
    let license = product_version::workspace_package(root, "license")?;
    let repository = product_version::workspace_package(root, "repository")?;
    [
        (products::COLLIE, "Agent skills for Collie, Goodfetch's Product for browser automation: drive a real browser with goodfetch-collie and read its Runs."),
        (products::FOXHOUND, "Agent skills for Foxhound, Goodfetch's Product for crawling, which contains Collie: crawl sites and drive a real browser with goodfetch-foxhound."),
    ]
    .into_iter()
    .map(|(executable, description)| {
        let manifest = serde_json::json!({
            "name": executable.package,
            "version": version,
            "description": description,
            "author": { "name": "Goodfetch" },
            "homepage": repository,
            "repository": repository,
            "license": license,
        });
        let mut text = serde_json::to_string_pretty(&manifest)?;
        text.push('\n');
        Ok((
            format!("{}/{PLUGIN_MANIFEST}", executable.folder),
            text.into_bytes(),
        ))
    })
    .collect()
}

/// Whether the workspace at `root` holds the executable crate the generated
/// file `path` belongs to: a workspace without one, such as a test's
/// fixture, gets none of its files.
fn carried(root: &Path, path: &str) -> bool {
    products::EXECUTABLES.iter().any(|executable| {
        path.strip_prefix(executable.folder)
            .is_some_and(|rest| rest.starts_with('/'))
            && root.join(executable.folder).is_dir()
    })
}

/// Each executable's list of the renderings it embeds, an array of the
/// `SkillFile`s its crate includes, by path from its skills folder.
fn embedded(files: &Files) -> Files {
    [
        ("Collie", COLLIE_FOLDER, COLLIE_LIST),
        ("Foxhound", FOXHOUND_FOLDER, FOXHOUND_LIST),
    ]
    .into_iter()
    .map(|(product, folder, list)| {
        let mut text = format!(
            "// Generated by `xtask codegen` from `skills/`: the files of the skills\n// {product}'s executable embeds and `skill install` writes; never edit by\n// hand.\n[\n"
        );
        let prefix = format!("{folder}/");
        for path in files.keys().filter_map(|path| path.strip_prefix(&prefix)) {
            let _ = write!(
                text,
                "    SkillFile {{\n        path: {path:?},\n        contents: include_bytes!(\"skills/{path}\"),\n    }},\n"
            );
        }
        text.push_str("]\n");
        (list.to_owned(), text.into_bytes())
    })
    .collect()
}

/// Every rendering, and every finding, one per kind and line.
fn rendered(
    root: &Path,
    catalog: &Value,
    command_lines: Option<&CommandLines>,
) -> Result<(Files, Vec<Finding>)> {
    let mut files = BTreeMap::new();
    let mut findings = Vec::new();
    let topics = folders(root, SOURCES, &[FOXHOUND_TOPICS])?
        .into_iter()
        .map(|folder| (folder, Product::ALL.to_vec()))
        .chain(
            folders(root, &format!("{SOURCES}/{FOXHOUND_TOPICS}"), &[])?
                .into_iter()
                .map(|folder| (folder, vec![Product::Foxhound])),
        );
    for (folder, carriers) in topics {
        for &product in &carriers {
            let reachable = Reachable::of(catalog, command_lines, product)?;
            let first = carriers[0] == product;
            render_skill(root, &folder, &reachable, first, &mut files, &mut findings)?;
        }
    }
    let mut seen = BTreeSet::new();
    findings.retain(|finding| seen.insert((finding.kind, finding.subject.clone())));
    Ok((files, findings))
}

/// The folders directly in `folder` that hold a [`SKILL_FILE`], by path from
/// the root, but those named in `except`.
fn folders(root: &Path, folder: &str, except: &[&str]) -> Result<Vec<String>> {
    let listing = match std::fs::read_dir(root.join(folder)) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("listing {folder}")),
    };
    let mut folders = Vec::new();
    for item in listing {
        let item = item.with_context(|| format!("listing {folder}"))?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !except.contains(&name.as_str()) && item.path().join(SKILL_FILE).is_file() {
            folders.push(format!("{folder}/{name}"));
        }
    }
    folders.sort();
    Ok(folders)
}

impl<'a> Reachable<'a> {
    fn of(
        catalog: &'a Value,
        command_lines: Option<&'a CommandLines>,
        product: Product,
    ) -> Result<Self> {
        let Some(operations) = catalog.get("operations").and_then(Value::as_array) else {
            bail!("the catalog lists no operations");
        };
        let mut reachable = Self {
            product,
            namespaces: BTreeSet::new(),
            operations: BTreeMap::new(),
            commands: command_lines.map(|lines| match product {
                Product::Collie => &lines.collie,
                Product::Foxhound => &lines.foxhound,
            }),
        };
        for operation in operations {
            let field = |key: &str| operation.get(key).and_then(Value::as_str);
            let (Some(name), Some(reach)) = (field("name"), field("reach")) else {
                bail!("a catalog operation has no name or reach");
            };
            if let Some((namespace, _)) = name.split_once('.') {
                reachable.namespaces.insert(namespace);
            }
            let carried = product == Product::Foxhound || field("product") == Some("collie");
            if carried {
                reachable.operations.insert(name, reach);
            }
        }
        Ok(reachable)
    }

    /// The person-only operation `words`, a command's path, calls, if any.
    fn person_only_command(&self, words: &[&str]) -> bool {
        let path = words.join(".").replace('-', "_");
        self.operations.get(path.as_str()) == Some(&"person_only")
    }
}

/// Renders the skill at `folder` for `reachable`'s Product into `files`.
/// `first` is the rendering of its first carrier, which names its folder.
fn render_skill(
    root: &Path,
    folder: &str,
    reachable: &Reachable<'_>,
    first: bool,
    files: &mut Files,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let product = reachable.product;
    let source = format!("{folder}/{SKILL_FILE}");
    let text = read(root, &source)?;
    let Some(front) = front_matter(&source, &text, product, findings) else {
        return Ok(());
    };
    let folder_name = folder.rsplit('/').next().unwrap_or(folder);
    if first && front.name != folder_name {
        findings.push(Finding::new(
            "skill-name",
            format!("{source}:{}", front.name_line),
            format!(
                "names the skill `{}` in {}'s rendering, not its folder's `{folder_name}`",
                front.name,
                product.key()
            ),
        ));
    }
    let base = match product {
        Product::Collie => COLLIE_FOLDER,
        Product::Foxhound => FOXHOUND_FOLDER,
    };
    for file in tree::files(root, folder)? {
        let relative = file
            .strip_prefix(&format!("{folder}/"))
            .unwrap_or(&file)
            .to_owned();
        let output = format!("{base}/{}/{relative}", front.name);
        if Path::new(&file)
            .extension()
            .is_none_or(|extension| extension != "md")
        {
            files.insert(output, std::fs::read(root.join(&file))?);
            continue;
        }
        let text = read(root, &file)?;
        findings.extend(wording::markdown(root, &file, &text)?);
        let page = if file == source { &front.source } else { &text };
        let rendering = match render::page(&file, page) {
            Ok(renderings) => renderings.into_iter().find(|it| it.product == product),
            Err(malformed) => {
                findings.extend(malformed);
                continue;
            }
        };
        let lines = rendering.map(|it| it.lines).unwrap_or_default();
        check_names(&file, &lines, reachable, findings);
        let mut written = if file == source {
            front.text()
        } else {
            String::new()
        };
        for line in &lines {
            written.push_str(&line.text);
            written.push('\n');
        }
        files.insert(output, written.into_bytes());
    }
    Ok(())
}

/// A rendered front matter, with the source lines of its keys.
struct Rendered {
    name: String,
    name_line: usize,
    description: String,
    /// The `SKILL.md` with its front matter in the Product's names, line
    /// for line, for the docs' rendering of its body.
    source: String,
}

impl Rendered {
    /// Its text, the `description` a YAML double-quoted scalar.
    fn text(&self) -> String {
        let description =
            serde_json::to_string(&self.description).expect("invariant: a string serializes");
        format!(
            "---\nname: {}\ndescription: {description}\n---\n",
            self.name
        )
    }
}

/// The `SKILL.md` at `source`'s front matter rendered for `product`, or
/// none when it does not parse; each broken limit is a finding. The docs'
/// rendering reads the body after it.
fn front_matter(
    source: &str,
    text: &str,
    product: Product,
    findings: &mut Vec<Finding>,
) -> Option<Rendered> {
    let lines: Vec<&str> = text.lines().collect();
    let malformed =
        |message: String| Finding::new("skill-front-matter", format!("{source}:1"), message);
    let end = (lines.first() == Some(&"---"))
        .then(|| lines.iter().skip(1).position(|line| *line == "---"))
        .flatten();
    let Some(end) = end else {
        findings.push(malformed(
            "a skill starts with a front matter between `---` lines".to_owned(),
        ));
        return None;
    };
    // The names are written before the YAML is read, so a source writes
    // `name: {{executable}}` unquoted.
    let yaml = render::with_names(&lines[1..=end].join("\n"), product);
    let parsed: FrontMatter = match serde_saphyr::from_str(&yaml) {
        Ok(parsed) => parsed,
        Err(error) => {
            findings.push(malformed(format!("reading the front matter: {error}")));
            return None;
        }
    };
    let line_of = |key: &str| {
        lines[1..=end]
            .iter()
            .position(|line| line.starts_with(&format!("{key}:")))
            .map_or(1, |index| index + 2)
    };
    let rendered = Rendered {
        name: parsed.name,
        name_line: line_of("name"),
        description: parsed.description,
        source: ["---", yaml.as_str(), "---"]
            .into_iter()
            .chain(lines[end + 2..].iter().copied())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    if !is_skill_name(&rendered.name) {
        findings.push(Finding::new(
            "skill-name",
            format!("{source}:{}", rendered.name_line),
            format!(
                "`{}` is no Agent Skills name: at most {NAME_LIMIT} lowercase letters, digits and single hyphens",
                rendered.name
            ),
        ));
    }
    let length = rendered.description.trim().chars().count();
    if length == 0 || length > DESCRIPTION_LIMIT {
        findings.push(Finding::new(
            "skill-description",
            format!("{source}:{}", line_of("description")),
            format!(
                "the description holds {length} characters in {}'s rendering; the Agent Skills limit is 1 to {DESCRIPTION_LIMIT}",
                product.key()
            ),
        ));
    }
    Some(rendered)
}

/// Whether `name` is an Agent Skills name.
fn is_skill_name(name: &str) -> bool {
    (1..=NAME_LIMIT).contains(&name.chars().count())
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Adds a finding for each operation or command `lines` name that their
/// Product does not offer a skill.
fn check_names(
    file: &str,
    lines: &[render::Line],
    reachable: &Reachable<'_>,
    findings: &mut Vec<Finding>,
) {
    let executable = reachable.product.names().executable;
    let mut fence: Option<String> = None;
    for line in lines {
        let subject = || format!("{file}:{}", line.source);
        let trimmed = line.text.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(info.trim().to_owned()),
            };
            continue;
        }
        let asks = line
            .text
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("ask"));
        match fence.as_deref() {
            Some("console") => {
                if let Some(command) = trimmed.strip_prefix("$ ") {
                    let words: Vec<&str> = command.split_whitespace().collect();
                    if words.first() == Some(&executable) {
                        check_command(&words[1..], true, asks, reachable, &subject, findings);
                    }
                }
            }
            Some(_) => {}
            None => {
                for span in line.text.split('`').skip(1).step_by(2) {
                    let words: Vec<&str> = span.split_whitespace().collect();
                    if words.first() == Some(&executable) {
                        check_command(&words[1..], false, asks, reachable, &subject, findings);
                    } else if let Some(reach) = operation_named(span, reachable, &subject, findings)
                        && reach == "person_only"
                        && !asks
                    {
                        findings.push(person_only(subject(), span));
                    }
                }
            }
        }
    }
}

/// Checks the command `words` name after the executable's name: an
/// `example` to run, or one on a line that `asks` for it.
fn check_command(
    words: &[&str],
    example: bool,
    asks: bool,
    reachable: &Reachable<'_>,
    subject: &dyn Fn() -> String,
    findings: &mut Vec<Finding>,
) {
    let Some(mut tree) = reachable.commands else {
        return;
    };
    let mut path = Vec::new();
    for &word in words {
        if !tree.is_group() || word.starts_with('-') {
            break;
        }
        let Some(command) = tree.command(word) else {
            path.push(word);
            findings.push(Finding::new(
                "skill-command",
                subject(),
                format!(
                    "`{} {}` is no command of {}'s executable",
                    reachable.product.names().executable,
                    path.join(" "),
                    reachable.product.key()
                ),
            ));
            return;
        };
        path.push(word);
        tree = command;
    }
    if reachable.person_only_command(&path) && (example || !asks) {
        findings.push(person_only(subject(), &path.join(" ")));
    }
}

/// The reach of the operation `span` names, when it names one; a finding
/// when its Product's catalog holds none of that name for a skill.
fn operation_named<'a>(
    span: &str,
    reachable: &Reachable<'a>,
    subject: &dyn Fn() -> String,
    findings: &mut Vec<Finding>,
) -> Option<&'a str> {
    let (namespace, member) = span.split_once('.')?;
    let is_name = |part: &str| {
        part.starts_with(|character: char| character.is_ascii_lowercase())
            && part.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            })
    };
    if !reachable.namespaces.contains(namespace) || !member.split('.').all(is_name) {
        return None;
    }
    match reachable.operations.get(span) {
        Some(&reach) if reach != "sdk_only" => Some(reach),
        found => {
            findings.push(Finding::new(
                "skill-operation",
                subject(),
                match found {
                    Some(_) => format!("`{span}` is offered to the SDKs only, never in a skill"),
                    None => format!(
                        "`{span}` is no operation of {}'s catalog",
                        reachable.product.key()
                    ),
                },
            ));
            None
        }
    }
}

/// The finding of a person's answer offered as the agent's own action.
fn person_only(subject: String, name: &str) -> Finding {
    Finding::new(
        "skill-person-only",
        subject,
        format!(
            "`{name}` is a person's answer: never an example to run, and its line tells the agent to ask the person for it"
        ),
    )
}

fn read(root: &Path, file: &str) -> Result<String> {
    std::fs::read_to_string(root.join(file)).with_context(|| format!("reading {file}"))
}

#[cfg(test)]
mod tests;
