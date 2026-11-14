---
title: Throughput
description: Every step a crawl takes on each page, which ones a crawl can switch off, what each costs, and the combination that crawls fastest.
products: [foxhound]
---

# Throughput

This guide is for a person who wants a Goodfetch Foxhound crawl to fetch as many pages a second as it can. Each step a crawl takes on a page is either **needed** for the result, or a **feature**. A feature has a setting that switches it off, and switching it off removes its work as well as its effect: nothing of it is read, built or written for any page (ADR 0138). A feature switched off is recorded in the Audit log and listed in the run's `weakened`, and the Run report's warnings say which result it leaves empty.

## The steps of a page

The steps run in this order for each page of a code-free crawl on the `http` tier. Each cost is what the counts benchmark B1 (`just counts --family B1`) counted on its 16-page site with the feature on, against its cell with the feature off; a figure is per page unless it says otherwise.

| Step | Class | Switched off by | Cost on, and off |
|---|---|---|---|
| Fetch the request, follow its redirects and refresh stubs, decode the document | needed | — | one request per page, and one per hop (`fetch.redirect.max`, `fetch.refresh.max_delay` bound them) |
| Check the address against the egress policy | needed | — | no counted work |
| Read the origin's robots.txt and obey it | feature | `site_preferences.respect_robots_txt: false`, with `site_preferences.mark_robots_txt_ignored` off, its default | 4 robots.txt parses per origin and 0.211 reads of the shared copy per request, against 0 and 0 |
| Wait for the domain's Politeness Permit | feature | no `politeness.domain.concurrency`, delay 0, no `politeness.domain.rate`, robots.txt off | 4.938 Politeness file transactions per attempt and 16.474 opens per Permit, against 0 and 0 |
| Judge the AI-use signals | feature | `site_preferences.respect_use_signals: false`, its default | the page's body is read for them only while they are applied |
| Read the document for a Block cause | feature | `fetch.block_classification: false` | 1 document read by the Block classification rules per page, against 0 |
| Judge the Page check | feature | no `session.page_check`, its default | none without one |
| Sign the page and learn URL rules | feature | `frontier.learning: false` | 1 parse and 1 readability run per page, against 0 and 0: with learning off, a page whose readers need no tree is read in one streaming pass |
| Read the page's links, when the crawl follows them | needed | `extraction.follow: none` | 1 links batch per page: 2 glob compilations with a Scope that names globs, 0 without |
| Check for an exact duplicate | needed | — | no counted work |
| Clean the page up and read its formats | feature | `extraction.formats: {}` | none with no format asked: the demand plan runs no clean-up (ADR 0138); a format asked reads the page's one tree, and `extraction.removal_share: exact` adds one more main-content step for the share of text the clean-up removed; the step tries a page that yields under 500 characters up to four times at `extraction.readability: retry`, its default, against once at `single_attempt`, within one readability run either way |
| Read the Record's formats and fields | feature | no `extraction.fields`, or `dataset.records: false` | B1's two fields read the page's one tree, or its streaming pass, with selectors compiled once for the engine: no parse or selector compilation of their own |
| Keep the page's Record | feature | `dataset.records: false` | 1 validation, 1 serialization, 1 deep copy and 2 lookups of a value constant for the Run per Record, against no Record |
| Write the Provenance log | feature | `provenance.log: false` | 1 task spawned per Run-trace entry, 17 entries for 16 pages, against no entry |
| Commit the attempt's outcome | needed | — | 1.539 run-file transactions per attempt (B1's cell `concurrency_16`): the attempt's claim and its outcome, committed in groups across pages |
| Sync the outcome to disk | feature | `run_state.durability: off` | 1.539 syncs per attempt at `full`, its default, against 0.924 at `fast` (B1's cells `concurrency_16` and `durability_fast`); `periodic` syncs one group per `run_state.commit_interval`, and `off` none for its groups, only its "unclean" mark's two per Run and the WAL's checkpoints |

## The maximum-throughput combination

A crawl that only fetches pages and follows their links, keeping nothing but the run's state, runs fastest with every feature above switched off. This is scenario 1's Foxhound cell in the comparison matrix (`just compare --scenario raw`):

```json
{
  "fetch": { "tier": "http", "block_classification": false },
  "site_preferences": { "respect_robots_txt": false },
  "politeness": { "sharing": "run", "domain": { "delay": 0, "concurrency": 16 } },
  "crawl": { "handler": { "concurrency": 16 } },
  "frontier": { "learning": false },
  "provenance": { "log": false },
  "dataset": { "records": false },
  "run_state": { "durability": "off" }
}
```

with the code-free extraction `{ "formats": {}, "follow": "scope" }`. Set `politeness.domain.concurrency` and `crawl.handler.concurrency` to the requests you want in flight.

What it gives up:

- **robots.txt and Politeness off** press each site as hard as the concurrency allows: crawl only sites that agree to it.
- **`frontier.learning: false`** learns no URL rule, so URLs that differ only by a parameter with no effect are each fetched.
- **`fetch.block_classification: false`** moves no request to the browser for a Challenge and ends none `blocked`, so a page a site blocked can be among the successes.
- **`dataset.records: false`** keeps no Record: `run.records` reads none, and each request's outcome says `records_off`.
- **`provenance.log: false`** writes no Provenance log, so the Run trace cannot say which tier fetched each page.
- **`run_state.durability: off`** keeps nothing through a crash: a run file a process crash or a power cut left is discarded when the Run is next started or resumed, and the Run starts again from nothing. A clean stop (`run.pause`, an abort) still leaves a run file the Run resumes from.

## What the run's state survives

`run_state.durability` sets the run's Durability level: what its run file keeps through a process crash and through a power cut. Each level groups the attempts' commits across pages; they differ in when a commit reaches the disk.

| Level | After a process crash | After a power cut | Syncs |
|---|---|---|---|
| `full`, the default | only the attempts in flight are lost | only the attempts in flight are lost | each group that holds an outcome or another write that needs a sync, before anything is told outside |
| `fast` | only the attempts in flight are lost | recent commits may roll back, without damaging the run file | none for its groups; the WAL's checkpoints still sync |
| `periodic` | only the attempts in flight are lost | at most the last `run_state.commit_interval` (5 s by default) of commits is lost | one group per interval, and an idle run's commits once the interval ends |
| `off` | the run file is discarded when the Run is next started or resumed, and the Run starts again from nothing | as after a process crash | none for its groups: its "unclean" mark's two per Run and the WAL's checkpoints |

At every level a clean stop (`run.pause`, an abort, a migration) leaves a consistent run file and Checkpoint, so a paused Run resumes. A level below `full` is named in the run report's `weakened` and in `config explain`. A Record a reader has read can be lost to a power cut below `full`, at `periodic` only from the last interval. The Audit log and the Usage ledger sync at every level.
