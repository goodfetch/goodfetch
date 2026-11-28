//! Confinement policy representation and readers: either a sample policy
//! TOML file (Seam A) or the workspace's `goodfetch-confinement` Rust sources
//! parsed with `syn` (keeping `xtask` free of engine/confinement crate builds,
//! ADR 0072).

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use syn::{Expr, Item};

use crate::rust_source::parse;

const SECCOMP_SOURCE: &str = "crates/goodfetch-confinement/src/seccomp.rs";
const DENY_LIST_SOURCE: &str = "crates/goodfetch-confinement/src/deny_list.rs";
const POLICY_SOURCE: &str = "crates/goodfetch-confinement/src/policy.rs";

/// Linux socket address families `0..=45` (`AF_UNSPEC` through `AF_MAX - 1`).
const LINUX_SOCKET_FAMILIES: u32 = 46;

/// A confinement policy for one OS and its built network levels.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicySpec {
    pub(super) os: String,
    #[serde(default)]
    pub(super) mechanisms: Vec<String>,
    #[serde(default)]
    pub(super) signal_rule: Option<String>,
    pub(super) levels: Vec<LevelSpec>,
    #[serde(default)]
    pub(super) deny_list: Vec<DenyEntry>,
    #[serde(default)]
    pub(super) file_rules: Vec<String>,
}

impl PolicySpec {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LevelSpec {
    pub(super) name: String,
    pub(super) seccomp: Vec<SeccompRule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SeccompRule {
    pub(super) name: String,
    pub(super) call: String,
    pub(super) verdict: Verdict,
    #[serde(default)]
    pub(super) socket_families: Option<u32>,
    #[serde(default)]
    pub(super) ioctl_request: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Verdict {
    Allow,
    Refuse,
    NotImplemented,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DenyEntry {
    pub(super) rule: String,
    pub(super) base: DenyBase,
    pub(super) path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DenyBase {
    Home,
    GoodfetchHome,
    AppData,
    LocalAppData,
    Anywhere,
}

/// Serialized probe table matching `goodfetch_confinement::probes::ProbeTable`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProbeTable {
    pub(super) call: Vec<CallCase>,
    pub(super) path: Vec<PathCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallCase {
    pub(super) rule: String,
    pub(super) call: String,
    pub(super) args: Vec<Arg>,
    pub(super) unconfined: String,
    pub(super) confined: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pid_namespace: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum Arg {
    Number(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PathCase {
    pub(super) rule: String,
    pub(super) access: String,
    pub(super) path: String,
    pub(super) unconfined: String,
    pub(super) landlock: String,
    pub(super) fallback: String,
}

/// Reads the workspace's Linux confinement rules from `goodfetch-confinement`.
pub(super) fn read_workspace_confinement(root: &Path) -> Result<PolicySpec> {
    let seccomp_path = root.join(SECCOMP_SOURCE);
    let none = read_seccomp_level(&seccomp_path, "NONE")?;
    let outgoing_ip = read_seccomp_level(&seccomp_path, "OUTGOING_IP")?;
    let deny_list = read_linux_deny_list(&root.join(DENY_LIST_SOURCE))?;
    let (signal_rule, file_rules) = read_policy_rules(&root.join(POLICY_SOURCE))?;
    Ok(PolicySpec {
        os: "linux".to_owned(),
        mechanisms: vec![
            "landlock".to_owned(),
            "pid_namespace".to_owned(),
            "mount_namespace".to_owned(),
        ],
        signal_rule: Some(signal_rule),
        levels: vec![
            LevelSpec {
                name: "none".to_owned(),
                seccomp: none,
            },
            LevelSpec {
                name: "proxy_tcp".to_owned(),
                seccomp: outgoing_ip.clone(),
            },
            LevelSpec {
                name: "direct".to_owned(),
                seccomp: outgoing_ip,
            },
        ],
        deny_list,
        file_rules,
    })
}

fn read_seccomp_level(path: &Path, extra_const: &str) -> Result<Vec<SeccompRule>> {
    let file = parse(path)?;
    let baseline = parse_rule_slice(&file, "BASELINE", path)?;
    let ioctls = parse_ioctl_slice(&file, path)?;
    let refused = parse_rule_slice(&file, "REFUSED", path)?;
    let mut extra = parse_rule_slice(&file, extra_const, path)?;
    for rule in &mut extra {
        if rule.name == "socket" {
            rule.socket_families = Some(LINUX_SOCKET_FAMILIES);
        }
    }
    let mut rules = Vec::with_capacity(baseline.len() + ioctls.len() + refused.len() + extra.len());
    rules.extend(baseline);
    rules.extend(ioctls);
    rules.extend(refused);
    rules.extend(extra);
    Ok(rules)
}

fn find_const<'a>(file: &'a syn::File, name: &str, path: &Path) -> Result<&'a Expr> {
    file.items
        .iter()
        .find_map(|item| match item {
            Item::Const(constant) if constant.ident == name => Some(constant.expr.as_ref()),
            _ => None,
        })
        .with_context(|| format!("{}: missing `const {name}`", path.display()))
}

fn parse_rule_slice(file: &syn::File, name: &str, path: &Path) -> Result<Vec<SeccompRule>> {
    let expr = find_const(file, name, path)?;
    let inner = match expr {
        Expr::Reference(reference) => reference.expr.as_ref(),
        other => other,
    };
    let Expr::Array(array) = inner else {
        bail!("{}: `const {name}` is not an array", path.display());
    };
    array
        .elems
        .iter()
        .map(|elem| parse_rule_expr(elem, path))
        .collect()
}

fn parse_ioctl_slice(file: &syn::File, path: &Path) -> Result<Vec<SeccompRule>> {
    let expr = find_const(file, "IOCTLS", path)?;
    let inner = match expr {
        Expr::Reference(reference) => reference.expr.as_ref(),
        other => other,
    };
    let Expr::Array(array) = inner else {
        bail!("{}: `const IOCTLS` is not an array", path.display());
    };
    let mut rules = Vec::new();
    for elem in &array.elems {
        let Expr::Tuple(tuple) = elem else {
            bail!("{}: IOCTLS entry is not a tuple", path.display());
        };
        let mut iter = tuple.elems.iter();
        let name = string_lit(
            iter.next()
                .with_context(|| format!("{}: IOCTLS tuple missing name", path.display()))?,
            path,
        )?;
        let request = int_lit(
            iter.next()
                .with_context(|| format!("{}: IOCTLS tuple missing request", path.display()))?,
            path,
        )?;
        rules.push(SeccompRule {
            name,
            call: "ioctl".to_owned(),
            verdict: Verdict::Allow,
            socket_families: None,
            ioctl_request: Some(request),
        });
    }
    Ok(rules)
}

fn parse_rule_expr(expr: &Expr, path: &Path) -> Result<SeccompRule> {
    match expr {
        Expr::Call(call) => {
            let func = last_ident(&call.func).with_context(|| {
                format!("{}: expected helper call in rule list", path.display())
            })?;
            let verdict = match func.as_str() {
                "allow" | "allow_when" => Verdict::Allow,
                "refuse" | "refuse_when" => Verdict::Refuse,
                other => bail!("{}: unknown rule helper `{other}`", path.display()),
            };
            let mut args = call.args.iter();
            let name = string_lit(
                args.next()
                    .with_context(|| format!("{}: rule helper missing name", path.display()))?,
                path,
            )?;
            let call_ident = last_ident(
                args.next()
                    .with_context(|| format!("{}: rule helper missing Call", path.display()))?,
            )
            .with_context(|| format!("{}: expected Call::Variant", path.display()))?;
            Ok(SeccompRule {
                name,
                call: pascal_to_snake(&call_ident),
                verdict,
                socket_families: None,
                ioctl_request: None,
            })
        }
        Expr::Struct(struct_expr) => {
            let mut name = None;
            let mut call_name = None;
            let mut verdict = None;
            for field in &struct_expr.fields {
                let syn::Member::Named(ident) = &field.member else {
                    continue;
                };
                match ident.to_string().as_str() {
                    "name" => name = Some(string_lit(&field.expr, path)?),
                    "call" => {
                        let variant = last_ident(&field.expr).with_context(|| {
                            format!("{}: Rule.call is not a path", path.display())
                        })?;
                        call_name = Some(pascal_to_snake(&variant));
                    }
                    "verdict" => {
                        let variant = last_ident(&field.expr).with_context(|| {
                            format!("{}: Rule.verdict is not a path", path.display())
                        })?;
                        verdict = Some(match variant.as_str() {
                            "Allow" => Verdict::Allow,
                            "Refuse" => Verdict::Refuse,
                            "NotImplemented" => Verdict::NotImplemented,
                            other => bail!("{}: unknown Verdict `{other}`", path.display()),
                        });
                    }
                    _ => {}
                }
            }
            Ok(SeccompRule {
                name: name.with_context(|| format!("{}: Rule missing name", path.display()))?,
                call: call_name
                    .with_context(|| format!("{}: Rule missing call", path.display()))?,
                verdict: verdict
                    .with_context(|| format!("{}: Rule missing verdict", path.display()))?,
                socket_families: None,
                ioctl_request: None,
            })
        }
        _ => bail!(
            "{}: unexpected expression in seccomp rule slice",
            path.display()
        ),
    }
}

fn read_linux_deny_list(path: &Path) -> Result<Vec<DenyEntry>> {
    let file = parse(path)?;
    let expr = find_const(&file, "DENY_LIST", path)?;
    let inner = match expr {
        Expr::Reference(reference) => reference.expr.as_ref(),
        other => other,
    };
    let Expr::Array(array) = inner else {
        bail!("{}: `const DENY_LIST` is not an array", path.display());
    };
    let mut entries = Vec::new();
    for elem in &array.elems {
        let Expr::Call(call) = elem else {
            bail!("{}: DENY_LIST element is not entry(...)", path.display());
        };
        let mut args = call.args.iter();
        let rule = string_lit(
            args.next()
                .with_context(|| format!("{}: entry missing rule", path.display()))?,
            path,
        )?;
        let base_ident = last_ident(
            args.next()
                .with_context(|| format!("{}: entry missing base", path.display()))?,
        )
        .with_context(|| format!("{}: entry base is not a path", path.display()))?;
        let base = match base_ident.as_str() {
            "Home" => DenyBase::Home,
            "GoodfetchHome" => DenyBase::GoodfetchHome,
            "AppData" => DenyBase::AppData,
            "LocalAppData" => DenyBase::LocalAppData,
            "Anywhere" => DenyBase::Anywhere,
            other => bail!("{}: unknown Base `{other}`", path.display()),
        };
        let entry_path = string_lit(
            args.next()
                .with_context(|| format!("{}: entry missing path", path.display()))?,
            path,
        )?;
        let os_ident = last_ident(
            args.next()
                .with_context(|| format!("{}: entry missing os", path.display()))?,
        )
        .with_context(|| format!("{}: entry os is not a path", path.display()))?;
        if matches!(os_ident.as_str(), "Linux" | "Unix" | "Every") {
            entries.push(DenyEntry {
                rule,
                base,
                path: entry_path,
            });
        }
    }
    Ok(entries)
}

fn read_policy_rules(path: &Path) -> Result<(String, Vec<String>)> {
    let file = parse(path)?;
    let signal_rule = string_lit(find_const(&file, "SIGNAL_RULE", path)?, path)?;
    let file_rules_expr = find_const(&file, "FILE_RULES", path)?;
    let inner = match file_rules_expr {
        Expr::Reference(reference) => reference.expr.as_ref(),
        other => other,
    };
    let Expr::Array(array) = inner else {
        bail!("{}: `const FILE_RULES` is not an array", path.display());
    };
    let file_rules = array
        .elems
        .iter()
        .map(|elem| string_lit(elem, path))
        .collect::<Result<Vec<_>>>()?;
    Ok((signal_rule, file_rules))
}

fn string_lit(expr: &Expr, path: &Path) -> Result<String> {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(lit),
        ..
    }) = expr
    {
        return Ok(lit.value());
    }
    bail!("{}: expected string literal", path.display())
}

fn int_lit(expr: &Expr, path: &Path) -> Result<u64> {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(lit),
        ..
    }) = expr
    {
        return lit
            .base10_parse()
            .with_context(|| format!("{}: parsing integer literal", path.display()));
    }
    bail!("{}: expected integer literal", path.display())
}

fn last_ident(expr: &Expr) -> Option<String> {
    let Expr::Path(path) = expr else {
        return None;
    };
    path.path.segments.last().map(|seg| seg.ident.to_string())
}

pub(super) fn pascal_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
