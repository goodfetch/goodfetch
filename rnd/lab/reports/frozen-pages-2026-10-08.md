# Frozen real pages (T1): rendering by Chrome and Lightpanda, October 8, 2026

Retained run: `extraction-rendered-test-2026-10-08T20-14-39-618Z-9f5f67`, in `local-dev` on committed sources, Mac in use. The 511 real pages of the WCXB test set (CC-BY-4.0) are loaded by Chrome and by Lightpanda, then their rendered DOM goes to rs-trafilatura. The result is compared with extraction on the raw HTML and with the WCXB ground truth.

## How the pages stay frozen

- **The archive proxy** (`fixtures/replay-proxy.ts`) serves each page at its original host and path. It refuses any other request and counts it: subresource missing from the archive, HTTPS tunnel to a non-archived host.
- **Preloaded HSTS hosts.** Chrome always loads the hosts in its HSTS preload list over HTTPS. The proxy therefore terminates TLS itself for the archived hosts, with the lab's local certificate, which Chrome is instructed to accept.
- **Chrome** sends everything through this proxy (`--proxy-server`, with no loopback exception). It cannot resolve any other host (`--host-resolver-rules`) and does not switch to HTTPS on its own.
- **Lightpanda** goes through `--http-proxy` and blocks every address except loopback (`--block-cidrs`).

Both engines therefore see the same bytes, and no request leaves the machine.

## Results

| Engine | Pages rendered | F1 (mean per page) | Required snippets found | Leaking boilerplate | Pages served by the archive | Refused requests | Refused tunnels | Median load (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| none (raw HTML) | 511/511 | 0.884 | 0.858 | 0.099 | — | — | — | — |
| Chrome | 511/511 | 0.884 | 0.858 | 0.099 | 511 | 8181 | 5413 | 286 |
| Lightpanda | 511/511 | 0.884 | 0.858 | 0.099 | 511 | 319 | 130 | 211 |

- **Rendering changes nothing for extraction on these pages.** The 1,022 texts extracted after rendering are identical, page by page, to those from the raw HTML. The DOM serialization differs, however: Chrome produces fewer bytes than the archive. The WCXB pages are snapshots taken after rendering, and without a network their scripts add no main content.
- **Lightpanda renders all 511 pages**, without failure or timeout. It requests almost no subresources (319 refused requests versus 8,181 for Chrome), which partly explains a shorter load. The durations come from a Mac in use and remain indicative.
- **What this says for the decisions:**
  - `D-light-engine`: on real static pages, Lightpanda reads as much as Chrome. Its limits identified on the fixtures remain valid: false success under the guard, no frames, popups or downloads.
  - `D-extraction`: browser rendering is not necessary to extract the main content of an already-rendered page. It remains necessary for a page that builds its content in JavaScript, which this corpus does not contain.

## Harness defects found along the way

The affected runs are classified `diagnostic`.

- **Page without a URL.** One page of the corpus has no URL; it made the whole "article" cell fail. It is served on a reserved host (`<id>.wcxb.invalid`).
- **Chrome's HTTPS upgrade.** Chrome switched to HTTPS on its own on HTTP navigations (HTTPS-Upgrades), and loaded the 47 pages of preloaded HSTS hosts over HTTPS. The first feature is disabled; the proxy now serves HTTPS for the archived hosts.
- **Overwritten predictions.** The prediction files of the rendered cells had the same name as those of the raw HTML and overwrote each other. They are now named by engine.

## Limitations

- The WCXB pages contain only the document: no subresource is archived. A real Web Page Replay archive, with scripts, styles and data requests, is necessary to measure pages that build themselves in JavaScript.
- The rerun in `local-controlled` is recorded in the ledger for the durations.
