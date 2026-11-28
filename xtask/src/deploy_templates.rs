//! The deployment template set's repository readers (spec #583, issue 1262;
//! ADRs 0049 and 0094): renders both Products' deployment files directly from
//! `packaging/deploy/` and `packaging/oci/seccomp-profile.json` through the
//! same pure `goodfetch_protocol::deploy::render` function that `init --deploy`
//! calls, feeding both the docs guides and area 1's deployment-target smoke
//! runs so none of the three can drift.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand, ValueEnum};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::deploy::{
    DeployImagePin, DeployTarget, DeployTemplateFile, RenderedDeployFile, render,
};
use serde::Deserialize;

use crate::{output, product_version};

/// Pinned OCI image digests per Product.
pub(crate) const DIGESTS_FILE: &str = "packaging/deploy/image-digests.toml";
/// Committed container seccomp profile referenced by the Compose target.
pub(crate) const SECCOMP_FILE: &str = "packaging/oci/seccomp-profile.json";

/// Source files belonging to the `compose` deployment target, in output path
/// order: `(output_path, repo_relative_source_path)`.
pub(crate) const COMPOSE_SOURCES: [(&str, &str); 3] = [
    ("compose.yaml", "packaging/deploy/compose/compose.yaml.tmpl"),
    (
        "operator-policy.toml",
        "packaging/deploy/compose/operator-policy.toml.tmpl",
    ),
    ("seccomp-profile.json", SECCOMP_FILE),
];

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProductArg {
    Collie,
    Foxhound,
}

impl From<ProductArg> for Product {
    fn from(value: ProductArg) -> Self {
        match value {
            ProductArg::Collie => Self::Collie,
            ProductArg::Foxhound => Self::Foxhound,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum TargetArg {
    Compose,
}

impl From<TargetArg> for DeployTarget {
    fn from(value: TargetArg) -> Self {
        match value {
            TargetArg::Compose => Self::Compose,
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Renders Markdown code blocks for a deployment guide directly from the
    /// deployment template set.
    Docs(DocsArguments),
    /// Writes a deployment target's rendered files into `--out` for a
    /// deployment-target smoke run (issue 206).
    Smoke(SmokeArguments),
}

#[derive(Args)]
pub(crate) struct DocsArguments {
    #[arg(long, value_enum)]
    product: ProductArg,
    #[arg(long, value_enum)]
    target: TargetArg,
    /// Product version to render; defaults to the workspace version.
    #[arg(long)]
    version: Option<String>,
}

#[derive(Args)]
pub(crate) struct SmokeArguments {
    #[arg(long, value_enum)]
    product: ProductArg,
    #[arg(long, value_enum)]
    target: TargetArg,
    /// Directory where the rendered smoke deployment files are written.
    #[arg(long)]
    out: PathBuf,
    /// Product version to render; defaults to the workspace version.
    #[arg(long)]
    version: Option<String>,
    /// Override the pinned image digest for a locally built smoke image.
    #[arg(long)]
    image_digest: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageDigests {
    pub(crate) collie: ProductDigests,
    pub(crate) foxhound: ProductDigests,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductDigests {
    pub(crate) repository: String,
    pub(crate) default: String,
    pub(crate) nightly: String,
}

pub(crate) fn run(command: Command, root: &Path) -> Result<ExitCode> {
    match command {
        Command::Docs(args) => {
            let version = resolve_version(root, args.version.as_deref())?;
            let markdown =
                render_docs_blocks(root, args.product.into(), args.target.into(), &version)?;
            output::line(markdown.trim_end_matches('\n'));
            Ok(ExitCode::SUCCESS)
        }
        Command::Smoke(args) => {
            let version = resolve_version(root, args.version.as_deref())?;
            let out_dir = if args.out.is_absolute() {
                args.out
            } else {
                root.join(args.out)
            };
            write_smoke_files(
                root,
                args.product.into(),
                args.target.into(),
                &version,
                args.image_digest.as_deref(),
                &out_dir,
            )?;
            output::line(&format!(
                "wrote deployment smoke files into {}",
                out_dir.display()
            ));
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Parses the workspace's Product version for repository-side template readers.
pub(crate) fn workspace_version(root: &Path) -> Result<ProductVersion> {
    resolve_version(root, None)
}

fn resolve_version(root: &Path, explicit: Option<&str>) -> Result<ProductVersion> {
    if let Some(text) = explicit {
        ProductVersion::parse(text)
            .map_err(|error| anyhow::anyhow!("invalid --version `{text}`: {error}"))
    } else {
        let raw = product_version::read(root)?;
        ProductVersion::parse(&raw)
            .map_err(|error| anyhow::anyhow!("invalid workspace version `{raw}`: {error}"))
    }
}

/// Loads `packaging/deploy/image-digests.toml` from `root`.
pub(crate) fn load_digests(root: &Path) -> Result<ImageDigests> {
    let path = root.join(DIGESTS_FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn load_pin(
    root: &Path,
    product: Product,
    digest_override: Option<&str>,
) -> Result<DeployImagePin> {
    let all = load_digests(root)?;
    let entry = match product {
        Product::Collie => all.collie,
        Product::Foxhound => all.foxhound,
    };
    Ok(DeployImagePin {
        repository: entry.repository,
        default_digest: digest_override.map(str::to_owned).unwrap_or(entry.default),
        nightly_digest: digest_override.map(str::to_owned).unwrap_or(entry.nightly),
    })
}

/// Renders a target's files from the repository's template sources through
/// `goodfetch_protocol::deploy::render`.
pub(crate) fn render_from_repo(
    root: &Path,
    product: Product,
    target: DeployTarget,
    version: &ProductVersion,
    digest_override: Option<&str>,
) -> Result<Vec<RenderedDeployFile>> {
    let pin = load_pin(root, product, digest_override)?;
    let sources = match target {
        DeployTarget::Compose => &COMPOSE_SOURCES[..],
    };
    let mut loaded: Vec<(&'static str, Vec<u8>)> = Vec::with_capacity(sources.len());
    for &(output_path, source_rel) in sources {
        let source_path = root.join(source_rel);
        let bytes = std::fs::read(&source_path)
            .with_context(|| format!("reading {}", source_path.display()))?;
        loaded.push((output_path, bytes));
    }
    let templates: Vec<DeployTemplateFile<'_>> = loaded
        .iter()
        .map(|(path, bytes)| DeployTemplateFile {
            path,
            contents: bytes.as_slice(),
        })
        .collect();
    render(&templates, target, None, product, version, &pin)
        .context("rendering deployment templates")
}

/// Renders Markdown fenced code blocks for a deployment guide directly from
/// [`render_from_repo`].
pub(crate) fn render_docs_blocks(
    root: &Path,
    product: Product,
    target: DeployTarget,
    version: &ProductVersion,
) -> Result<String> {
    let files = render_from_repo(root, product, target, version, None)?;
    let mut out = String::new();
    for file in &files {
        // The seccomp profile is large and attached as a release asset;
        // docs guides embed the Compose and Operator policy files verbatim
        // and reference `seccomp-profile.json` beside them.
        if file.path == "seccomp-profile.json" {
            continue;
        }
        let fence = match Path::new(&file.path)
            .extension()
            .and_then(|ext| ext.to_str())
        {
            Some("yaml" | "yml") => "yaml",
            Some("toml") => "toml",
            Some("json") => "json",
            _ => "text",
        };
        let body = std::str::from_utf8(&file.contents)
            .with_context(|| format!("{} is not UTF-8", file.path))?;
        let _ = writeln!(out, "### `{}`\n", file.path);
        let _ = writeln!(out, "```{fence}");
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");
    }
    Ok(out)
}

/// Writes the rendered files for a deployment-target smoke run into `out_dir`.
pub(crate) fn write_smoke_files(
    root: &Path,
    product: Product,
    target: DeployTarget,
    version: &ProductVersion,
    digest_override: Option<&str>,
    out_dir: &Path,
) -> Result<Vec<RenderedDeployFile>> {
    let files = render_from_repo(root, product, target, version, digest_override)?;
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    for file in &files {
        let dest = out_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&dest, &file.contents)
            .with_context(|| format!("writing {}", dest.display()))?;
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::connection::ProductVersion;
    use goodfetch_protocol::deploy::DeployTarget;

    use super::render_docs_blocks;

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives under repo root")
    }

    #[test]
    fn docs_blocks_emit_compose_and_operator_policy_without_seccomp_dump() {
        let version = ProductVersion::parse("1.0.0").expect("version");
        let docs_md = render_docs_blocks(
            repo_root(),
            Product::Collie,
            DeployTarget::Compose,
            &version,
        )
        .expect("render docs");
        assert!(docs_md.contains("### `compose.yaml`"));
        assert!(docs_md.contains("### `operator-policy.toml`"));
        assert!(!docs_md.contains("### `seccomp-profile.json`"));
    }
}
