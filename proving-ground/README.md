# Proving ground

The website Goodfetch owns and generates (ADRs 0025, 0032 and 0063): every page comes from a seed, generated on request and never stored, and every task has an expected result derived from the same seed data, never from a tool's output. It depends on no Goodfetch crate but `goodfetch-test-support`, which `xtask dependency-direction` checks.

## `serve`

```sh
cargo run -p goodfetch-proving-ground -- serve --seed 158 --copy local --address 127.0.0.1:0 --network-profile none
```

The first output line is one JSON object: `address` (the bound address, port 0 replaced), `version` (the workspace version plus the commit, `0.0.0+<sha>`), `seed` (an unsigned 64-bit integer), and the address of each [network fixture](#network-fixtures): `proxies` (each forward proxy's address by name), `proxy_credentials` (`username` and `password`, derived from the seed), `dns` and `target`. It serves until Ctrl-C or `SIGTERM`. Only the `local` copy and the `none` network profile exist yet; the online copy is #168's and the delayed and lossy profiles #160's.

## HTTP/1.1, HTTP/2 and TLS

The one address serves plain text and TLS: a connection whose first byte starts a TLS handshake gets TLS, any other plain text. Either way it speaks HTTP/2 when its first bytes are HTTP/2's preface (ALPN offers `h2` and `http/1.1`) and HTTP/1.1 otherwise.

TLS comes from a certificate authority the Proving ground makes from its seed (ADR 0059's one seeded authority): the same seed gives the same authority byte for byte, its keys derived from the seed and every signature RFC 6979's deterministic ECDSA on P-256. `GET /certificate-authority` serves it as PEM. A client trusts it explicitly (the engine's per-domain `trusted_cas`, ADR 0042, or a competitor's documented option); nothing turns verification off. Each host name below has its own leaf certificate, never a wildcard, dated from the clock and valid for 397 days, since some platforms refuse a longer-lived server certificate.

## Page families

Each family is served on its own host name, and each of its tasks on a host name of its own, `<ordinal>.<family host>`, which serves the same pages, so every request names its task. A client maps them to the bound address (the `resolve` setting, ADR 0059):

- `catalog.test`, and `1.catalog.test` to `3.catalog.test` for its tasks: the `catalog` family, ported from the lab's HTTP catalog. Listing pages `/catalog/1` to `/catalog/10` link ten products each and the next page (`rel="next"`); product pages `/products/P-0001` to `/products/P-0100` show each product's ID, name and price; `/products/P-NNNN/order` is the product's order form, which posts `product=P-NNNN` to `/orders` and orders it once.

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract` or `submit`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`), the `missing_effects` and `extra_effects`, and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes;
- `connections`, the connection record: every connection a listener of the Proving ground accepted, the site's own address included, each with its `id` (a request's `connection`), `time_step`, `at_microseconds`, `listener` (`site`, `target` or a proxy's name), `source`, `destination`, and `relay`, the relay that opened it when a forward proxy did. A test asserts that no connection reached a target from it (ADR 0067);
- `relays`, every request a forward proxy received, each with its `id`, `time_step`, `at_microseconds`, `proxy`, `kind`, `connection` (the client's connection to the proxy), `method`, `target` as sent, `task` (of the target's host), `username` (never the password), `sticky` (`provider`, `session`, `country` and `lifetime_minutes` its username asked for), `exit`, `destination` (the address it resolved to), `source` (the proxy's side of the connection it opened), `outcome` (`relayed`, `proxy_authentication_required`, `tunnel_refused`, `malformed`, `unresolved`, `destination_prohibited` or `connection_refused`) and `bytes` (`received` from the client and `sent` to it, the proxy's own answer included);
- `unrecorded_connections` and `unrecorded_relays`, those past the first 100,000 of each.

`PUT /time-step` with `{"step": n}` sets the time step the connection record stamps each later entry with (it starts at 0); an unreadable body answers `400` with `malformed_time_step`.

## Network fixtures

Each listens on its own port of the site's address (loopback when the site binds every address), reported in `serve`'s first line, so proxies, rebinding and SSRF refusals are tested for real, never mocked (ADRs 0059 and 0067):

- **Forward proxies.** Each relays one request per client connection: plain HTTP in absolute form (`GET http://catalog.test:<port>/… HTTP/1.1`), rewritten to origin form with `Connection: close`, or a `CONNECT host:port` tunnel. A proxy resolves a host as the DNS fixture answers it, so a Proving ground host name reaches the site and its requests are observed for their task, and it connects only to this host's loopback or the site's address: another address is refused with `403` (`destination_prohibited`), an unknown name with `502` (`unresolved`), an address where nothing listens with `502` (`connection_refused`). Its own refusals name their cause in `Proxy-Status` (RFC 9209). The proxies are:
  - `open`, without authentication;
  - `refusing_tunnels`, without authentication, which answers `CONNECT` with `403` and still relays plain HTTP;
  - `down`, an address where nothing listens, so a connection to it is refused;
  - `datacenter`, `isp`, `residential` and `mobile`, one per proxy kind, each taking Basic credentials (`407` with `Proxy-Authenticate: Basic realm="proving-ground"` without them) and relaying through its own pool of three exits, `<kind>-1` to `<kind>-3`. Each relay takes the next exit, except that a sticky session keeps the exit its first relay took. The username is the reported one, or a provider adapter's sticky username built on it: `bright_data`'s `brd-customer-<username>-zone-<zone>-session-<id>` (with `-country-<cc>`), `oxylabs`'s `customer-<username>-sessid-<id>` (with `-cc-<CC>` and `-sesstime-<minutes>`), or `apify`'s `session-<id>,country-<CC>` pairs (whose username names no account, so only the password is checked). The relay records the session, country and lifetime it asked for.
- **DNS fixture**, over UDP. Every answer has a TTL of 0. The host names the Proving ground serves resolve to its address. A name under `alternating.dns.test` answers the addresses its labels declare in turn, one per query of each record type: `93-184-215-14.127-0-0-1.alternating.dns.test` answers `93.184.215.14`, then `127.0.0.1`, then `93.184.215.14` again. A name under `mixed.dns.test` answers all of them at once. An IPv4 label is its numbers joined by `-`, an IPv6 label its 32 hexadecimal digits, and an A query answers only the IPv4 ones, an AAAA query the IPv6 ones. Any other name is `NXDOMAIN`.
- **Target**, an address no test should reach: whatever reaches it gets `200` with `reached`, and the connection record lists every connection to it under the listener `target`.
