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

- `extraction.test`, and `1.extraction.test` to `6.extraction.test`: the `extraction` family, ADR 0025's Extraction cases on seeded products (J-011). `/products` links product pages carrying schema.org JSON-LD (brand, GTIN, rating, currency) and Open Graph metadata (description); `/shop` loads its products only from its hidden API, `/api/products/1` and `/api/products/2`; `/churn` links pages whose class names differ on every page; `/variants` links pages in six layouts; `/huge` is one table of 2,000 products, over 5 MiB, generated on request; `/<region>/products/X-0100` shows one product in `us`, `de`, `fr`, `jp` and `in`, each in its locale's language, currency and number format, linked by `hreflang` alternates.
- `encodings.test`, `1.encodings.test` and `2.encodings.test`: the `encodings` family (ADRs 0073 and 0119, CO-24). `/pages` links one page per case: Shift_JIS, EUC-KR, GB18030, Big5, windows-1251, ISO-8859-2, ISO-8859-15, UTF-16 with a BOM, a charset that contradicts its `<meta>`, no declared charset in UTF-8 and in a legacy encoding, the `latin1` and `ascii` labels with byte 0x80, a `<meta>` UTF-16 label, `x-user-defined`, a `<meta charset>` past the first 1,024 bytes among head tags and one after the body tag, and a byte UTF-8 cannot decode. Each page's expected text is held in UTF-8, with the encoding ADR 0119's order reads it with and where that comes from (`bom`, `header`, `meta`, `utf8` or `detected`).
- `languages.test` and `1.languages.test`, and the IDN host name `bücher.languages.test` (`xn--bcher-kva.languages.test`): the `languages` family (ADR 0073). `/pages` links Arabic and Hebrew with their marks, Japanese and Chinese without spaces, Devanagari, Thai, combining marks served decomposed (NFD), emoji sequences, astral characters, the Unicode record (bidi text with marks, NFC and NFD forms of one word, a ZWJ sequence, an astral character, a zero-width joiner, a BOM, a tab and a newline), and a page at the non-ASCII path `/wörter/日本語?q=café&ort=Zürich`, which needs its query. Every expected text is in the normalization form served.
- `files.test`, and `1.files.test` to `3.files.test`: the `files` family (ADRs 0025 and 0119). `/files` links a PDF, a ZIP, a JSON response, a 404 typed `text/csv`, an HTML page and a PDF served with no `Content-Type`, and a page whose `<a download>` link offers `/files/export.csv`.

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract` or `submit`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`, and for a submitted text the `difference`: the index `at` of the first character that differs and the `expected` and `submitted` characters there as `U+XXXX`), the `missing_effects` and `extra_effects`, the `quality` of the extraction, scored field by field against the ground truth (for each field, and in all, how many of the `expected` values are `correct`; a missing Record's fields count as missing values), and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |
| `extraction/1` | `crawl` | `/products` | each product's text, JSON-LD and Open Graph fields |
| `extraction/2` | `extract` | `/shop` | the products its hidden API serves |
| `extraction/3` | `crawl` | `/churn` | each product, whatever its class names |
| `extraction/4` | `crawl` | `/variants` | each product, whatever its layout |
| `extraction/5` | `extract` | `/huge` | its 2,000 products |
| `extraction/6` | `crawl` | `/us/products/X-0100` | the product's price in each region, as a plain decimal |
| `encodings/1` | `crawl` | `/pages` | each page's title and text in UTF-8 |
| `encodings/2` | `crawl` | `/pages` | the same, with each page's encoding and its source |
| `languages/1` | `crawl` | `/pages` | each page's title and text as served |
| `languages/2` | `extract` | `/` on `xn--bcher-kva.languages.test` | the page on the IDN host name |
| `files/1` | `crawl` | `/files` | each linked response's status, type as a navigation computes it, and SHA-256 |
| `files/2` | `extract` | `/files/report.pdf` | the PDF's title and text |
| `files/3` | `extract` | `/files/download` | the download's suggested name and SHA-256 |

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes.
