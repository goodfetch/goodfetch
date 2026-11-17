//! The docs code samples (ADR 0068, as ADR 0097 amends it). `xtask
//! docs-samples extract` reads each Product's rendering of every `.md` and
//! `.mdx` page of `docs/` (not `docs/adr/` or `docs/agents/`), and every
//! package's README, and writes each sample to a file for its tool under
//! `target/docs-samples/`, with `manifest.json`:
//! - `ts` and `tsx` blocks as modules, Twoslash's `// ---cut---` lines
//!   dropped, under `typescript/`;
//! - `python` blocks as modules, under `python/`;
//! - a page's `rust` blocks as one Markdown file, under `rust/`;
//! - each `console` block as a `.trycmd` file, under `trycmd/`;
//! - each `json mcp-request` block and the `json mcp-answer` block after it
//!   as one JSON file, under `mcp/`.
//!
//! Each manifest entry names its page, rendering, line, language, markers
//! and runner, the Product whose executable or packages it runs or is
//! checked on, its check (`run`, `static`, or `listed` for a `console`
//! block or MCP pair that does not run) and the lines where the extracted
//! file holds a placeholder for the online Proving ground's origin.
//!
//! A page both Products carry runs once, in Collie's rendering, and its
//! Foxhound rendering is checked statically; a page Foxhound alone carries
//! runs on Foxhound; a README runs on its package's Product, and a part
//! marked as naming a Product is checked statically on that Product. A
//! `no_run` block is checked statically, or only listed. Before writing,
//! it runs the names check over every rendering of a page, and writes
//! nothing when a check finds anything.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod blocks;
mod names;
mod origin;
mod render;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;
use crate::products::Product;
use crate::tree;

use blocks::{Block, BodyLine};
use render::{MALFORMED, Rendering};

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Writes every sample under `target/docs-samples/`, with its manifest;
    /// fails, writing nothing, on a page it cannot read or a code block
    /// that carries the other Product's names.
    Extract,
}

/// Where the samples are written, from the workspace's folder.
const OUTPUT: &str = "target/docs-samples";

/// Folders of `docs/` that publish nothing: the decisions and the agents'
/// configuration.
const UNPUBLISHED: [&str; 2] = ["docs/adr/", "docs/agents/"];

/// The folders that hold packages, whose READMEs are published with them.
const PACKAGE_FOLDERS: [&str; 4] = ["sdks", "adapters", "integrations", "crates"];

/// The manifests that make a folder a package.
const PACKAGE_MANIFESTS: [&str; 3] = ["package.json", "pyproject.toml", "Cargo.toml"];

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Extract => extract(&environment.root),
    }
}

/// A file samples are read from.
struct Source {
    path: String,
    /// The package's Product for a README; `None` for a docs page.
    readme_of: Option<Product>,
}

#[derive(Serialize)]
struct Manifest {
    samples: Vec<Sample>,
}

#[derive(Serialize)]
struct Sample {
    page: String,
    rendering: &'static str,
    line: usize,
    language: String,
    markers: Vec<String>,
    runner: &'static str,
    product: &'static str,
    check: Check,
    /// The extracted file, from the output folder; none for a listed one.
    file: Option<String>,
    /// The source lines whose online origin the sample's file holds a
    /// placeholder for.
    origin: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Check {
    /// Run by its runner.
    Run,
    /// Compiled, type-checked or linted, never run.
    Static,
    /// Only listed: nothing checks it.
    Listed,
}

/// The tool a block is a sample for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    TypeScript,
    Python,
    Rust,
    Console,
    McpRequest,
    McpAnswer,
}

impl Tool {
    fn of(block: &Block) -> Option<Self> {
        match block.language.as_str() {
            "ts" | "tsx" => Some(Self::TypeScript),
            "python" => Some(Self::Python),
            "rust" => Some(Self::Rust),
            "console" => Some(Self::Console),
            "json" if block.marked("mcp-request") => Some(Self::McpRequest),
            "json" if block.marked("mcp-answer") => Some(Self::McpAnswer),
            _ => None,
        }
    }

    fn runner(self) -> &'static str {
        match self {
            Self::TypeScript => "vitest",
            Self::Python => "pytest",
            Self::Rust => "rustdoc",
            Self::Console => "trycmd",
            Self::McpRequest | Self::McpAnswer => "rmcp",
        }
    }
}

/// A sample's blocks: one block, or an MCP request and its answer.
#[derive(Clone, Copy)]
enum Piece<'a> {
    One(&'a Block, Tool),
    Pair(&'a Block, &'a Block),
}

/// The samples among `blocks`, each MCP request with the answer after it,
/// or what is wrong with an MCP block that has no partner.
fn pieces(blocks: &[Block]) -> Vec<Result<Piece<'_>, (&Block, &'static str)>> {
    let mut pieces = Vec::new();
    let mut rest = blocks.iter().peekable();
    while let Some(block) = rest.next() {
        let piece = match Tool::of(block) {
            None => continue,
            Some(Tool::McpRequest) => rest
                .next_if(|next| Tool::of(next) == Some(Tool::McpAnswer))
                .map(|answer| Piece::Pair(block, answer))
                .ok_or((
                    block,
                    "an `mcp-request` block is followed by no `mcp-answer` block",
                )),
            Some(Tool::McpAnswer) => Err((
                block,
                "an `mcp-answer` block follows no `mcp-request` block",
            )),
            Some(tool) => Ok(Piece::One(block, tool)),
        };
        pieces.push(piece);
    }
    pieces
}

/// What the extraction writes: each file by its path from the output
/// folder, and the manifest's entries.
#[derive(Default)]
struct Extraction {
    files: Vec<(String, String)>,
    samples: Vec<Sample>,
    findings: Vec<Finding>,
}

fn extract(root: &Path) -> Result<ExitCode> {
    let output = root.join(OUTPUT);
    if output.exists() {
        std::fs::remove_dir_all(&output)
            .with_context(|| format!("clearing {}", output.display()))?;
    }
    let mut extraction = Extraction::default();
    for source in sources(root)? {
        let text = std::fs::read_to_string(root.join(&source.path))
            .with_context(|| format!("reading {}", source.path))?;
        let renderings = match source.readme_of {
            Some(product) => vec![render::readme(product, &text)],
            None => match render::page(&source.path, &text) {
                Ok(renderings) => renderings,
                Err(findings) => {
                    extraction.findings.extend(findings);
                    continue;
                }
            },
        };
        for rendering in &renderings {
            let blocks = blocks::fenced(&rendering.lines);
            if source.readme_of.is_none() {
                extraction
                    .findings
                    .extend(names::check(&source.path, rendering.product, &blocks));
            }
            extraction.rendering(&source.path, rendering, &blocks);
        }
    }
    if !extraction.findings.is_empty() {
        return Ok(check::report(&extraction.findings));
    }
    for (path, contents) in &extraction.files {
        write(&output.join(path), contents)?;
    }
    let manifest = serde_json::to_string_pretty(&Manifest {
        samples: extraction.samples,
    })
    .context("writing the manifest")?;
    write(&output.join("manifest.json"), &format!("{manifest}\n"))?;
    output::line(&format!(
        "{} files of samples written to {OUTPUT}",
        extraction.files.len()
    ));
    Ok(ExitCode::SUCCESS)
}

fn write(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

/// Every docs page, and every README of a package whose folder names a
/// Product, by path.
fn sources(root: &Path) -> Result<Vec<Source>> {
    let mut sources: Vec<Source> = tree::files(root, "docs")?
        .into_iter()
        .filter(|path| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "md" || extension == "mdx")
                && !UNPUBLISHED.iter().any(|folder| path.starts_with(folder))
        })
        .map(|path| Source {
            path,
            readme_of: None,
        })
        .collect();
    for folder in PACKAGE_FOLDERS {
        let files = tree::files(root, folder)?;
        let present: BTreeSet<&str> = files.iter().map(String::as_str).collect();
        for path in &files {
            let Some(package) = path.strip_suffix("/README.md") else {
                continue;
            };
            let is_package = PACKAGE_MANIFESTS
                .iter()
                .any(|manifest| present.contains(format!("{package}/{manifest}").as_str()));
            if let Some(product) = package_product(package).filter(|_| is_package) {
                sources.push(Source {
                    path: path.clone(),
                    readme_of: Some(product),
                });
            }
        }
    }
    sources.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(sources)
}

/// The Product a package's folder names in a word of its path: Foxhound's
/// before Collie's, since Foxhound contains Collie (ADR 0084) and a Foxhound
/// package's path may name Collie's packages it builds on.
fn package_product(package: &str) -> Option<Product> {
    let words: BTreeSet<&str> = package.split(['/', '-', '_']).collect();
    [Product::Foxhound, Product::Collie]
        .into_iter()
        .find(|product| words.contains(product.key()))
}

impl Extraction {
    /// The samples of one rendering of the page or README at `page`.
    fn rendering(&mut self, page: &str, rendering: &Rendering, blocks: &[Block]) {
        let stem = Path::new(page)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let folder = |tool: &str| format!("{tool}/{}/{stem}", rendering.product.key());
        let mut rust = Vec::new();
        for piece in pieces(blocks) {
            let piece = match piece {
                Ok(piece) => piece,
                Err((block, message)) => {
                    self.malformed(page, rendering, block, message);
                    continue;
                }
            };
            let (block, tool) = match piece {
                Piece::One(block, tool) => (block, tool),
                Piece::Pair(request, _) => (request, Tool::McpRequest),
            };
            let (product, mut check) = placement(block, rendering.product);
            let mut origin = origin_lines(&block.body);
            let file = match piece {
                Piece::One(_, Tool::TypeScript) => Some(self.file(
                    format!(
                        "{}/line_{}.{}",
                        folder("typescript"),
                        block.line,
                        block.language
                    ),
                    module(&block.body, true),
                )),
                Piece::One(_, Tool::Python) => Some(self.file(
                    format!("{}/line_{}.py", folder("python"), block.line),
                    module(&block.body, false),
                )),
                // The page's one file, written once all its blocks are read.
                Piece::One(_, Tool::Rust) => {
                    rust.push(rust_block(block, check));
                    Some(format!("{}.md", folder("rust")))
                }
                // A `console` block: `pieces` pairs the MCP blocks.
                Piece::One(_, _) => {
                    check = listed_unless_run(check);
                    (check == Check::Run).then(|| {
                        self.file(
                            format!("{}/line_{}.trycmd", folder("trycmd"), block.line),
                            format!("```console\n{}```\n", module(&block.body, false)),
                        )
                    })
                }
                Piece::Pair(_, answer) => {
                    if answer.marked("no_run") {
                        check = Check::Static;
                    }
                    check = listed_unless_run(check);
                    origin.extend(origin_lines(&answer.body));
                    match (check == Check::Run).then(|| mcp_pair(block, answer)) {
                        None => None,
                        Some(Ok(pair)) => Some(
                            self.file(format!("{}/line_{}.json", folder("mcp"), block.line), pair),
                        ),
                        Some(Err(message)) => {
                            self.malformed(page, rendering, block, &message);
                            continue;
                        }
                    }
                }
            };
            self.samples.push(Sample {
                page: page.to_owned(),
                rendering: rendering.product.key(),
                line: block.line,
                language: block.language.clone(),
                markers: block.markers.clone(),
                runner: tool.runner(),
                product: product.key(),
                check,
                file,
                origin,
            });
        }
        if !rust.is_empty() {
            self.file(format!("{}.md", folder("rust")), rust.join("\n"));
        }
    }

    /// Records a file to write, and answers its path.
    fn file(&mut self, path: String, contents: String) -> String {
        self.files.push((path.clone(), contents));
        path
    }

    /// Records what is wrong with `block`, once: in the first rendering
    /// that carries it, since every other carries it alike.
    fn malformed(&mut self, page: &str, rendering: &Rendering, block: &Block, message: &str) {
        if block.carriers.first() == Some(&rendering.product) {
            self.findings.push(Finding::new(
                MALFORMED,
                format!("{page}:{}", block.line),
                message,
            ));
        }
    }
}

/// The Product a block runs or is checked on in `rendering`, and how: a
/// part marked as naming a Product is checked statically on it; elsewhere
/// a block runs in the first rendering that carries it, Collie's before
/// Foxhound's, and is checked statically in the others; `no_run` keeps it
/// from running.
fn placement(block: &Block, rendering: Product) -> (Product, Check) {
    let (product, runs) = match block.names {
        Some(named) => (named, false),
        None => (rendering, block.carriers.first() == Some(&rendering)),
    };
    let check = if runs && !block.marked("no_run") {
        Check::Run
    } else {
        Check::Static
    };
    (product, check)
}

/// A `console` block or an MCP pair is only listed where it does not run:
/// nothing checks it statically.
fn listed_unless_run(check: Check) -> Check {
    match check {
        Check::Run => Check::Run,
        Check::Static | Check::Listed => Check::Listed,
    }
}

fn origin_lines(body: &[BodyLine]) -> Vec<usize> {
    body.iter()
        .filter(|line| origin::holds(&line.text))
        .map(|line| line.source)
        .collect()
}

/// A block's lines as a file, the placeholder for the origin, and without
/// Twoslash's cut lines when `twoslash`.
fn module(body: &[BodyLine], twoslash: bool) -> String {
    let mut file = String::new();
    for line in body.iter().filter(|line| !(twoslash && is_cut(&line.text))) {
        file.push_str(&origin::in_text(&line.text));
        file.push('\n');
    }
    file
}

/// Whether a line is one of Twoslash's cut markers (`// ---cut---`,
/// `// ---cut-before---`, `// ---cut-after---`, `// ---cut-start---`,
/// `// ---cut-end---`).
fn is_cut(line: &str) -> bool {
    line.trim()
        .strip_prefix("// ---cut")
        .is_some_and(|rest| rest.ends_with("---"))
}

/// A `rust` block as its page's Markdown file holds it: its markers, and
/// `no_run` where it is checked statically; the origin's literals as the
/// macro's calls, the macro's hidden line after the crate's attributes.
fn rust_block(block: &Block, check: Check) -> String {
    let mut info = vec!["rust".to_owned()];
    info.extend(block.markers.iter().cloned());
    if check == Check::Static && !block.marked("no_run") {
        info.push("no_run".to_owned());
    }
    let mut called = false;
    let mut lines: Vec<String> = block
        .body
        .iter()
        .map(|line| {
            let (text, calls) = origin::in_rust(&line.text);
            called |= calls;
            text
        })
        .collect();
    if called {
        let attributes = lines
            .iter()
            .take_while(|line| {
                let line = line.trim_start();
                line.starts_with("#![") || line.starts_with("# #![")
            })
            .count();
        lines.insert(attributes, origin::RUST_MACRO.to_owned());
    }
    format!("```{}\n{}\n```\n", info.join(" "), lines.join("\n"))
}

/// An MCP pair as one JSON document, `{"request": …, "answer": …}`.
fn mcp_pair(request: &Block, answer: &Block) -> Result<String, String> {
    let parse = |block: &Block, what: &str| {
        serde_json::from_str::<serde_json::Value>(&module(&block.body, false))
            .map_err(|error| format!("the {what} is no JSON: {error}"))
    };
    let pair = serde_json::json!({
        "request": parse(request, "request")?,
        "answer": parse(answer, "answer")?,
    });
    serde_json::to_string_pretty(&pair)
        .map(|text| format!("{text}\n"))
        .map_err(|error| error.to_string())
}
