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
| Choose which site's request goes next | feature | `crawl.domain_order: round_robin` | no counted work either way: each admission takes the first key of one index of the keys with a request waiting, by requests in flight and then turn at `fewest_in_flight`, its default, and by turn alone at `round_robin`; B1 runs both (its cell `round_robin`) |
| Wait for the domain's Politeness Permit | feature | no `politeness.domain.concurrency`, delay 0, no `politeness.domain.rate`, robots.txt off | 4.938 Politeness file transactions per attempt and 16.474 opens per Permit, against 0 and 0; with `politeness.delegation`, its default, a key no other engine of the user uses takes its Permits from memory, with no transaction on the Politeness file (ADR 0142) |
| Judge the AI-use signals | feature | `site_preferences.respect_use_signals: false`, its default | the page's body is read for them only while they are applied |
| Read the document for a Block cause | feature | `fetch.block_classification: false` | 1 document read by the Block classification rules per page, against 0 |
| Judge the Page check | feature | no `session.page_check`, its default | none without one |
| Sign the page and learn URL rules | feature | `frontier.learning: false` | 1 parse and 1 readability run per page, against 0 and 0: with learning off, a page whose readers need no tree is read in one streaming pass |
| Read the page's links, when the crawl follows them | needed | `extraction.follow: none` | 1 links batch per page: 2 glob compilations with a Scope that names globs, 0 without |
| Check for an exact duplicate | needed | — | no counted work |
| Clean the page up and read its formats | feature | `extraction.formats: {}` | none with no format asked: the demand plan runs no clean-up (ADR 0138); a format asked reads the page's one tree, and `extraction.removal_share: exact` adds one more main-content step for the share of text the clean-up removed; the step tries a page that yields under 500 characters up to four times at `extraction.readability: retry`, its default, against once at `single_attempt`, within one readability run either way |
| Remove the page's ads, trackers and annoyances | feature | `filter_lists.ads`, `.trackers` and `.annoyances` all `false` | 1 rule an element is matched against per page on B4's article site (688 before issue 2191 filed attribute rules by their value), about 0.89 M instructions per page beneath the removal, and the version's generic set built once per loaded version, about 11.1 M instructions (B4, below), against 0 rule lists and 0 tests |
| Keep recent hosts' Clean-up rules built | feature | `filter_lists.host_cache: 0` | 1 rule list built per host the cache misses, 0.063 a page on B4's one-host site (its one host, once), and 689 bytes held per host, 705,536 at its default 1,024 hosts (B4's `many_hosts`), against 1 rule list built per page and 0 bytes |
| Retry a short page's main content | feature | `extraction.readability: single_attempt` | up to four main-content attempts on a page that yields under 500 characters at `retry`, its default, against one; on B4's article site each page yields more, so both make one attempt a page |
| Read the Record's formats and fields | feature | no `extraction.fields`, or `dataset.records: false` | B1's two fields read the page's one tree, or its streaming pass, with selectors compiled once for the engine: no parse or selector compilation of their own |
| Keep the page's Record | feature | `dataset.records: false` | 1 validation, 1 serialization, 1 deep copy and 2 lookups of a value constant for the Run per Record, against no Record |
| Write the Provenance log | feature | `provenance.log: false` | 1 task spawned per Run-trace entry, 17 entries for 16 pages, against no entry |
| Commit the attempt's outcome | needed | — | 1.539 run-file transactions per attempt at `full` (B1's cell `concurrency_16`): the attempt's claim and its outcome; at `batch`, the default, the Run's changes are written in Run-state batches of up to 1,000 attempts |
| Sync the outcome to disk | feature | `run_state.durability: off` | 1.539 syncs per attempt at `full`, against 0.924 at `fast` (B1's cells `concurrency_16` and `durability_fast`); `batch`, the default, syncs one per Run-state batch, `periodic` one per `run_state.commit_interval`, and `off` none, only its "unclean" mark's two per Run and the WAL's checkpoints |

The Filter lists' and the readability rows are counted by B4 (`just counts --family B4`, spec #2164), on its 16-page article site with Foxhound's defaults otherwise. Its Callgrind figures are in the benchmark's own build, so they compare with each other, not with a release build's. The clean-up's removal reads 887,064.563 instructions a page (`F1.clean_up_instructions`), 25.5 % of the 3,477,288.063 it read before issue 2191, against a target of 40 %. That figure leaves out work paid once per engine or per loaded version (ADR 0138): the build of the version's generic set, 11,088,372 instructions a crawl in B4 (`F1.generic_set_instructions`), and the compilation of the selectors a crawl meets, 118,302 (`F1.selector_compilation_instructions`). Counting the generic set's build into B4's 16 pages puts the clean-up at about 1.58 M instructions a page, 45 % of the base: the cut alone does not reach 40 %, and a longer crawl spreads the build over more pages.

## The maximum-throughput combination

A crawl that only fetches pages and follows their links, keeping nothing but the run's state, runs fastest with every per-page feature above switched off, its run state at the default `batch` level and the default shared Politeness, with Delegations. This is scenario 1's Foxhound cell in the comparison matrix (`just compare --scenario raw`):

```json
{
  "fetch": { "tier": "http", "block_classification": false },
  "site_preferences": { "respect_robots_txt": false },
  "politeness": { "domain": { "delay": 0, "concurrency": 16 } },
  "crawl": { "handler": { "concurrency": 16 } },
  "frontier": { "learning": false },
  "provenance": { "log": false },
  "dataset": { "records": false },
  "run_state": { "durability": "batch" }
}
```

with the code-free extraction `{ "formats": {}, "follow": "scope" }`. Set `politeness.domain.concurrency` and `crawl.handler.concurrency` to the requests you want in flight. `run_state.durability: batch` is the default, written out to show the level the combination keeps.

The Politeness is the default: `politeness.sharing: shared` and `politeness.delegation: true`. While no other engine of the user uses a domain, the Run's engine holds that domain's Delegation: it keeps the domain's Permits and state in memory, each Permit with no transaction on the Politeness file, and writes to the file at once what a site asked (a 429, a `Retry-After`, a backoff, a halved ceiling, a new Crawl-delay). Another engine that needs the domain recalls the Delegation, so the domain's limits hold across every Run of the user. `politeness.sharing: run` gives the Run a Politeness of its own instead, which no other Run sees.

The same scenario has a second Foxhound cell beside it, at the catalog's defaults: it sets nothing but the crawl's site and the scenario's pace, `politeness.domain.delay: 0` and `politeness.domain.concurrency: 16`, so robots.txt is respected and the per-page features run as the catalog sets them. Its two rows, `fetch.http_from: browser`, the default, and `learned`, show what the defaults cost against the combination.

What it gives up:

- **robots.txt off and no per-domain delay** press each site as hard as the concurrency allows: crawl only sites that agree to it.
- **`frontier.learning: false`** learns no URL rule, so URLs that differ only by a parameter with no effect are each fetched.
- **`fetch.block_classification: false`** moves no request to the browser for a Challenge and ends none `blocked`, so a page a site blocked can be among the successes.
- **`dataset.records: false`** keeps no Record: `run.records` reads none, and each request's outcome says `records_off`.
- **`provenance.log: false`** writes no Provenance log, so the Run trace cannot say which tier fetched each page.
- **`run_state.durability: batch`** loses at most the last Run-state batch to a crash or a power cut: those attempts are fetched again when the Run resumes. `off` would keep nothing through a crash, and saves only the batch's one sync.

## What the run's state survives

`run_state.durability` sets the run's Durability level: how the Run's state reaches the disk, and so what its run file keeps through a process crash and through a power cut. Each level is a preset of the `run_state.batch.*` keys and `run_state.commit_interval`.

| Level | Writes | Syncs | After a process crash | After a power cut |
|---|---|---|---|---|
| `batch`, the default | the Run's changes in Run-state batches, of 1,000 attempts, 1 s or 8 MiB, whichever comes first | each batch | at most the last batch is lost | at most the last batch is lost |
| `full` | each attempt's claim and outcome as a batch of its own (`synchronous=FULL`, with `fullfsync` on macOS) | each batch | only the attempts in flight are lost | only the attempts in flight are lost |
| `fast` | as `full` | none (`synchronous=NORMAL`) | only the attempts in flight are lost | recent commits may roll back, possibly many, without damaging the run state |
| `periodic` | as `full` | one batch every `run_state.commit_interval` (5 s by default) | only the attempts in flight are lost | at most that interval of work is lost |
| `off` | as `batch` | none (`synchronous=OFF`) | the run file is discarded when the Run is next started or resumed, and the Run starts again from nothing | as after a process crash |

A level unfolds into its preset's keys at the layer that set it: a key set at that layer or a higher one overrides the preset's value, and a lock on the level locks every key of its preset. The keys:

| Key | Default | What it sets |
|---|---|---|
| `run_state.batch.per_attempt` | `false` | each attempt's claim written before its fetch and its outcome before anything is told outside, as its own batch: slowest, and a crash then loses only the attempts in flight |
| `run_state.batch.pages` | 1,000 | a batch is written once this many attempts have ended since the last |
| `run_state.batch.interval` | 1 s | a batch is written at least this often, so a crash loses at most about this much work |
| `run_state.batch.bytes` | 8 MiB | a batch is written once it holds this many bytes of rows, which bounds the memory changes not yet written take |
| `run_state.batch.sync` | `each` | which batches reach the disk before the next starts: `each` survives a power cut, `interval` syncs one batch per `run_state.commit_interval`, `never` survives a process crash only, and `off` syncs nothing, so a crash discards the run file |
| `run_state.commit_interval` | 5 s | how often `run_state.batch.sync: interval` syncs a batch, and an idle Run's last changes |

`run_state.memory.budget`, 64 MiB by default, is the most memory the Run's Frontier, Leases and changes not yet written may hold. Past it, or under memory pressure, state already written leaves memory and is read back from the run file; 0 keeps the Frontier in the run file. A budget of 0 with `run_state.batch.pages`, `.interval` and `.bytes` all `none` is refused, since nothing would ever leave memory.

At every level a clean stop (`run.pause`, an abort, a migration) writes and syncs everything, so a paused Run resumes. The run report's `weakened` names `off`, and each preset key whose value weakens the Run; `config explain` gives each key's value and the layer that set it. The Audit log and the Usage ledger sync at every level.
