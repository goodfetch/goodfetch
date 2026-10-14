# 07. Resume a killed crawl and retry its failures

## Task statement

> Run the crawl of scenario 05 under a name you choose. Kill the process part-way through. Start the program again with the same name: the crawl continues where it stopped, and the dataset ends with every book exactly once. Then retry only the requests that failed in that crawl. Run it a third time with nothing left to do: it must say that nothing was done and why, instead of finishing silently. While the crawl runs, the program can print what is in flight and what is waiting, and why.

## Target

`books.toscrape.com`, as in scenario 05.

## Leader

Scrapy 2.19.0 (Python): `JOBDIR`. Crawlee 3.18.2 (TypeScript): persisted storage with `purgeOnStart: false`. Crawlee 4.0.0-rc.1, which continues by default and explains a run that processes nothing, is noted.

## Pains

P17 run state surprises, P18 duplicates on retry, P09 live status, P08 deadlines (shutdown); keeps K05 seeing what happened.

## Failure points

1. Items written just before the kill are written again after the resume.
2. The second start silently does nothing, because every request is already marked as seen.
3. Retrying failures also re-runs requests that succeeded.
4. The saved state is missing or damaged.
5. The program changed between the kill and the resume.
6. The crawl hangs while shutting down.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | | | | |
| Crawlee (TypeScript) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

The SDK variants of J-005 (resume after a kill, with no Record delivered twice); Proving ground families: `catalog`, and incident and recovery.
