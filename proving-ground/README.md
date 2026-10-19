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
- `rendered.test`, and `1.rendered.test` and `2.rendered.test` for its tasks: the `rendered` family, ADR 0025's AJAX loading, infinite scroll, shadow DOM and iframes. `/feed`'s script loads its 24 items from `/api/feed/1` to `/api/feed/4` by AJAX, the next page each time the end of the list comes into view. Each item page `/items/I-0001` to `/items/I-0024` loads its item from `/api/items/<id>`, puts the name in the open shadow root of its `item-card` element and the price in the page, and shows the description in a same-origin frame, `/items/<id>/details`. None of a Record's data is in a page's own HTML.
- `login.test`, and `1.login.test` and `2.login.test` for its tasks: the `login` family, ADR 0025's logins with MFA and logins that expire. `/login` takes a username and password; account 1 then asks for a TOTP code (RFC 6238: HMAC-SHA-1, 30-second steps, six digits) at `/mfa`, accepting the steps before and after the current one; account 2's login expires after three pages behind it. `/account` lists six orders, `/account/orders/O-01` to `O-06`; without a live session both send to `/login` (`303`). The accounts come from the seed, so each seed (and each Journey) has its own, and the task list names them.
- `schedule.test`, and `1.schedule.test` for its task: the `schedule` family, ADR 0025's content that changes on a known schedule. `/board` shows the revision of the current time step: it changes at every step but those that leave a remainder of 2 when divided by 3 (`R-0` at step 0, `R-1` at 1 and 2, `R-2` at 3, `R-3` at 4 and 5, and so on). The board at a step is a pure function of the seed and the step, never of the wall clock.

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract`, `submit` or `watch`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. A task behind a login also has `account` (`username`, `password`, and `totp_secret` in unpadded base32 for an account with MFA); a watch task has `time_steps`, the count of steps from 0 it spans. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`), the `missing_effects` and `extra_effects`, and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.
- `PUT /time-step` sets the time step, `{"step": 3}`, which starts at 0, and answers the step it set. Whoever runs a watch task sets each step in turn; a step that is not a whole number answers `400` with `malformed_time_step`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |
| `rendered/1` | `crawl` | `/feed` | every item, with `id`, `name`, `price` and `description` |
| `rendered/2` | `extract` | one item page, chosen by the seed | that item |
| `login/1` | `crawl` | `/login`, as account 1 (MFA) | its six orders, with `id`, `item` and `total` |
| `login/2` | `crawl` | `/login`, as account 2 (login that expires) | its six orders |
| `schedule/1` | `watch` | `/board`, over time steps 0 to 5 | revisions `R-0` to `R-3`, with `id`, `headline` and `price` |

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes.
