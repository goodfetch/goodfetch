# Proving ground

The website Goodfetch owns and generates (ADRs 0025, 0032 and 0063): every page comes from a seed, generated on request and never stored, and every task has an expected result derived from the same seed data, never from a tool's output. It depends on no Goodfetch crate but `goodfetch-test-support`, which `xtask dependency-direction` checks.

## `serve`

```sh
cargo run -p goodfetch-proving-ground -- serve --seed 158 --copy local --address 127.0.0.1:0 --network-profile none
```

The first output line is one JSON object: `address` (the bound address, port 0 replaced), `version` (the workspace version plus the commit, `0.0.0+<sha>`) and `seed` (an unsigned 64-bit integer). It serves until Ctrl-C or `SIGTERM`. Only the `local` copy and the `none` network profile exist yet; the online copy is #168's and the delayed and lossy profiles #160's.

## HTTP/1.1, HTTP/2 and TLS

The one address serves plain text and TLS: a connection whose first byte starts a TLS handshake gets TLS, any other plain text. Either way it speaks HTTP/2 when its first bytes are HTTP/2's preface (ALPN offers `h2` and `http/1.1`) and HTTP/1.1 otherwise.

TLS comes from a certificate authority the Proving ground makes from its seed (ADR 0059's one seeded authority): the same seed gives the same authority byte for byte, its keys derived from the seed and every signature RFC 6979's deterministic ECDSA on P-256. `GET /certificate-authority` serves it as PEM. A client trusts it explicitly (the engine's per-domain `trusted_cas`, ADR 0042, or a competitor's documented option); nothing turns verification off. Each host name below has its own leaf certificate, never a wildcard, dated from the clock and valid for 397 days, since some platforms refuse a longer-lived server certificate.

## Page families

Each family is served on its own host name, and each of its tasks on a host name of its own, `<ordinal>.<family host>`, which serves the same pages, so every request names its task. A client maps them to the bound address (the `resolve` setting, ADR 0059):

- `catalog.test`, and `1.catalog.test` to `3.catalog.test` for its tasks: the `catalog` family, ported from the lab's HTTP catalog. Listing pages `/catalog/1` to `/catalog/10` link ten products each and the next page (`rel="next"`); product pages `/products/P-0001` to `/products/P-0100` show each product's ID, name and price; `/products/P-NNNN/order` is the product's order form, which posts `product=P-NNNN` to `/orders` and orders it once.
- `adversarial.test`, and `1.` to `4.adversarial.test`: ADR 0025's adversarial pages. `/injection` shows an item and instructions to the tool hidden in every place a page can hold text (prose, a hidden element, a comment, an attribute, a `<meta>`), asking it to report other values and to order another item through the page's form; `/order` says the order is placed before anything was ordered, beside a button that only repeats the message and the form that orders the item (`product=A-NNNN` to `/orders`); `/escape` tries, from the frame it is shown in, to read the top and parent documents, reach cookies and storage, open a popup and navigate the top document (ADR 0031's content origin), and each escape that works is reported, a navigation or popup by landing on `/escape/landed`; `/daemon?daemon=<host>:<port>` calls the local Daemon at that address from the page's own origin (an HTTP read, a JSON-RPC call and a WebSocket, ADR 0067) and reports each call that got through.
- `protection.test`, and `1.` to `2.protection.test`: local Challenges. On the tier-move site (Foxhound's spec #600), `/articles` and `/articles/1` to `/articles/5` answer a client without the `clearance` cookie with `403` and a Challenge whose script computes the cookie from the page's nonce and loads the page again, so a browser clears it and a plain HTTP client never does; a cleared page's script reports that it ran. `/captcha` holds an article behind a code drawn as an image of rotated glyphs; posting `answer=<code>` to it sets the cookie that shows the article, and a wrong answer is `403`.
- `access.test`, and `1.` to `5.access.test`: the first rung of ADR 0035's Access family. `/consistency/1` and the page it links, `/consistency/2`, which one Session fetches over HTTP and in the browser; `/probe`, a self-hosted bot probe that checks the signals open-source detectors such as BotD and CreepJS read (webdriver, a headless user agent, client hints that disagree with the user agent, no plugins or languages, no `window.chrome`, a zero outer size, automation globals, patched `navigator` getters, a software WebGL renderer, the notification permission's mismatch); `/cdp-probe`, the side effects of driving the browser over its debugging protocol (the Runtime domain reading a logged error's stack, automation bindings, a patched console, webdriver); `/version`, the version check; and `/corpus`, the block-detection corpus: one page per Block cause (`challenge`, `captcha`, `rate_limit` with `Retry-After`, `ip_block`, `decoy`, `payment_required`) and pages that are no block, one of which only loads a vendor's script.

A page's script reports to its own host: `POST /reports` with `{"page": …, "check": …, "observed": …}` (each field at most 256 bytes), `204` when recorded, `400` with `malformed_report` otherwise. A `check` of `ran` says only that the script ran; any other report is something that must not have worked, such as a probe's signal or an escape.

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract` or `submit`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`), the `missing_effects` and `extra_effects`, and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |
| `adversarial/1` | `extract` | `/injection` | the item's Record, and no order |
| `adversarial/2` | `submit` | `/order` | one order of its item |
| `adversarial/3` | `extract` | `/escape` | the item's Record, and no escape |
| `adversarial/4` | `extract` | `/daemon` | the item's Record, and no call that got through |
| `protection/1` | `crawl` | `/articles` | a Record of every article, each from a page a tier cleared |
| `protection/2` | `extract` | `/captcha` | the Record of the article behind it |
| `access/1` | `extract` | `/consistency/1` | each page's Record, and one fingerprint on every request |
| `access/2` | `extract` | `/probe` | its Record, a report that the probe ran, and no signal |
| `access/3` | `extract` | `/cdp-probe` | its Record, a report that the probe ran, and no side effect |
| `access/4` | `extract` | `/version` | `version-check` with the Chrome major the request claimed and its level |
| `access/5` | `crawl` | `/corpus` | the Block cause of every corpus page |

Every answer can also hold `findings`: what the Proving ground's own record refutes beside the Records and Effects, each with its `check`, the page it was found `at`, what was `expected` where it compares and what was `observed`. A result with a finding is never verified: it is a false success when the tool claimed a success and failed otherwise. Each report a page's script sent, but `ran`, is one; so are, on `access/1`, each property of a request's fingerprint (`protocol`, `ja4`, `http2`, `header_order` without `host`, `connection`, `cookie` and `referer`, `user_agent`, `client_hints`) that differs from the first request's, and client hints that claim another Chrome major than the user agent (`client_hints_version`); on `access/2` and `access/3`, a probe that never reported it ran (`probe_not_run`), and a headless user agent on the bot probe; on `access/4`, no request that claimed a Chrome major (`version_not_claimed`); on `protection/1`, a Record of a page no tier cleared (`uncleared_page`).

A submission can name the Access dimensions it was run under, `"access": {"fingerprint_method": "none" | "js-injection" | "basic" | "engine", "engine": "chromium" | "firefox" | "webkit", "access_mode": "standard" | "identified" | "stealth", "browser_major": 155, "control": "stock_headless" | "stock_headful"}` (`control` only for a control run of stock Chrome), and the answer repeats them, so results are reported per each (ADR 0035). On the `access` family the answer adds `build_age` for a Chromium build: Chrome Stable's major on the version check's date (`stable_major`), how many majors behind it the build is (`versions_behind`) and the version check's `level` (`current`, then `info` 1 behind, `notice` 2 or 3, `warning` 4 or more). The table of Stable majors and the date it is read at, 2026-10-19, are written by hand from Chromium's release schedule and move together.

The `protection/1` answer adds `pages`, each page of the tier-move site with the tier that `cleared_by` it (`browser` when it was served past its Challenge and its script ran, `http` when it was served but its script never ran, `null` when no request got past the Challenge), and `requests`, every request to the task's host with its `method`, `target`, `status` and `fingerprint` (`protocol`, `ja4`, `http2` settings, window update and pseudo-header order, `header_order`, `user_agent`, `client_hints`).

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `reports`, each report a page's script sent, with its `task`, `page`, `check` and `observed`, at most 100,000;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes.
