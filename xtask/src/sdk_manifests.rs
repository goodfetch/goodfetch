//! `xtask sdk-manifests`, in the Quick suite: every member an SDK's manifest
//! of hand-written public members lists serves a catalog operation or none
//! (ADR 0099), and never one only a person may call (`reach =
//! "person_only"`, ADR 0041), such as `usage.raise`: no SDK member raises a
//! Limit (ADR 0080). ADR 0099's completeness tests, which match a manifest
//! to the members an SDK exports, are not this check.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

/// The catalog `xtask codegen` emits from `goodfetch-protocol`'s types.
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";
/// Each SDK's manifest; an SDK that lists its hand-written members adds its
/// own here.
const MANIFESTS: [&str; 2] = [
    "sdks/typescript/manifest.toml",
    "crates/foxhound/goodfetch-foxhound/manifest.toml",
];
/// The `entry` of a member that serves no catalog entry, with its reason.
const NO_ENTRY: &str = "none";
/// The reach of an operation only a person's surface offers.
const PERSON_ONLY: &str = "person_only";

#[derive(Deserialize)]
struct Catalog {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
struct Operation {
    name: String,
    #[serde(default)]
    reach: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "member", default)]
    members: Vec<Member>,
}

#[derive(Deserialize)]
struct Member {
    receiver: String,
    member: String,
    entry: String,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join(CATALOG);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let catalog: Catalog =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let reach: BTreeMap<&str, Option<&str>> = catalog
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation.reach.as_deref()))
        .collect();
    let mut findings = Vec::new();
    for manifest in MANIFESTS {
        let path = root.join(manifest);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: Manifest =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for member in &parsed.members {
            let subject = format!("{manifest} {}.{}", member.receiver, member.member);
            if member.entry == NO_ENTRY {
                continue;
            }
            match reach.get(member.entry.as_str()) {
                None => findings.push(Finding::new(
                    "unknown-entry",
                    subject,
                    format!("`{}` is no operation of the catalog", member.entry),
                )),
                Some(Some(PERSON_ONLY)) => findings.push(Finding::new(
                    "person-only",
                    subject,
                    format!(
                        "`{}` is only a person's to call (ADR 0041); no SDK member serves it",
                        member.entry
                    ),
                )),
                Some(_) => {}
            }
        }
    }
    Ok(findings)
}
