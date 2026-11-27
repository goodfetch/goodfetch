//! The browser sandbox startup probe (`SandboxProbe::of`, moved from #642 into
//! #429): generates and checks `crates/goodfetch-engine/probes/browser-sandbox.toml`
//! against every `SandboxEnvironment` and `NoSandboxPlatform` variant.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use syn::Item;

use crate::check::Finding;
use crate::rust_source::parse;

use super::policy::pascal_to_snake;

const PROTOCOL_SECURITY: &str = "crates/goodfetch-protocol/src/security.rs";
const SANDBOX_PROBE_SOURCE: &str = "crates/goodfetch-engine/src/sandbox_probe.rs";
const PROBE_TABLE_PATH: &str = "crates/goodfetch-engine/probes/browser-sandbox.toml";

const STARTS_RULE: &str = "browser_sandbox.starts";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSandboxTable {
    case: Vec<BrowserSandboxCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSandboxCase {
    rule: String,
    mechanism: String,
    positive_control: String,
    user_namespace: String,
    seccomp_filtered: bool,
    apparmor_restricts: bool,
    #[serde(default)]
    env_var: Option<String>,
    #[serde(default)]
    env_value: Option<String>,
    expected_probed: String,
    #[serde(default)]
    expected_environment: Option<String>,
    #[serde(default)]
    expected_platform: Option<String>,
    #[serde(default)]
    expected_error_code: Option<String>,
}

pub(super) fn generate(root: &Path) -> Result<()> {
    let environments = enum_variants(&root.join(PROTOCOL_SECURITY), "SandboxEnvironment")?;
    let platforms = enum_variants(&root.join(SANDBOX_PROBE_SOURCE), "NoSandboxPlatform")?;
    let table = build_table(&environments, &platforms)?;
    let out_path = root.join(PROBE_TABLE_PATH);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&out_path, format_table(&table))
        .with_context(|| format!("writing {}", out_path.display()))?;
    Ok(())
}

pub(super) fn check(root: &Path) -> Result<Vec<Finding>> {
    let environments = enum_variants(&root.join(PROTOCOL_SECURITY), "SandboxEnvironment")?;
    let platforms = enum_variants(&root.join(SANDBOX_PROBE_SOURCE), "NoSandboxPlatform")?;
    let path = root.join(PROBE_TABLE_PATH);
    if !path.is_file() {
        return Ok(vec![Finding::new(
            "missing-probe-table",
            "browser_sandbox",
            format!("expected browser sandbox probe table at {}", path.display()),
        )]);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let table: BrowserSandboxTable =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    let mut expected_rules = BTreeSet::new();
    expected_rules.insert(STARTS_RULE.to_owned());
    for env in &environments {
        expected_rules.insert(format!("browser_sandbox.environment.{env}"));
    }
    for platform in &platforms {
        expected_rules.insert(format!("browser_sandbox.platform.{platform}"));
    }

    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for case in &table.case {
        let subject = format!("browser_sandbox:{}", case.rule);
        seen.insert(case.rule.clone());
        if !expected_rules.contains(&case.rule) {
            findings.push(Finding::new(
                "probe-without-rule",
                subject.clone(),
                "browser sandbox probe names a rule that does not exist in SandboxProbe",
            ));
            continue;
        }
        if case.positive_control != "starts" {
            findings.push(Finding::new(
                "no-positive-control",
                subject.clone(),
                "browser sandbox probe must verify the `starts` positive control first",
            ));
        }
        if matches!(case.expected_probed.as_str(), "refused" | "unknown")
            && case.expected_error_code.as_deref() != Some("browser_sandbox_unavailable")
        {
            findings.push(Finding::new(
                "wrong-outcome",
                subject,
                "a refused browser sandbox probe must expect `browser_sandbox_unavailable`",
            ));
        }
    }
    for rule in expected_rules {
        if !seen.contains(&rule) {
            findings.push(Finding::new(
                "rule-without-probe",
                format!("browser_sandbox:{rule}"),
                "browser sandbox outcome has no probe case",
            ));
        }
    }
    Ok(findings)
}

fn build_table(environments: &[String], platforms: &[String]) -> Result<BrowserSandboxTable> {
    let mut cases = vec![BrowserSandboxCase {
        rule: STARTS_RULE.to_owned(),
        mechanism: "user_namespace".to_owned(),
        positive_control: "starts".to_owned(),
        user_namespace: "created".to_owned(),
        seccomp_filtered: false,
        apparmor_restricts: false,
        env_var: None,
        env_value: None,
        expected_probed: "starts".to_owned(),
        expected_environment: None,
        expected_platform: None,
        expected_error_code: None,
    }];
    for env in environments {
        cases.push(environment_case(env)?);
    }
    for platform in platforms {
        cases.push(platform_case(platform)?);
    }
    Ok(BrowserSandboxTable { case: cases })
}

fn environment_case(env: &str) -> Result<BrowserSandboxCase> {
    let rule = format!("browser_sandbox.environment.{env}");
    let case = match env {
        "docker_default_seccomp" => BrowserSandboxCase {
            rule,
            mechanism: "user_namespace".to_owned(),
            positive_control: "starts".to_owned(),
            user_namespace: "refused".to_owned(),
            seccomp_filtered: true,
            apparmor_restricts: false,
            env_var: None,
            env_value: None,
            expected_probed: "refused".to_owned(),
            expected_environment: Some(env.to_owned()),
            expected_platform: None,
            expected_error_code: Some("browser_sandbox_unavailable".to_owned()),
        },
        "kubernetes_runtime_default" => BrowserSandboxCase {
            rule,
            mechanism: "user_namespace".to_owned(),
            positive_control: "starts".to_owned(),
            user_namespace: "refused".to_owned(),
            seccomp_filtered: true,
            apparmor_restricts: false,
            env_var: Some("KUBERNETES_SERVICE_HOST".to_owned()),
            env_value: Some("10.96.0.1".to_owned()),
            expected_probed: "refused".to_owned(),
            expected_environment: Some(env.to_owned()),
            expected_platform: None,
            expected_error_code: Some("browser_sandbox_unavailable".to_owned()),
        },
        "apparmor_userns" => BrowserSandboxCase {
            rule,
            mechanism: "user_namespace".to_owned(),
            positive_control: "starts".to_owned(),
            user_namespace: "unmapped".to_owned(),
            seccomp_filtered: false,
            apparmor_restricts: true,
            env_var: None,
            env_value: None,
            expected_probed: "restricted".to_owned(),
            expected_environment: None,
            expected_platform: None,
            expected_error_code: None,
        },
        "unknown" => BrowserSandboxCase {
            rule,
            mechanism: "user_namespace".to_owned(),
            positive_control: "starts".to_owned(),
            user_namespace: "refused".to_owned(),
            seccomp_filtered: false,
            apparmor_restricts: false,
            env_var: None,
            env_value: None,
            expected_probed: "unknown".to_owned(),
            expected_environment: None,
            expected_platform: None,
            expected_error_code: Some("browser_sandbox_unavailable".to_owned()),
        },
        other => bail!("no browser sandbox probe template for SandboxEnvironment `{other}`"),
    };
    Ok(case)
}

fn platform_case(platform: &str) -> Result<BrowserSandboxCase> {
    let (env_var, env_value) = match platform {
        "aws_lambda" => ("AWS_LAMBDA_FUNCTION_NAME", "probe"),
        "fargate" => ("AWS_EXECUTION_ENV", "AWS_ECS_FARGATE"),
        "eks_fargate" => ("GOODFETCH_EKS_FARGATE_PROFILE", "probe"),
        "azure_container_apps" => ("CONTAINER_APP_NAME", "probe"),
        other => bail!("no browser sandbox probe template for NoSandboxPlatform `{other}`"),
    };
    Ok(BrowserSandboxCase {
        rule: format!("browser_sandbox.platform.{platform}"),
        mechanism: "user_namespace".to_owned(),
        positive_control: "starts".to_owned(),
        user_namespace: "refused".to_owned(),
        seccomp_filtered: true,
        apparmor_restricts: false,
        env_var: Some(env_var.to_owned()),
        env_value: Some(env_value.to_owned()),
        expected_probed: "no_sandbox".to_owned(),
        expected_environment: None,
        expected_platform: Some(platform.to_owned()),
        expected_error_code: None,
    })
}

fn format_table(table: &BrowserSandboxTable) -> String {
    let mut out = String::new();
    out.push_str("# Browser sandbox startup probe cases (ADR 0049, ADR 0067; #642 -> #429),\n");
    out.push_str("# generated by `xtask probes generate`: one case per SandboxEnvironment\n");
    out.push_str(
        "# and NoSandboxPlatform variant, each paired with the `starts` positive control.\n\n",
    );
    for case in &table.case {
        out.push_str("[[case]]\n");
        let _ = writeln!(out, "rule = {:?}", case.rule);
        let _ = writeln!(out, "mechanism = {:?}", case.mechanism);
        let _ = writeln!(out, "positive_control = {:?}", case.positive_control);
        let _ = writeln!(out, "user_namespace = {:?}", case.user_namespace);
        let _ = writeln!(out, "seccomp_filtered = {}", case.seccomp_filtered);
        let _ = writeln!(out, "apparmor_restricts = {}", case.apparmor_restricts);
        if let Some(var) = &case.env_var {
            let _ = writeln!(out, "env_var = {var:?}");
        }
        if let Some(val) = &case.env_value {
            let _ = writeln!(out, "env_value = {val:?}");
        }
        let _ = writeln!(out, "expected_probed = {:?}", case.expected_probed);
        if let Some(env) = &case.expected_environment {
            let _ = writeln!(out, "expected_environment = {env:?}");
        }
        if let Some(platform) = &case.expected_platform {
            let _ = writeln!(out, "expected_platform = {platform:?}");
        }
        if let Some(code) = &case.expected_error_code {
            let _ = writeln!(out, "expected_error_code = {code:?}");
        }
        out.push('\n');
    }
    out
}

fn enum_variants(path: &Path, enum_name: &str) -> Result<Vec<String>> {
    let file = parse(path)?;
    let item = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(en) if en.ident == enum_name => Some(en),
            _ => None,
        })
        .with_context(|| format!("{}: missing `enum {enum_name}`", path.display()))?;
    Ok(item
        .variants
        .iter()
        .map(|variant| pascal_to_snake(&variant.ident.to_string()))
        .collect())
}
