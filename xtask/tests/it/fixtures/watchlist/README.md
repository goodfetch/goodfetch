A workspace for `xtask watchlist` (story 61): the coverage file of Collie's
block-detection rules, and the two watchlists in their own layouts,
hand-written with vendor names, categories and descriptions of our own
wording only, so no detection pattern or text of either list (GPL-3.0,
NPOSL-3.0) is copied. Their Security category also holds technologies that
protect from no bots (HSTS, a bug bounty platform), which the job skips:

- `webappanalyzer/`: `src/categories.json` and `src/technologies/*.json`, as
  enthec/webappanalyzer lays them out;
- `antibot-detector/`: `detectors/index.json`, as scrapfly/Antibot-Detector
  lays it out.
