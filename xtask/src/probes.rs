//! `xtask probes generate` and `check` (ADR 0067, ticket #429): generates one
//! probe case per confinement rule and network level (and for the browser
//! sandbox startup probe, moved from #642), and checks in Quick that no rule
//! lacks a probe, no probe lacks a rule, and every refusal keeps a positive
//! control.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod browser_sandbox;
mod policy;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use crate::check::{self, Finding};
use crate::environment::Environment;

/// Where the workspace's confinement probe tables live.
const CONFINEMENT_PROBES: &str = "crates/goodfetch-confinement/probes";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Generates probe tables from the confinement policy rules and the
    /// browser sandbox startup probe, or from a sample `--policy` file into
    /// `--out`.
    Generate {
        /// A sample policy TOML file (Seam A); omitted to read the workspace's
        /// confinement and browser-sandbox rules.
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Output folder for a sample `--policy` generation.
        #[arg(long, requires = "policy")]
        out: Option<PathBuf>,
    },
    /// Fails on a rule without a probe, a probe without a rule, a refusal
    /// without a positive control, or a mismatched call or outcome.
    Check {
        /// A sample policy TOML file (Seam A); omitted to check the
        /// workspace's committed probe tables.
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Folder holding the probe tables for `--policy`.
        #[arg(long, requires = "policy")]
        probes: Option<PathBuf>,
    },
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Generate {
            policy: Some(policy_path),
            out,
        } => {
            let policy_path = environment.path(policy_path);
            let out_dir = out.as_ref().map_or_else(
                || environment.root.join(CONFINEMENT_PROBES),
                |path| environment.path(path),
            );
            let spec = policy::PolicySpec::read(&policy_path)?;
            generate_policy(&spec, &out_dir)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Generate { policy: None, .. } => {
            let spec = policy::read_workspace_confinement(&environment.root)?;
            generate_policy(&spec, &environment.root.join(CONFINEMENT_PROBES))?;
            browser_sandbox::generate(&environment.root)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            policy: Some(policy_path),
            probes,
        } => {
            let policy_path = environment.path(policy_path);
            let probes_dir = probes.as_ref().map_or_else(
                || environment.root.join(CONFINEMENT_PROBES),
                |path| environment.path(path),
            );
            let spec = policy::PolicySpec::read(&policy_path)?;
            let findings = check_policy(&spec, &probes_dir)?;
            Ok(check::report(&findings))
        }
        Command::Check { policy: None, .. } => {
            let spec = policy::read_workspace_confinement(&environment.root)?;
            let mut findings = check_policy(&spec, &environment.root.join(CONFINEMENT_PROBES))?;
            findings.extend(browser_sandbox::check(&environment.root)?);
            Ok(check::report(&findings))
        }
    }
}

fn generate_policy(spec: &policy::PolicySpec, out_dir: &Path) -> Result<()> {
    let os_dir = out_dir.join(&spec.os);
    std::fs::create_dir_all(&os_dir).with_context(|| format!("creating {}", os_dir.display()))?;
    for level in &spec.levels {
        let table = render::generate_table(spec, level)?;
        let text = render::format_table(&spec.os, &level.name, &spec.mechanisms, &table);
        let path = os_dir.join(format!("{}.toml", level.name));
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn check_policy(spec: &policy::PolicySpec, probes_dir: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for level in &spec.levels {
        let prefix = format!("{}/{}", spec.os, level.name);
        let path = probes_dir
            .join(&spec.os)
            .join(format!("{}.toml", level.name));
        if !path.is_file() {
            findings.push(Finding::new(
                "missing-probe-table",
                prefix,
                format!("expected probe table at {}", path.display()),
            ));
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let table: policy::ProbeTable =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        findings.extend(render::check_table(&prefix, &table, spec, level));
    }
    Ok(findings)
}
