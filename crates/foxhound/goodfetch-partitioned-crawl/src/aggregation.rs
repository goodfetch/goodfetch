//! Aggregated reporting for a Partitioned crawl (ADR 0054, story:3-119):
//! - **Lifecycle:** `running` while any Partition runs; else `failed` if any
//!   Partition failed; else `stopped` if any Partition stopped; else
//!   `finished`.
//! - **Verdict:** worst of the Partitions' Verdicts: `rejected`, then
//!   `needs_review` (or an unknown Verdict), then `verified`.
//! - **Completeness:** `incomplete` if any Partition's is; else `unverified`
//!   if any Partition's is; else `complete`.
//! - **Usage:** summed across every Partition, preserving the `partitioned_crawl`
//!   Limit scope (and tightest remaining amount for each applied Limit).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::model::ModelTokens;
use goodfetch_protocol::run::{
    AppliedLimit, AxisReason, Completeness, KindUsage, Lifecycle, ModelAllowance, ModelCounts,
    ModelUsage, PluginUsage, ProxyKindBytes, RunId, RunReport, RunUsage, RunWarning, UsageAmount,
    Verdict,
};
use serde_json::{Map, Value};

/// Aggregates Partition [`Lifecycle`]s according to ADR 0054.
#[must_use]
pub fn aggregate_lifecycle(lifecycles: impl IntoIterator<Item = Lifecycle>) -> Lifecycle {
    let mut any_running = false;
    let mut any_failed = false;
    let mut any_stopped = false;
    for lifecycle in lifecycles {
        match lifecycle {
            Lifecycle::Running => any_running = true,
            Lifecycle::Failed => any_failed = true,
            Lifecycle::Stopped => any_stopped = true,
            Lifecycle::Finished => {}
        }
    }
    if any_running {
        Lifecycle::Running
    } else if any_failed {
        Lifecycle::Failed
    } else if any_stopped {
        Lifecycle::Stopped
    } else {
        Lifecycle::Finished
    }
}

/// Aggregates Partition [`Verdict`]s according to ADR 0054:
/// `rejected` > `needs_review` / unknown > `verified`.
#[must_use]
pub fn aggregate_verdict<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> Verdict {
    let mut worst = Verdict::Verified;
    for verdict in verdicts {
        match (&worst, verdict) {
            (Verdict::Rejected, _) => {}
            (_, Verdict::Rejected) => worst = Verdict::Rejected,
            (Verdict::Verified, Verdict::NeedsReview) => worst = Verdict::NeedsReview,
            (Verdict::Verified, Verdict::Unknown(u)) => worst = Verdict::Unknown(u.clone()),
            _ => {}
        }
    }
    worst
}

/// Aggregates Partition [`Completeness`]es according to ADR 0054:
/// `incomplete` > `unverified` > `complete`.
#[must_use]
pub fn aggregate_completeness(
    completenesses: impl IntoIterator<Item = Completeness>,
) -> Completeness {
    let mut worst = Completeness::Complete;
    for completeness in completenesses {
        match (worst, completeness) {
            (Completeness::Incomplete, _) => {}
            (_, Completeness::Incomplete) => worst = Completeness::Incomplete,
            (Completeness::Complete, Completeness::Unverified) => {
                worst = Completeness::Unverified;
            }
            _ => {}
        }
    }
    worst
}

/// Sums [`RunUsage`] across every Partition (ADR 0054).
#[must_use]
pub fn sum_usage<'a>(usages: impl IntoIterator<Item = &'a RunUsage>) -> RunUsage {
    let mut total = RunUsage::default();
    let mut proxy_by_kind: BTreeMap<String, u64> = BTreeMap::new();
    let mut proxy_price: BTreeMap<Option<String>, u64> = BTreeMap::new();
    let mut paid_solves: BTreeMap<Option<String>, u64> = BTreeMap::new();
    let mut paid_fetches: BTreeMap<Option<String>, u64> = BTreeMap::new();
    let mut plugins: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut other: BTreeMap<(String, Option<String>), u64> = BTreeMap::new();
    let mut limits: Vec<AppliedLimit> = Vec::new();
    let mut allowances: Vec<ModelAllowance> = Vec::new();

    for usage in usages {
        add_model_usage(&mut total.model, &usage.model);
        total.browser_time = total.browser_time.saturating_add(usage.browser_time);
        total.proxy_bytes = total.proxy_bytes.saturating_add(usage.proxy_bytes);
        total.requests = total.requests.saturating_add(usage.requests);
        total.uncapped |= usage.uncapped;

        for entry in &usage.proxy_bytes_by_kind {
            *proxy_by_kind.entry(entry.kind.clone()).or_default() = proxy_by_kind
                .get(&entry.kind)
                .copied()
                .unwrap_or_default()
                .saturating_add(entry.bytes);
        }
        accumulate_amounts(&mut proxy_price, &usage.proxy_price);
        accumulate_amounts(&mut paid_solves, &usage.paid_solves);
        accumulate_amounts(&mut paid_fetches, &usage.paid_fetches);
        for p in &usage.plugins {
            let slot = plugins.entry(p.plugin.clone()).or_default();
            slot.0 = slot.0.saturating_add(p.calls);
            slot.1 = slot.1.saturating_add(p.time);
        }
        for k in &usage.other {
            let slot = other
                .entry((k.usage_kind.clone(), k.currency.clone()))
                .or_default();
            *slot = slot.saturating_add(k.amount);
        }
        merge_limits(&mut limits, &usage.limits);
        merge_allowances(&mut allowances, &usage.model_allowance);
    }

    total.proxy_bytes_by_kind = proxy_by_kind
        .into_iter()
        .map(|(kind, bytes)| ProxyKindBytes { kind, bytes })
        .collect();
    total.proxy_price = collect_amounts(proxy_price);
    total.paid_solves = collect_amounts(paid_solves);
    total.paid_fetches = collect_amounts(paid_fetches);
    total.plugins = plugins
        .into_iter()
        .map(|(plugin, (calls, time))| PluginUsage {
            plugin,
            calls,
            time,
        })
        .collect();
    total.other = other
        .into_iter()
        .map(|((usage_kind, currency), amount)| KindUsage {
            usage_kind,
            currency,
            amount,
        })
        .collect();
    total.limits = limits;
    total.model_allowance = allowances;
    total
}

fn add_model_usage(dst: &mut ModelUsage, src: &ModelUsage) {
    add_model_counts(&mut dst.counts, &src.counts);
    for (name, counts) in &src.by_model {
        add_model_counts(dst.by_model.entry(name.clone()).or_default(), counts);
    }
}

fn add_model_counts(dst: &mut ModelCounts, src: &ModelCounts) {
    dst.calls = dst.calls.saturating_add(src.calls);
    dst.tokens = dst.tokens.saturating_add(src.tokens);
    dst.parts = ModelTokens {
        input_tokens: dst
            .parts
            .input_tokens
            .saturating_add(src.parts.input_tokens),
        cache_read_tokens: dst
            .parts
            .cache_read_tokens
            .saturating_add(src.parts.cache_read_tokens),
        cache_write_tokens: dst
            .parts
            .cache_write_tokens
            .saturating_add(src.parts.cache_write_tokens),
        output_tokens: dst
            .parts
            .output_tokens
            .saturating_add(src.parts.output_tokens),
        reasoning_tokens: dst
            .parts
            .reasoning_tokens
            .saturating_add(src.parts.reasoning_tokens),
    };
    dst.price = match (&dst.price, &src.price) {
        (Some(a), Some(b)) if a.currency == b.currency => Some(UsageAmount {
            currency: a.currency.clone(),
            amount: a.amount.saturating_add(b.amount),
        }),
        (None, Some(b)) if dst.calls == src.calls => Some(b.clone()),
        (Some(a), None) if src.calls == 0 => Some(a.clone()),
        _ => None,
    };
}

fn accumulate_amounts(dst: &mut BTreeMap<Option<String>, u64>, src: &[UsageAmount]) {
    for item in src {
        let entry = dst.entry(item.currency.clone()).or_default();
        *entry = entry.saturating_add(item.amount);
    }
}

fn collect_amounts(map: BTreeMap<Option<String>, u64>) -> Vec<UsageAmount> {
    map.into_iter()
        .map(|(currency, amount)| UsageAmount { currency, amount })
        .collect()
}

fn merge_limits(dst: &mut Vec<AppliedLimit>, src: &[AppliedLimit]) {
    for incoming in src {
        if let Some(existing) = dst.iter_mut().find(|existing| {
            existing.scope == incoming.scope
                && existing.usage_kind == incoming.usage_kind
                && existing.currency == incoming.currency
                && existing.period == incoming.period
                && existing.amount == incoming.amount
        }) {
            // Keep the tightest remaining budget observed across Partitions.
            existing.left = existing.left.min(incoming.left);
        } else {
            dst.push(incoming.clone());
        }
    }
}

fn merge_allowances(dst: &mut Vec<ModelAllowance>, src: &[ModelAllowance]) {
    for incoming in src {
        if let Some(existing) = dst.iter_mut().find(|existing| {
            existing.kind == incoming.kind && existing.currency == incoming.currency
        }) {
            existing.remains = existing.remains.min(incoming.remains);
            if existing.reached_at.is_none() {
                existing.reached_at = incoming.reached_at;
            }
        } else {
            dst.push(incoming.clone());
        }
    }
}

/// Aggregates Partition [`RunReport`]s into one [`RunReport`] for the
/// Partitioned crawl `crawl_id` (ADR 0054).
///
/// # Panics
///
/// Panics if `reports` is empty.
#[must_use]
pub fn aggregate_reports(crawl_id: &RunId, reports: &[RunReport]) -> RunReport {
    assert!(
        !reports.is_empty(),
        "invariant: a Partitioned crawl has at least one Partition report"
    );

    let lifecycle = aggregate_lifecycle(reports.iter().map(|r| r.lifecycle));
    let verdict = aggregate_verdict(reports.iter().map(|r| &r.verdict));
    let completeness = aggregate_completeness(reports.iter().map(|r| r.completeness));
    let usage = sum_usage(reports.iter().map(|r| &r.usage));

    let started_at = reports
        .iter()
        .map(|r| r.started_at)
        .min()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let ended_at = reports
        .iter()
        .map(|r| r.ended_at)
        .max()
        .unwrap_or(started_at);

    let stop_reason = if lifecycle == Lifecycle::Stopped {
        reports.iter().find_map(|r| r.stop_reason.clone())
    } else {
        None
    };
    let error = if lifecycle == Lifecycle::Failed {
        reports.iter().find_map(|r| r.error.clone())
    } else {
        None
    };

    let mut verdict_reasons: Vec<AxisReason> = Vec::new();
    let mut completeness_reasons: Vec<AxisReason> = Vec::new();
    let mut warnings: Vec<RunWarning> = Vec::new();
    let mut weakened = Vec::new();
    for report in reports {
        verdict_reasons.extend(report.verdict_reasons.iter().cloned());
        completeness_reasons.extend(report.completeness_reasons.iter().cloned());
        warnings.extend(report.warnings.iter().cloned());
        for w in &report.weakened {
            if !weakened.contains(w) {
                weakened.push(w.clone());
            }
        }
    }

    let kind_parts = aggregate_crawl_kind_parts(reports);

    RunReport {
        run_id: crawl_id.clone(),
        lifecycle,
        stop_reason,
        verdict,
        verdict_reasons,
        completeness,
        completeness_reasons,
        started_at,
        ended_at,
        usage,
        weakened,
        plan: None,
        policy_version: reports.iter().find_map(|r| r.policy_version.clone()),
        settings: reports.first().and_then(|r| r.settings.clone()),
        plugins: None,
        warnings,
        recording: None,
        checkpoint: reports.iter().find_map(|r| {
            r.checkpoint.clone().map(|mut cp| {
                cp.run_id = crawl_id.clone();
                cp
            })
        }),
        nothing_done: None,
        uncertain_effects: None,
        error,
        redone: {
            let mut total = goodfetch_protocol::run::RedoneReport::default();
            for r in reports {
                if let Some(redone) = r.redone {
                    total.attempts = total.attempts.saturating_add(redone.attempts);
                    total.batch_writes = total.batch_writes.saturating_add(redone.batch_writes);
                }
            }
            (!total.is_empty()).then_some(total)
        },
        kind_parts,
    }
}

/// Sums Foxhound crawl `kind_parts` (`requests` and `records`) across
/// Partitions.
fn aggregate_crawl_kind_parts(reports: &[RunReport]) -> Map<String, Value> {
    let mut out = Map::new();

    // Aggregate `requests` object if present in any Partition report.
    let mut req_succeeded = 0_u64;
    let mut req_failed = 0_u64;
    let mut req_skipped = 0_u64;
    let mut req_blocked = 0_u64;
    let mut req_pending = 0_u64;
    let mut any_requests = false;

    for report in reports {
        if let Some(requests) = report.kind_parts.get("requests").and_then(Value::as_object) {
            any_requests = true;
            req_succeeded = req_succeeded.saturating_add(
                requests
                    .get("succeeded")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
            req_failed = req_failed
                .saturating_add(requests.get("failed").and_then(Value::as_u64).unwrap_or(0));
            req_skipped = req_skipped
                .saturating_add(requests.get("skipped").and_then(Value::as_u64).unwrap_or(0));
            req_blocked = req_blocked
                .saturating_add(requests.get("blocked").and_then(Value::as_u64).unwrap_or(0));
            req_pending = req_pending
                .saturating_add(requests.get("pending").and_then(Value::as_u64).unwrap_or(0));
        }
    }
    if any_requests {
        let mut req_obj = Map::new();
        req_obj.insert("succeeded".to_owned(), Value::from(req_succeeded));
        req_obj.insert("failed".to_owned(), Value::from(req_failed));
        req_obj.insert("skipped".to_owned(), Value::from(req_skipped));
        req_obj.insert("blocked".to_owned(), Value::from(req_blocked));
        if req_pending > 0 {
            req_obj.insert("pending".to_owned(), Value::from(req_pending));
        }
        out.insert("requests".to_owned(), Value::Object(req_obj));
    }

    // Aggregate `records` per-dataset counts if present.
    let mut datasets: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    for report in reports {
        if let Some(records) = report.kind_parts.get("records").and_then(Value::as_object) {
            for (dataset, counts) in records {
                if let Some(obj) = counts.as_object() {
                    let entry = datasets.entry(dataset.clone()).or_default();
                    entry.0 = entry
                        .0
                        .saturating_add(obj.get("committed").and_then(Value::as_u64).unwrap_or(0));
                    entry.1 = entry
                        .1
                        .saturating_add(obj.get("dropped").and_then(Value::as_u64).unwrap_or(0));
                    entry.2 = entry
                        .2
                        .saturating_add(obj.get("rejected").and_then(Value::as_u64).unwrap_or(0));
                }
            }
        }
    }
    if !datasets.is_empty() {
        let mut rec_obj = Map::new();
        for (dataset, (committed, dropped, rejected)) in datasets {
            let mut ds = Map::new();
            ds.insert("committed".to_owned(), Value::from(committed));
            ds.insert("dropped".to_owned(), Value::from(dropped));
            ds.insert("rejected".to_owned(), Value::from(rejected));
            rec_obj.insert(dataset, Value::Object(ds));
        }
        out.insert("records".to_owned(), Value::Object(rec_obj));
    }

    out
}

/// Builds a lightweight synthetic [`RunReport`] for a Partition that is still
/// running (used when `run.report` is called while one or more Partitions are
/// live).
#[must_use]
pub fn running_placeholder(run_id: &RunId, started_at: SystemTime, usage: RunUsage) -> RunReport {
    RunReport {
        run_id: run_id.clone(),
        lifecycle: Lifecycle::Running,
        stop_reason: None,
        verdict: Verdict::Verified,
        verdict_reasons: Vec::new(),
        completeness: Completeness::Complete,
        completeness_reasons: Vec::new(),
        started_at,
        ended_at: started_at + Duration::ZERO,
        usage,
        weakened: Vec::new(),
        plan: None,
        policy_version: None,
        settings: None,
        plugins: None,
        warnings: Vec::new(),
        recording: None,
        checkpoint: None,
        nothing_done: None,
        uncertain_effects: None,
        error: None,
        redone: None,
        kind_parts: Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use goodfetch_protocol::run::{
        Completeness, Lifecycle, RunId, RunReport, RunUsage, StopReason, Verdict,
    };

    use super::{
        aggregate_completeness, aggregate_lifecycle, aggregate_reports, aggregate_verdict,
        running_placeholder, sum_usage,
    };

    fn sample_report(
        id: &str,
        lifecycle: Lifecycle,
        verdict: Verdict,
        completeness: Completeness,
        requests: u64,
        proxy_bytes: u64,
    ) -> RunReport {
        let mut report = running_placeholder(
            &RunId::parse(id).unwrap(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(100),
            RunUsage {
                requests,
                proxy_bytes,
                browser_time: Duration::from_millis(250),
                ..RunUsage::default()
            },
        );
        report.lifecycle = lifecycle;
        report.verdict = verdict;
        report.completeness = completeness;
        if lifecycle == Lifecycle::Stopped {
            report.stop_reason = Some(StopReason::Pause);
        }
        report
    }

    /// Covers: story:3-119
    #[test]
    fn lifecycle_verdict_completeness_and_usage_aggregate_per_adr_0054() {
        assert_eq!(
            aggregate_lifecycle([Lifecycle::Finished, Lifecycle::Running, Lifecycle::Failed]),
            Lifecycle::Running
        );
        assert_eq!(
            aggregate_lifecycle([Lifecycle::Finished, Lifecycle::Stopped, Lifecycle::Failed]),
            Lifecycle::Failed
        );
        assert_eq!(
            aggregate_lifecycle([Lifecycle::Finished, Lifecycle::Stopped]),
            Lifecycle::Stopped
        );
        assert_eq!(
            aggregate_lifecycle([Lifecycle::Finished, Lifecycle::Finished]),
            Lifecycle::Finished
        );

        assert_eq!(
            aggregate_verdict(&[Verdict::Verified, Verdict::NeedsReview, Verdict::Rejected]),
            Verdict::Rejected
        );
        assert_eq!(
            aggregate_verdict(&[Verdict::Verified, Verdict::NeedsReview]),
            Verdict::NeedsReview
        );
        assert_eq!(
            aggregate_verdict(&[Verdict::Verified, Verdict::Verified]),
            Verdict::Verified
        );

        assert_eq!(
            aggregate_completeness([
                Completeness::Complete,
                Completeness::Unverified,
                Completeness::Incomplete,
            ]),
            Completeness::Incomplete
        );
        assert_eq!(
            aggregate_completeness([Completeness::Complete, Completeness::Unverified]),
            Completeness::Unverified
        );
        assert_eq!(
            aggregate_completeness([Completeness::Complete, Completeness::Complete]),
            Completeness::Complete
        );

        let r0 = sample_report(
            "crawl-p0",
            Lifecycle::Finished,
            Verdict::Verified,
            Completeness::Complete,
            12,
            1000,
        );
        let r1 = sample_report(
            "crawl-p1",
            Lifecycle::Stopped,
            Verdict::NeedsReview,
            Completeness::Unverified,
            8,
            500,
        );
        let r2 = sample_report(
            "crawl-p2",
            Lifecycle::Finished,
            Verdict::Verified,
            Completeness::Complete,
            5,
            250,
        );

        let usage = sum_usage([&r0.usage, &r1.usage, &r2.usage]);
        assert_eq!(usage.requests, 25);
        assert_eq!(usage.proxy_bytes, 1750);
        assert_eq!(usage.browser_time, Duration::from_millis(750));

        let agg = aggregate_reports(&RunId::parse("crawl-all").unwrap(), &[r0, r1, r2]);
        assert_eq!(agg.run_id.as_str(), "crawl-all");
        assert_eq!(agg.lifecycle, Lifecycle::Stopped);
        assert_eq!(agg.stop_reason, Some(StopReason::Pause));
        assert_eq!(agg.verdict, Verdict::NeedsReview);
        assert_eq!(agg.completeness, Completeness::Unverified);
        assert_eq!(agg.usage.requests, 25);
    }
}
