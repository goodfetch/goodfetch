//! The same-work check: before any figure counts, every trial of both tools
//! fetched the same pages and was served the same bytes, as the Proving
//! ground's own observations record them, never a tool's report (spec
//! #152: the Proving ground is the benchmark's only source of bytes).

use std::collections::BTreeSet;

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::check::Finding;

use super::trials::{Comparison, Tool};

/// What the Proving ground saw one trial do on the site.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Work {
    /// The site's paths it answered `200`, once each.
    pub(crate) pages: BTreeSet<String>,
    /// Its requests for them.
    pub(crate) requests: u64,
    /// The bytes it sent answering them.
    pub(crate) bytes: u64,
}

/// The work `observations` (`GET /observations`) records on `host` under
/// the path prefix `site`: every `GET` it answered `200`. Anything else,
/// such as a robots.txt, is no page of the site.
pub(crate) fn work_of(observations: &Value, host: &str, site: &str) -> Result<Work> {
    let requests = observations["requests"]
        .as_array()
        .context("the observations hold no requests")?;
    let unrecorded = observations["unrecorded_requests"].as_u64().unwrap_or(0);
    anyhow::ensure!(
        unrecorded == 0,
        "the Proving ground left {unrecorded} requests unrecorded"
    );
    let mut work = Work {
        pages: BTreeSet::new(),
        requests: 0,
        bytes: 0,
    };
    for request in requests {
        let on_host = request["host"].as_str().is_some_and(|seen| {
            seen == host || seen.split_once(':').is_some_and(|(name, _)| name == host)
        });
        let Some(target) = request["target"].as_str() else {
            continue;
        };
        if !on_host
            || request["method"] != "GET"
            || request["status"] != 200
            || !target.starts_with(site)
        {
            continue;
        }
        work.pages.insert(target.to_owned());
        work.requests += 1;
        work.bytes += request["bytes"]["sent"]
            .as_u64()
            .with_context(|| format!("the request for {target} counts no bytes sent"))?;
    }
    Ok(work)
}

/// Each finding that keeps the trials' figures from counting: a tool short
/// of or past its declared trials, and every trial whose pages or bytes
/// differ from the first trial's.
pub(crate) fn check(trials: &Comparison) -> Vec<Finding> {
    let mut findings = Vec::new();
    for tool in Tool::ALL {
        let count = trials
            .trials
            .iter()
            .filter(|trial| trial.tool == tool)
            .count();
        if u32::try_from(count).ok() != Some(trials.declared.trials) {
            findings.push(Finding::new(
                "trial_count",
                tool.name(),
                format!(
                    "{count} trials, where {} were declared",
                    trials.declared.trials
                ),
            ));
        }
    }
    let Some(first) = trials.trials.first() else {
        return findings;
    };
    let reference: BTreeSet<&String> = first.pages.iter().collect();
    for trial in &trials.trials[1..] {
        let subject = format!("{} trial {}", trial.tool.name(), trial.number);
        let pages: BTreeSet<&String> = trial.pages.iter().collect();
        if pages != reference {
            let missing = reference.difference(&pages).count();
            let extra = pages.difference(&reference).count();
            findings.push(Finding::new(
                "different_pages",
                subject,
                format!(
                    "{missing} of {} trial {}'s pages missing and {extra} other pages fetched",
                    first.tool.name(),
                    first.number
                ),
            ));
        } else if trial.bytes != first.bytes || trial.requests != first.requests {
            findings.push(Finding::new(
                "different_bytes",
                subject,
                format!(
                    "{} requests served {} bytes, where {} trial {}'s {} served {}",
                    trial.requests,
                    trial.bytes,
                    first.tool.name(),
                    first.number,
                    first.requests,
                    first.bytes
                ),
            ));
        }
    }
    findings
}

#[cfg(test)]
mod tests;
