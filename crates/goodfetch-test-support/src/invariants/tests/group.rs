//! The checkers of Daemon groups and Partitioned crawls: one lead,
//! Partitions, the Audit log's chain and bounded liveness.

use rstest::rstest;

use super::log;
use crate::invariants::{Violation, ViolationKind, audit_chain, liveness, one_lead, partitions};
use crate::observation::{Epoch, Operation, Record};

fn lead(host: &str, epoch: Epoch) -> Record<()> {
    Record::new(host, Operation::Lead { epoch })
}

fn fired(host: &str, occurrence: &str) -> Record<()> {
    Record::new(
        host,
        Operation::Fired {
            schedule: "nightly".to_owned(),
            occurrence: occurrence.to_owned(),
        },
    )
}

/// `a` leads under epoch 1 and fires one occurrence; `b` is promoted under
/// epoch 2 and fires the next; `a`, demoted, leads again under epoch 3.
fn lead_handed_over() -> Vec<Record<()>> {
    vec![
        lead("a", 1),
        fired("a", "2026-01-05T00:00:00Z"),
        lead("a", 1),
        lead("b", 2),
        fired("b", "2026-01-06T00:00:00Z"),
        lead("a", 3),
    ]
}

#[test]
fn one_lead_passes_a_lead_handed_over_under_a_greater_epoch() {
    let verdict = one_lead(&log(&lead_handed_over()));

    assert_eq!(verdict, Ok(()));
}

#[rstest]
#[case::two_leads_in_one_epoch(
    vec![lead("a", 1), lead("b", 2), lead("c", 2)],
    ViolationKind::TwoLeads,
    vec![lead("b", 2), lead("c", 2)],
)]
#[case::a_demoted_lead_acting_under_its_old_epoch(
    vec![lead("a", 1), lead("b", 2), lead("a", 1)],
    ViolationKind::StaleLead,
    vec![lead("b", 2), lead("a", 1)],
)]
#[case::a_schedule_firing_twice_for_one_occurrence(
    vec![lead("a", 1), fired("a", "2026-01-05T00:00:00Z"), lead("b", 2), fired("b", "2026-01-05T00:00:00Z")],
    ViolationKind::ScheduleFiredTwice,
    vec![fired("a", "2026-01-05T00:00:00Z"), fired("b", "2026-01-05T00:00:00Z")],
)]
fn one_lead_fails_on_two_daemons_acting_as_lead(
    #[case] records: Vec<Record<()>>,
    #[case] kind: ViolationKind,
    #[case] shown: Vec<Record<()>>,
) {
    let verdict = one_lead(&log(&records));

    assert_eq!(
        verdict,
        Err(Violation {
            kind,
            records: shown
        })
    );
}

fn discovered(url: &str) -> Record<()> {
    Record::new(
        "p1",
        Operation::Discovered {
            url: url.to_owned(),
        },
    )
}

fn frontier(partition: &str, url: &str) -> Record<()> {
    Record::new(
        partition,
        Operation::Frontier {
            partition: partition.to_owned(),
            domain: domain(url).to_owned(),
            url: url.to_owned(),
        },
    )
}

fn admitted(partition: &str, domain: &str) -> Record<()> {
    Record::new(
        partition,
        Operation::Admitted {
            partition: partition.to_owned(),
            domain: domain.to_owned(),
        },
    )
}

fn handed_off(partition: &str, domain: &str) -> Record<()> {
    Record::new(
        partition,
        Operation::HandedOff {
            partition: partition.to_owned(),
            domain: domain.to_owned(),
        },
    )
}

/// The domain of the test's URLs, `https://<domain>/<path>`.
fn domain(url: &str) -> &str {
    url.trim_start_matches("https://")
        .split('/')
        .next()
        .expect("invariant: a split yields one part")
}

/// Partition `p1` crawls `a.example`, discovers a URL of `b.example`, which
/// `p2` takes; then `p1` hands `a.example` off to `p2`, with its URLs.
fn domain_handed_off() -> Vec<Record<()>> {
    vec![
        discovered("https://a.example/"),
        frontier("p1", "https://a.example/"),
        admitted("p1", "a.example"),
        discovered("https://b.example/"),
        frontier("p2", "https://b.example/"),
        admitted("p2", "b.example"),
        handed_off("p1", "a.example"),
        frontier("p2", "https://a.example/"),
        admitted("p2", "a.example"),
    ]
}

#[test]
fn partitions_pass_a_domain_handed_off_with_its_urls() {
    let verdict = partitions(&log(&domain_handed_off()));

    assert_eq!(verdict, Ok(()));
}

#[rstest]
#[case::a_url_in_two_frontiers_at_once(
    vec![discovered("https://a.example/"), frontier("p1", "https://a.example/"), frontier("p2", "https://a.example/")],
    ViolationKind::UrlInTwoFrontiers,
    vec![frontier("p1", "https://a.example/"), frontier("p2", "https://a.example/")],
)]
#[case::a_url_in_no_frontier(
    vec![discovered("https://a.example/"), frontier("p1", "https://a.example/"), discovered("https://b.example/")],
    ViolationKind::UrlLost,
    vec![discovered("https://b.example/")],
)]
#[case::a_url_handed_off_to_no_frontier(
    vec![discovered("https://a.example/"), frontier("p1", "https://a.example/"), handed_off("p1", "a.example")],
    ViolationKind::UrlLost,
    vec![discovered("https://a.example/")],
)]
#[case::a_domain_held_by_two_partitions(
    vec![admitted("p1", "a.example"), admitted("p2", "a.example")],
    ViolationKind::DomainHeldTwice,
    vec![admitted("p1", "a.example"), admitted("p2", "a.example")],
)]
fn partitions_fail_when_a_url_or_a_domain_is_not_in_exactly_one_partition(
    #[case] records: Vec<Record<()>>,
    #[case] kind: ViolationKind,
    #[case] shown: Vec<Record<()>>,
) {
    let verdict = partitions(&log(&records));

    assert_eq!(
        verdict,
        Err(Violation {
            kind,
            records: shown
        })
    );
}

/// The product's chain verifier stands in here as a stub, since test
/// support depends on no Goodfetch crate: a chain verifies unless one of its
/// entries says it was tampered with.
fn stub_verify(entries: &[&str]) -> bool {
    entries.iter().all(|entry| !entry.contains("tampered"))
}

fn audit(host: &str, hash: &str, previous: &str, merges: Option<&str>) -> Record<()> {
    Record::new(
        host,
        Operation::Audit {
            hash: hash.to_owned(),
            previous: previous.to_owned(),
            entry: format!("entry {hash}"),
            merges: merges.map(str::to_owned),
        },
    )
}

fn tampered(record: Record<()>) -> Record<()> {
    let Operation::Audit {
        hash,
        previous,
        merges,
        ..
    } = record.operation
    else {
        panic!("not an Audit entry: {record:?}");
    };
    Record::new(
        record.host,
        Operation::Audit {
            entry: format!("entry {hash}, tampered"),
            hash,
            previous,
            merges,
        },
    )
}

/// The lead chains `h1` and `h2`; Group member `m`, cut off, chains `m3`
/// from `h2`, the last lead hash it saw, while the lead chains `h3`; on its
/// return the lead appends `h4`, which merges `m3`.
fn branch_merged() -> Vec<Record<()>> {
    vec![
        audit("lead", "h1", "0", None),
        audit("lead", "h2", "h1", None),
        audit("m", "m3", "h2", None),
        audit("lead", "h3", "h2", None),
        audit("lead", "h4", "h3", Some("m3")),
    ]
}

#[test]
fn audit_chain_passes_a_branch_the_lead_merged() {
    let verdict = audit_chain(&log(&branch_merged()), stub_verify);

    assert_eq!(verdict, Ok(()));
}

#[rstest]
#[case::an_entry_the_verifier_refuses(
    vec![audit("lead", "h1", "0", None), tampered(audit("lead", "h2", "h1", None)), audit("lead", "h3", "h2", None)],
    ViolationKind::AuditChainBroken,
    vec![audit("lead", "h1", "0", None), tampered(audit("lead", "h2", "h1", None))],
)]
#[case::a_branch_entry_the_verifier_refuses(
    vec![
        audit("lead", "h1", "0", None),
        tampered(audit("m", "m2", "h1", None)),
        audit("lead", "h2", "h1", None),
        audit("lead", "h3", "h2", Some("m2")),
    ],
    ViolationKind::AuditChainBroken,
    vec![audit("lead", "h1", "0", None), tampered(audit("m", "m2", "h1", None))],
)]
#[case::an_entry_chained_to_no_entry_of_the_log(
    vec![audit("lead", "h1", "0", None), audit("lead", "h3", "h2", None)],
    ViolationKind::AuditChainBroken,
    vec![audit("lead", "h3", "h2", None)],
)]
#[case::a_merge_of_no_entry_of_the_log(
    vec![audit("lead", "h1", "0", None), audit("lead", "h2", "h1", Some("m9"))],
    ViolationKind::AuditChainBroken,
    vec![audit("lead", "h2", "h1", Some("m9"))],
)]
#[case::a_branch_the_lead_never_merged(
    vec![audit("lead", "h1", "0", None), audit("m", "m2", "h1", None), audit("lead", "h2", "h1", None)],
    ViolationKind::AuditBranchUnmerged,
    vec![audit("m", "m2", "h1", None), audit("lead", "h2", "h1", None)],
)]
#[case::a_branch_extended_past_its_merge(
    with_more(branch_merged(), audit("m", "m4", "m3", None)),
    ViolationKind::AuditBranchUnmerged,
    vec![audit("lead", "h4", "h3", Some("m3")), audit("m", "m4", "m3", None)],
)]
fn audit_chain_fails_on_a_broken_or_unmerged_branch(
    #[case] records: Vec<Record<()>>,
    #[case] kind: ViolationKind,
    #[case] shown: Vec<Record<()>>,
) {
    let verdict = audit_chain(&log(&records), stub_verify);

    assert_eq!(
        verdict,
        Err(Violation {
            kind,
            records: shown
        })
    );
}

fn with_more(mut records: Vec<Record<()>>, record: Record<()>) -> Vec<Record<()>> {
    records.push(record);
    records
}

fn started(run: &str, at: u64) -> Record<()> {
    Record::new(
        "m",
        Operation::Started {
            run: run.to_owned(),
        },
    )
    .at(at)
}

fn finished(run: &str, at: u64) -> Record<()> {
    Record::new(
        "m",
        Operation::Finished {
            run: run.to_owned(),
        },
    )
    .at(at)
}

fn resumable(run: &str, at: u64) -> Record<()> {
    Record::new(
        "m",
        Operation::Resumable {
            run: run.to_owned(),
        },
    )
    .at(at)
}

/// Faults stop at 1,000 ms; `r1`, started before, finishes by 1,500 ms;
/// `r2`, started at 4,000 ms, is checkpointed 500 ms later; `r3` finishes,
/// is resumed and finishes again in time.
fn runs_settled() -> Vec<Record<()>> {
    vec![
        started("r1", 0),
        started("r3", 10),
        finished("r3", 20),
        started("r3", 1_200),
        finished("r1", 1_500),
        finished("r3", 1_900),
        started("r2", 4_000),
        resumable("r2", 4_500),
    ]
}

#[test]
fn liveness_passes_runs_that_finish_or_are_resumable_in_time() {
    let verdict = liveness(&log(&runs_settled()), 1_000, 1_000);

    assert_eq!(verdict, Ok(()));
}

#[rstest]
#[case::a_run_finishing_past_its_bound(
    vec![started("r1", 0), finished("r1", 2_001)],
    vec![started("r1", 0), finished("r1", 2_001)],
)]
#[case::a_run_never_finishing(
    vec![started("r1", 0), started("r2", 0), finished("r1", 10)],
    vec![started("r2", 0)],
)]
#[case::a_run_resumed_and_never_finishing_again(
    vec![started("r1", 0), finished("r1", 10), started("r1", 1_500), finished("r2", 5_000)],
    vec![started("r1", 1_500)],
)]
fn liveness_fails_when_a_run_makes_no_progress_past_its_bound(
    #[case] records: Vec<Record<()>>,
    #[case] shown: Vec<Record<()>>,
) {
    let verdict = liveness(&log(&records), 1_000, 1_000);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::ProgressMissing,
            records: shown
        })
    );
}
