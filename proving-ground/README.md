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

The Crawl families (ADR 0025's Crawl cases, and ADR 0005's parameter that looks like it has no effect) keep, per task host, what they saw a crawl do, and record each misstep as a finding (see Observations); the rules that depend on the client's address read the address of the connection:

- `statuses.test` (`statuses/1`): an index of 40 articles, 30 linked directly, 5 through one redirect each (`301`, `302`, `303`, `307`, `308`) and 5 through a chain of four; a link to every HTTP error status IANA registers but the unused 418, `402` included, each answering its status with an error page in the articles' template (and the headers the status requires); soft 404s, `200` pages in that template that say the article does not exist (every unknown article number is one); and three redirect loops (`/loops/a` and `/loops/b`, `/loops/self`, and `/loops/endless/<n>`). A task may make 100 requests to the loops; each one past it is a `trap` finding.
- `traps.test` (`traps/1`): a shop of 30 items in three categories, each item linked at two URLs that the fixed URL rules (ADR 0120) make one: a fragment, `utm_*`, `gclid`, `fbclid`, `;jsessionid=`, a percent-encoded `-`, or parameters in another order. A page fetched again under any of its URLs is a `duplicate` finding. Two traps page on without end: the events calendar `/calendar/<yyyy-mm>` and the archive's relative `more/` link; a task may make 30 requests to them together, and each one past it is a `trap` finding. Each page carries four honeypot links under `/hp/`, hidden by `hidden`, `display:none`, `visibility:hidden` and an `aria-hidden` element off screen, and disallowed by `/robots.txt`: a request for one is a `honeypot` finding and flags the address that made it, refused with `403` on that task's host from then on.
- `parameters.test` (`parameters/1`): four listing pages `/products?page=<n>` of twelve products, each linked in three colours, `/product?id=P-NNNN&colour=<1..3>&ref=listing`. `ref` never changes the page, and neither does `colour` on the first listing page, whose products come in one colour (a page shows a product's first colour for one it lacks); further on, some products have a distinct page per colour, with its own ID `P-NNNN-<colour>` and price. A crawl that learned `colour` has no effect loses those pages.
- `limits.test` (`limits/1` to `limits/3`): one list of 24 items behind a rule per task. `limits/1` lets each address make 10 requests, then answers `429` with `Retry-After: 1` and pauses it for that second; a request in the pause is a `retry_after_ignored` finding, and the third flags the address. `limits/2` lets each address fetch 10 pages and flags the one that asks for more, so a complete crawl needs more than one address. `limits/3` holds each page 50 ms and allows 2 requests in flight on its host at once; one more is answered `503` with `Retry-After: 1` and is an `over_in_flight_cap` finding.
- `scale.test` (`scale/1`, `scale/2`): 4,194,304 item pages `/items/1` to `/items/4194304`, generated on request, in sections of 256 listed by `/sections/<n>`; each section has a sitemap index `/sections/<n>/sitemap.xml` of two sitemaps, and `/sitemap.xml` indexes 128 sitemaps of 32,768 items each. Sitemaps name absolute `http://` URLs on the host and port the request named. `measurements/scale/` holds the declared measurement of how fast the family is served beside how fast a competitor consumes it (`node proving-ground/measurements/scale/measure.ts --proving-ground <release binary> --crawlee <node_modules with crawlee>`), and its runs in `runs/`. Its first run, on 2026-10-19, met its threshold (the Proving ground's slowest trial served 542 pages a second, Crawlee's fastest consumed 125) on a host far past the `local-controlled` gates, with a one-minute load of 553 on 14 CPUs; it is to be run again on a quiet host.

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract` or `submit`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`), the `missing_effects` and `extra_effects`, the `findings` the Proving ground recorded for the task, and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. A task with any finding is never `verified`: a false success when the tool claimed success, `failed` otherwise. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |
| `statuses/1` | `crawl` | `/` | a Record (`id`, `title`) of every article, none of an error page or a soft 404 |
| `traps/1` | `crawl` | `/` | a Record (`id`, `name`) of every item, and no finding |
| `parameters/1` | `crawl` | `/products?page=1` | a Record (`id`, `name`, `price`) of every distinct product page |
| `limits/1` to `limits/3` | `crawl` | `/` | a Record (`id`, `name`) of every item, and no finding |
| `scale/1` | `crawl` | one section page, chosen by the seed | a Record (`id`, `name`) of each of its 256 items |
| `scale/2` | `crawl` | one section's sitemap index, chosen by the seed | a Record (`id`, `name`) of each of its 256 items |

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `findings`, each misstep a Crawl family saw, with its `task`, its `kind` (`duplicate` with the `page`, `trap`, `honeypot`, `retry_after_ignored` or `over_in_flight_cap`) and the `target` asked for; `unrecorded_findings` counts those past the first 10,000;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes.
