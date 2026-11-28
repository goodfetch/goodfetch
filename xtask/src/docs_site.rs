//! The docs site's source tree (ADR 0097). `xtask docs-site render` writes,
//! under `target/docs-site/source/`, what Docusaurus builds in `website/`:
//! - `docs/<product>/<page>`: each Product's rendering of every `.md` and
//!   `.mdx` page of `docs/` that it carries (not `docs/adr/`, `docs/agents/`
//!   or `docs/testing/`, which publish nothing), with its names, as
//!   `xtask docs-samples` reads it;
//! - `docs/<page>`: the root's pages, from `docs/root/`, copied as written,
//!   their links written against the site's tree;
//! - `sidebars.json`: one sidebar for the root and one per Product, whose
//!   Foxhound sidebar groups the pages Collie carries under a heading that
//!   names Collie, each such page saying it is Collie's.
//!
//! A link to another page that the section does not carry is pointed at the
//! section that does, or at the root, and a link into a section's folder,
//! such as `foxhound/runs.md`, at that section's rendering, each as a file
//! link Docusaurus resolves in the reader's version. It writes nothing when
//! a page cannot be rendered.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use goodfetch_protocol::catalog::Product as ProtocolProduct;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::deploy::DeployTarget;
use serde_json::{Value, json};

use crate::check::{self, Finding};
use crate::deploy_templates;
use crate::docs_samples::render::{self, Rendering};
use crate::environment::Environment;
use crate::output;
use crate::products::Product;
use crate::tree;

mod reference;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Writes the site's source tree under `target/docs-site/source/`;
    /// fails, writing nothing, on a page it cannot render.
    Render,
}

/// Where the source tree is written, from the workspace's folder.
const OUTPUT: &str = "target/docs-site/source";

/// Folders of `docs/` that publish nothing: the decisions, the agents'
/// configuration and `TESTING.md`'s sections.
const UNPUBLISHED: [&str; 3] = ["docs/adr/", "docs/agents/", "docs/testing/"];

/// The folder of the root's pages.
const ROOT_PAGES: &str = "docs/root/";

/// The heading Foxhound's sidebar groups Collie's pages under.
const COLLIE_HEADING: &str = "Goodfetch Collie";

/// What each of Collie's pages in Foxhound's section says first.
const COLLIE_NOTE: [&str; 6] = [
    "",
    ":::note[Goodfetch Collie's page]",
    "",
    "This page is Goodfetch Collie's. Foxhound contains Collie, so it is part of Foxhound's \
     docs, written with Foxhound's names.",
    "",
    ":::",
];

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Render => render(&environment.root),
    }
}

/// Where a page is written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Root,
    Of(Product),
}

/// A published page of `docs/`.
struct Page {
    /// Its path from the workspace's folder.
    source: String,
    /// Its path from `docs/`, or from `docs/root/` for a root page.
    relative: String,
    text: String,
    content: Content,
}

enum Content {
    /// A root page, copied as written.
    Root,
    /// Each Product's rendering, Collie's first.
    Rendered(Vec<Rendering>),
}

impl Page {
    /// Each section it is written in, with its path from the tree's
    /// `docs/`.
    fn outputs(&self) -> Vec<(Section, String)> {
        match &self.content {
            Content::Root => vec![(Section::Root, self.relative.clone())],
            Content::Rendered(renderings) => renderings
                .iter()
                .map(|rendering| {
                    (
                        Section::Of(rendering.product),
                        format!("{}/{}", rendering.product.key(), self.relative),
                    )
                })
                .collect(),
        }
    }

    /// Whether it is a section's home, which no sidebar group holds.
    fn is_home(&self) -> bool {
        self.relative == "index.md" || self.relative == "index.mdx"
    }

    /// Whether Collie carries it, which puts it under Collie's heading in
    /// Foxhound's section.
    fn is_collies(&self) -> bool {
        matches!(&self.content, Content::Rendered(renderings)
            if renderings.iter().any(|rendering| rendering.product == Product::Collie))
    }
}

/// Whether `path` names a docs page, a `.md` or `.mdx` file.
fn is_page(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "md" || extension == "mdx")
}

fn render(root: &Path) -> Result<ExitCode> {
    let mut pages = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    for source in tree::files(root, "docs")? {
        let published =
            is_page(&source) && !UNPUBLISHED.iter().any(|folder| source.starts_with(folder));
        if !published {
            continue;
        }
        let text = std::fs::read_to_string(root.join(&source))
            .with_context(|| format!("reading {source}"))?;
        let (relative, content) = match source.strip_prefix(ROOT_PAGES) {
            Some(relative) => (relative.to_owned(), Content::Root),
            None => match render::page(&source, &text) {
                Ok(renderings) => (
                    source.trim_start_matches("docs/").to_owned(),
                    Content::Rendered(renderings),
                ),
                Err(found) => {
                    findings.extend(found);
                    continue;
                }
            },
        };
        pages.push(Page {
            source,
            relative,
            text,
            content,
        });
    }
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    let outputs: BTreeMap<&str, Vec<(Section, String)>> = pages
        .iter()
        .map(|page| (page.source.as_str(), page.outputs()))
        .collect();
    let mut deploy_version: Option<ProductVersion> = None;
    let mut files: Vec<(String, String)> = Vec::new();
    for page in &pages {
        match &page.content {
            Content::Root => files.push((page.relative.clone(), page.text.clone())),
            Content::Rendered(renderings) => {
                for rendering in renderings {
                    let output = format!("{}/{}", rendering.product.key(), page.relative);
                    let text = rendered(
                        root,
                        page,
                        rendering,
                        &output,
                        &outputs,
                        &mut deploy_version,
                        &mut findings,
                    )?;
                    files.push((output, text));
                }
            }
        }
    }
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    let collie_ref = reference::generate(Product::Collie);
    let foxhound_ref = reference::generate(Product::Foxhound);
    files.extend(collie_ref.files);
    files.extend(foxhound_ref.files);
    let target = root.join(OUTPUT);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .with_context(|| format!("clearing {}", target.display()))?;
    }
    for (path, text) in &files {
        write(&target.join("docs").join(path), text)?;
    }
    let sidebars =
        serde_json::to_string_pretty(&sidebars(&pages, collie_ref.sidebar, foxhound_ref.sidebar))
            .context("writing the sidebars")?;
    write(&target.join("sidebars.json"), &format!("{sidebars}\n"))?;
    output::line(&format!("{} pages written to {OUTPUT}", files.len()));
    Ok(ExitCode::SUCCESS)
}

/// Parses a standalone deployment-template directive line `{/* deploy: <target> */}`.
fn deploy_directive(line: &str) -> Option<&str> {
    let comment = line.trim().strip_prefix("{/*")?.strip_suffix("*/}")?.trim();
    let (kind, value) = comment.split_once(':')?;
    (kind.trim() == "deploy").then(|| value.trim())
}

fn protocol_product(product: Product) -> ProtocolProduct {
    match product {
        Product::Collie => ProtocolProduct::Collie,
        Product::Foxhound => ProtocolProduct::Foxhound,
    }
}

/// The page's text in `rendering`'s section, written at `output`.
fn rendered(
    root: &Path,
    page: &Page,
    rendering: &Rendering,
    output: &str,
    outputs: &BTreeMap<&str, Vec<(Section, String)>>,
    deploy_version: &mut Option<ProductVersion>,
    findings: &mut Vec<Finding>,
) -> Result<String> {
    let section = Section::Of(rendering.product);
    let mut lines: Vec<String> = front_matter(&page.text);
    let body_start = lines.len();
    let mut noted =
        !(rendering.product == Product::Foxhound && page.is_collies()) || page.is_home();
    let mut fenced = false;
    for line in &rendering.lines {
        let text = &line.text;
        if text.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced {
            lines.push(text.clone());
            continue;
        }
        if let Some(target_name) = deploy_directive(text) {
            let Some(target) = DeployTarget::named(target_name) else {
                let subject = format!("{}:{}", page.source, line.source);
                if !findings.iter().any(|finding| finding.subject == subject) {
                    findings.push(Finding::new(
                        "docs-site",
                        subject,
                        format!("`{target_name}` is no deployment target `docs-site` knows"),
                    ));
                }
                continue;
            };
            let version = match deploy_version.as_ref() {
                Some(version) => version,
                None => deploy_version.insert(deploy_templates::workspace_version(root)?),
            };
            let block = deploy_templates::render_docs_blocks(
                root,
                protocol_product(rendering.product),
                target,
                version,
            )?;
            lines.extend(block.trim_end_matches('\n').lines().map(str::to_owned));
            continue;
        }
        lines.push(relinked(text, &page.source, section, output, outputs));
        if !noted && text.starts_with("# ") {
            lines.extend(COLLIE_NOTE.map(str::to_owned));
            noted = true;
        }
    }
    if !noted {
        lines.splice(body_start..body_start, COLLIE_NOTE.map(str::to_owned));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Ok(text)
}

/// The page's front matter, its fences included, without the `products`
/// key the rendering has applied; nothing when no other key is left.
fn front_matter(text: &str) -> Vec<String> {
    let mut source = text.lines();
    if source.next() != Some("---") {
        return Vec::new();
    }
    let mut keys = Vec::new();
    let mut in_products = false;
    for line in source.by_ref() {
        if line == "---" {
            break;
        }
        let continues = line.starts_with(' ') || line.starts_with('-') || line.is_empty();
        if line.starts_with("products:") || (in_products && continues) {
            in_products = true;
            continue;
        }
        in_products = false;
        keys.push(line.to_owned());
    }
    if keys.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["---".to_owned()];
    lines.extend(keys);
    lines.push("---".to_owned());
    lines
}

/// `line` with each Markdown link to a published page pointed at that
/// page's file in `section`, or at the root's or the other section's when
/// `section` lacks it, relative to `output`.
fn relinked(
    line: &str,
    source: &str,
    section: Section,
    output: &str,
    outputs: &BTreeMap<&str, Vec<(Section, String)>>,
) -> String {
    let mut text = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        let (before, after) = rest.split_at(start + 2);
        text.push_str(before);
        let Some(end) = after.find(')') else {
            rest = after;
            break;
        };
        let (target, after) = after.split_at(end);
        text.push_str(&retarget(target, source, section, output, outputs));
        rest = after;
    }
    text.push_str(rest);
    text
}

fn retarget(
    target: &str,
    source: &str,
    section: Section,
    output: &str,
    outputs: &BTreeMap<&str, Vec<(Section, String)>>,
) -> String {
    let (path, fragment) = target
        .find('#')
        .map_or((target, ""), |at| target.split_at(at));
    if !is_page(path) || path.contains("://") || path.starts_with('/') {
        return target.to_owned();
    }
    let Some(linked) = resolved(source, path) else {
        return target.to_owned();
    };
    // A link into a section's folder names that section's rendering.
    let (linked, section) = linked
        .strip_prefix("docs/")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(key, page)| Some((format!("docs/{page}"), Product::named(key)?)))
        .map_or((linked, section), |(page, product)| {
            (page, Section::Of(product))
        });
    let Some(written) = outputs.get(linked.as_str()) else {
        return target.to_owned();
    };
    let chosen = [section, Section::Root]
        .into_iter()
        .find_map(|wanted| written.iter().find(|(section, _)| *section == wanted))
        .or_else(|| written.first());
    match chosen {
        Some((_, file)) => format!("{}{fragment}", relative_to(output, file)),
        None => target.to_owned(),
    }
}

/// The path from the workspace's folder of `link`, written in the page at
/// `source`; none when it climbs out of the workspace.
fn resolved(source: &str, link: &str) -> Option<String> {
    let mut parts: Vec<&str> = source.split('/').collect();
    parts.pop();
    for part in link.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// The link from the file at `from` to the file at `to`, both from the
/// tree's `docs/`.
fn relative_to(from: &str, to: &str) -> String {
    let mut from: Vec<&str> = from.split('/').collect();
    from.pop();
    let to: Vec<&str> = to.split('/').collect();
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec![".."; from.len() - shared];
    parts.extend(&to[shared..]);
    parts.join("/")
}

/// The root's sidebar and each Product's, by doc ID, each section's home
/// first; Foxhound's lists its own pages, then Collie's under Collie's
/// heading.
fn sidebars(pages: &[Page], collie_ref: Value, foxhound_ref: Value) -> Value {
    let id = |output: &str| {
        output
            .trim_end_matches(".mdx")
            .trim_end_matches(".md")
            .to_owned()
    };
    let homes_first = |mut ids: Vec<(bool, String)>| {
        ids.sort_by_key(|(home, _)| !*home);
        ids.into_iter()
            .map(|(_, id)| Value::String(id))
            .collect::<Vec<_>>()
    };
    let in_section = |wanted: Section, filter: &dyn Fn(&Page) -> bool| {
        homes_first(
            pages
                .iter()
                .filter(|page| filter(page))
                .flat_map(|page| {
                    page.outputs()
                        .into_iter()
                        .filter(|(section, _)| *section == wanted)
                        .map(|(_, output)| (page.is_home(), id(&output)))
                })
                .collect(),
        )
    };
    let mut collie = in_section(Section::Of(Product::Collie), &|_| true);
    collie.push(collie_ref);
    let mut foxhound = in_section(Section::Of(Product::Foxhound), &|page| {
        page.is_home() || !page.is_collies()
    });
    let collies = in_section(Section::Of(Product::Foxhound), &|page| {
        !page.is_home() && page.is_collies()
    });
    if !collies.is_empty() {
        foxhound.push(json!({
            "type": "category",
            "label": COLLIE_HEADING,
            "collapsible": false,
            "items": collies,
        }));
    }
    foxhound.push(foxhound_ref);
    json!({
        "root": in_section(Section::Root, &|_| true),
        "collie": collie,
        "foxhound": foxhound,
    })
}

fn write(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
