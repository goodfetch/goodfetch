# 03. Repeat AI steps with no model call

## Task statement

> On https://quotes.toscrape.com/, using steps written in plain words rather than selectors: open the tag "love" from the tag list, go to its second page, and open the "about" page of the first quote's author. Before returning, confirm without a model that the page reached is an author page (its address starts with https://quotes.toscrape.com/author/). Return the author's name, birth date and birthplace. Keep what the model resolved for each step in a file next to the program. A second run must repeat the same steps with no model call at all. If the site changes so that one kept step no longer applies, only that step goes back to the model, and the program says which step that was.

## Target

`quotes.toscrape.com`: the tag cloud, `/tag/love/page/2/` and author pages such as `/author/Albert-Einstein/` with their birth date and place (checked 2026-10-14).

## Leader

Stagehand 4.2.1, `act` with its cache (TypeScript). Stagehand 4's cache is kept on Browserbase's server and has no effect locally; Stagehand 3.7.3 kept a local cache folder, noted in the comparison.

## Pains

P03 replay without the model, P26 local-first artifacts, P04 verified rather than claimed, P36 reviewable output; keeps K02 generate once, then own the deterministic artifact.

## Failure points

1. A step resolves to the wrong element but the run continues (a plausible wrong result).
2. A kept step no longer matches the page (Drift).
3. The model is unavailable on the first run.
4. A kept step drifts on a later run that has no model configured.
5. The end state is not reached, and the confirmation fails.
6. The kept file comes from an older version of the library.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand (TypeScript) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces; Proving ground families: AI, and repeated runs with Drift.
