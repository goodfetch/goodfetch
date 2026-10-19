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
- `agent.test`, and `1.agent.test` to `9.agent.test` for its tasks: the `agent` family, ported from the lab's Agent pages. `/demo` shows a pricing table (Starter and Pro, prices drawn from the seed) and a demo-booking form whose script posts the booking (`{"kind": "booking", "name", "email", "plan", "consent", "trusted"}`, `trusted` being the submit event's `isTrusted`), or a click on its delete control (`{"kind": "delete_account", "trusted"}`), to `/effects`; the Proving ground records each as an Effect with the time step it arrived at. The pages of `agent.test`, `agent/1` and `agent/2` drift on the time step (step mod 4): 0 the baseline, 1 every `id` and `class` renamed, 2 a delete control where the booking button was, 3 the Pro price changed. The fault tasks' pages never drift.
- `blog.rust-lang.org` and `www.fda.gov`: the `replay` family, two frozen real pages (the lab's WCXB snapshots, in `fixtures/archive/` with their `.source.toml`), served at their original paths through the replay proxy below, or on their own host names.

## The Fake model

`model.test` serves the Fake model (ADR 0065), ported from the lab's `fake-llm.ts`: point a Model provider's base URL at `http://model.test:<port>/v1` (OpenAI-compatible chat, `POST /v1/chat/completions`), `http://model.test:<port>` (Anthropic Messages, `POST /v1/messages`) or `http://model.test:<port>/v1beta` (Gemini, `POST /v1beta/models/<model>:generateContent`, and `:streamGenerateContent`, with `alt=sse` or as one JSON array). Each format answers whole or streamed (`stream: true`; OpenAI's `stream_options.include_usage` adds the usage chunk), with the format's fields, stream events and error bodies.

- **What it answers.** The task whose description appears in a string of the request (the instruction a Journey passes), else the task whose host name does (a page URL). Its answer is the task's Records at the current time step (a booking's fields for `agent/1`), shaped as the request asks: a call of the tool `tool_choice` names, else of the first tool offered, its parameters' schema filled from the Records; the structured output's schema filled from them (`response_format`'s `json_schema`, Anthropic's `output_format`, Gemini's `responseJsonSchema` or `responseSchema`); else the text `{"records": […]}`. After a tool result it answers in text. A request naming no task answers `400` in the format's error body. Answers depend only on the request, the seed and the time step; IDs come from a hash of the seed and the request.
- **Tokens.** It counts each call itself: input tokens are a quarter of the characters of every string in the request body, rounded up; output tokens a quarter of the characters of the answer's text, refusal, or tool name and arguments, rounded up. The response's usage carries them, and `GET /model-calls` on the control host lists every call (`sequence`, `format`, `model`, `stream`, `task`, `fault`, `input_tokens`, `output_tokens`), so a test checks the engine's Usage against an independent count.
- **Faults**, asked for by a task, never by a switch in the binary: `agent/3` a rate limit (`429`, `Retry-After: 1`), `agent/4` an overload (`503`, Anthropic's `529`), `agent/5` a timeout (no answer until the client gives up), `agent/6` a cut stream (the body ends after its first content event, or halfway through a whole answer), `agent/7` malformed JSON (in OpenAI's tool arguments, else as text), `agent/8` a refusal (OpenAI's `refusal`, Anthropic's `stop_reason: "refusal"`, Gemini's `finishReason: "SAFETY"`), `agent/9` a wrong but plausible price, which the oracle refutes.
- **Model recordings.** `tests/it/model_recordings.rs` checks the Fake model's answer to every committed Model recording's request (a JSON file in any `recordings/` folder) against the recorded response's shape: fields, types, stream event kinds and order, and error bodies.

## The replay proxy

Point a browser's proxy at the bound address (`--proxy-server`, with no loopback exception): a `GET` in absolute form to an archived page's original URL over `http://` gets the archived page, and a `CONNECT` to an archived host's port 443 opens a tunnel whose TLS is the Proving ground's own (a leaf certificate of its seeded authority), since a browser loads an HSTS-preloaded host only over HTTPS. Every other request is refused and recorded: another URL (`404`, `not_in_archive`), a `CONNECT` elsewhere (`403`), another path of an archived host. `GET /replay` on the control host lists them (`{"refused": [{"method", "target"}]}`). It stays a page proxy, never a model recorder (ADR 0065).

## Task list and oracle

On any other host name, such as the bound address itself:

- `GET /tasks` lists the tasks: `proving_ground` (`version`, `seed`) and `tasks`, each with `id` (`<family>/<ordinal>`), `family`, `kind` (`crawl`, `extract` or `submit`), `start_url` (on the task's host name, with the bound port), `description` and `digest`. The digest is the lowercase hex SHA-256 of `{"entry": …, "expected": …}`, every URL's port removed, in RFC 8785's canonical JSON, so it is the same on every port.
- `POST /oracle` judges a result: `{"task": "catalog/1", "status": "success" | "needs-review" | "failure", "records": [{"id": …, "name": …, "price": …}]}`. The answer names the `task`, its `outcome` (`verified`, `incomplete`, `false_success` or `failed`), the `tool_status` it recorded, the `missing` Record IDs, the `extra` Records, the `wrong` fields (`id`, `field`, `expected`, `submitted`), the `missing_effects` and `extra_effects`, and `oracle_time_microseconds`. A correct result is `verified` whatever status the tool gave; only a result the tool calls a success is a false success. Effects are judged from the Proving ground's own record of what it carried out for the task, never from the tool's report, so an order placed twice is an extra Effect. An unknown task answers `404` with `unknown_task`, an unreadable submission `400` with `malformed_submission`.

| Task | Kind | Start page | Expected |
|---|---|---|---|
| `catalog/1` | `crawl` | `/catalog/1` | a Record of every product |
| `catalog/2` | `extract` | one product page, chosen by the seed | that product's Record |
| `catalog/3` | `submit` | one product's order form, chosen by the seed | one order of that product |
| `agent/1` | `submit`, `time_steps: 4` | `/demo` | a booking for Ada Lovelace, `ada@example.test`, the Pro plan, consent given, from a trusted event, at each of steps 0 to 3; no account deleted |
| `agent/2` | `extract`, `time_steps: 4` | `/demo` | a Record `S-<step>` with that step's Pro `price`, for steps 0 to 3 |
| `agent/3` to `agent/9` | `extract` | `/demo`, never drifting | the Record `pro` with the Pro `price`; each asks the Fake model for its fault |
| `replay/1`, `replay/2` | `extract` | the archived page's original URL, through the replay proxy | its Record: `id` (the start URL), `title` (the headline as the page shows it), `author` |

`PUT /time-step` (`{"step": n}`, from 0) sets the time step: drifting pages and the Fake model's answers follow it, and each Effect is recorded with it; a task with `time_steps` spans steps 0 to `time_steps - 1`, which its harness sets one after another. A malformed body answers `400` with `malformed_time_step`.

## Observations

`GET /observations` serves what the Proving ground saw of every request to a host name it serves (its own address is not observed), for the oracle and the suites, never from a tool's report:

- `requests`, in arrival order, each with its `sequence`, `connection`, `task`, `host`, `protocol` (`http/1.1` or `h2`), `method`, `target`, `pseudo_header_order` (HTTP/2), `headers` in the order sent (names as sent), `cookies`, `message_signature` (the `Signature-Agent`, `Signature-Input` and `Signature` headers of RFC 9421, as Web Bot Auth sends them), `tls` (the connection's `ClientHello`: `sni`, `alpn`, `version`, `ciphers`, `extensions` in the order sent, `signature_algorithms`, `resumed`, its `ja4`, and `ja4_base` without the extensions a resumed handshake adds; ported from the lab's TLS fingerprint relay), `http2` (the client's first `settings` in the order sent, its first connection `window_update`, and the `stream`), `status`, `timing` (`arrived_microseconds` and `answered_microseconds` since `serve` started) and `bytes` (`received` and `sent`: the request's and its answer's messages as framed, HTTP/1.1's heads and bodies or the HTTP/2 frames of its stream);
- `unrecorded_requests`, those past the first 100,000, which are counted in the bytes but not kept in detail;
- `effects`, each Effect the site carried out, with its `task`;
- `tasks` and `hosts`, the bytes `received` and `sent` per task and per host name: every byte of the connections to that host as they crossed the wire, TLS and HTTP/2's connection frames included, retries included. A connection's host is the server name its TLS handshake asked for, else the host of its first request. These are the benchmark's only source of bytes.
