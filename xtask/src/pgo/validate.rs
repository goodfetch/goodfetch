//! `xtask pgo validate`: the profile-guided build against a plain one, on
//! scenario 1 and on each Product's held-out workload, trial by trial in
//! turn (ADR 0057's note of 2026-11-14). The builds are the caller's: two
//! manifests of the Products' executables.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;

use crate::compare::training::{self, Executables, Workload};
use crate::environment::Environment;
use crate::output;
use crate::test_tools::{Tool, ToolManifest};

#[derive(Args)]
pub(crate) struct Arguments {
    /// The manifest of the plain build's Products (`GOODFETCH_PGO=off`).
    #[arg(long)]
    plain: PathBuf,
    /// The manifest of the profile-guided build's Products.
    #[arg(long)]
    guided: PathBuf,
    /// The manifest that names the Proving ground.
    #[arg(long)]
    proving_ground: PathBuf,
    /// Trials of each workload on each build.
    #[arg(long, default_value_t = 9)]
    trials: u32,
}

pub(crate) fn validate(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    if arguments.trials == 0 {
        bail!("--trials is at least 1");
    }
    let plain = ToolManifest::read(&environment.path(&arguments.plain))?;
    let guided = ToolManifest::read(&environment.path(&arguments.guided))?;
    let proving_ground = ToolManifest::read(&environment.path(&arguments.proving_ground))?;
    let browser = training::browser()?;
    let driver = training::collie_driver(&environment.root)?;
    let proving_ground = proving_ground.path(Tool::ProvingGround)?;
    let builds = [&plain, &guided].map(|manifest| -> Result<Executables<'_>> {
        Ok(Executables {
            proving_ground,
            foxhound: manifest.path(Tool::Foxhound)?,
            collie: manifest.path(Tool::Collie)?,
            browser: &browser,
            collie_driver: &driver,
            goodfetch_variables: &environment.goodfetch_variables,
        })
    });
    let [plain, guided] = builds;
    let builds = [plain?, guided?];
    let workloads: Vec<Workload> = std::iter::once(training::SCENARIO_1)
        .chain(training::HELD_OUT)
        .collect();
    let mut seconds = vec![[Vec::new(), Vec::new()]; workloads.len()];
    for trial in 0..arguments.trials {
        // Each trial starts with the build the last one ended with.
        let order = if trial % 2 == 0 { [0, 1] } else { [1, 0] };
        for (workload, figures) in workloads.iter().zip(&mut seconds) {
            for build in order {
                figures[build].push(workload.run(&builds[build])?);
            }
        }
    }
    for (workload, [plain, guided]) in workloads.iter().zip(&seconds) {
        let (plain, guided) = (training::median(plain), training::median(guided));
        output::line(&format!(
            "{}: plain {plain:.3} s, profile-guided {guided:.3} s ({:+.1} %)",
            workload.name(),
            (guided / plain - 1.0) * 100.0
        ));
    }
    Ok(ExitCode::SUCCESS)
}
