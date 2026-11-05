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
| Read the origin's robots.txt and obey it | feature | `site_preferences.respect_robots_txt: false` | 4 robots.txt parses per origin and 0.211 reads of the shared copy per request, against 0 and 0 |
| Wait for the domain's Politeness Permit | feature | no `politeness.domain.concurrency`, delay 0, no `politeness.domain.rate`, robots.txt off | 4.938 Politeness file transactions per attempt and 16.474 opens per Permit, against 0 and 0 |
| Judge the AI-use signals | feature | `site_preferences.respect_use_signals: false`, its default | the page's body is read for them only while they are applied |
| Read the document for a Block cause | feature | `fetch.block_classification: false` | 1 document read by the Block classification rules per page, against 0 |
| Judge the Page check | feature | no `session.page_check`, its default | none without one |
| Sign the page and learn URL rules | feature | `frontier.learning: false` | 7 parses, 11 selector compilations and 1 readability run per page, against 4, 6 and 0 |
| Read the page's links, when the crawl follows them | needed | `extraction.follow: none` | 1 links batch per page: 2 glob compilations with a Scope that names globs, 0 without |
| Check for an exact duplicate | needed | — | no counted work |
| Clean the page up and read its formats | feature | `extraction.formats: {}` | none with no format asked: the demand plan runs no clean-up (ADR 0138) |
| Read the Record's formats and fields | feature | no `extraction.fields`, or `dataset.records: false` | with B1's two fields, 1 parse and 2 selector compilations per page: 7 and 11, against 6 and 9 with `dataset.records` off |
| Keep the page's Record | feature | `dataset.records: false` | 1 validation, 1 serialization, 1 deep copy and 2 lookups of a value constant for the Run per Record, against no Record |
| Write the Provenance log | feature | `provenance.log: false` | 1 task spawned per Run-trace entry, 17 entries for 16 pages, against no entry |
| Commit the attempt's outcome | needed | — | 5.375 run-file transactions per attempt |
| Sync the outcome to disk | feature | `run_state.durability: fast` | 4.616 syncs per attempt at `full`, its default, against 1.924 at `fast` (B1's cells `concurrency_16` and `durability_fast`) |

The durable levels `periodic` and `off` come with #1927.

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
  "run_state": { "durability": "fast" }
}
```

with the code-free extraction `{ "formats": {}, "follow": "scope" }`. Set `politeness.domain.concurrency` and `crawl.handler.concurrency` to the requests you want in flight.

What it gives up:

- **robots.txt and Politeness off** press each site as hard as the concurrency allows: crawl only sites that agree to it.
- **`frontier.learning: false`** learns no URL rule, so URLs that differ only by a parameter with no effect are each fetched.
- **`fetch.block_classification: false`** moves no request to the browser for a Challenge and ends none `blocked`, so a page a site blocked can be among the successes.
- **`dataset.records: false`** keeps no Record: `run.records` reads none, and each request's outcome says `records_off`.
- **`provenance.log: false`** writes no Provenance log, so the Run trace cannot say which tier fetched each page.
- **`run_state.durability: fast`** can lose the last commits to a power cut, never to a crash of the process alone.
