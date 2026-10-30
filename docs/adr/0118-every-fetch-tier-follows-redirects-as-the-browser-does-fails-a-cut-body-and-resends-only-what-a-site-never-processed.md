# Every fetch tier follows redirects as the browser does, fails a cut body, and resends only what a site never processed

A crawl request reaches its handler only as a whole document, after the same redirects on every fetch tier. A fetch follows at most 19 redirects (20 requests), as the browser does, a refresh stub's target counting as one of them. A body that broke off fails its attempt. A request the site never processed is sent again once, at no cost to the request's retries or the site's Politeness. A site's `Retry-After` is never cut short, and the backoff is spread with jitter. Before this, ADR 0020 said only that "the engine sets decompression, redirects and timeouts itself", and ADR 0037's SA-24 read that as decided. No hop limit, loop outcome or method rule was stated, and nothing said what a truncated body, a dropped connection or a `Retry-After` of hours does.

**What this answers.** Rows U12, U13, H02, H05, H07, H15 and H24 of the [pitfall catalog](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md): redirect chains and loops, refresh stubs on the HTTP tiers, stale DNS answers and a broken IPv6 path, HTTP/2 GOAWAY and stale keep-alive connections, a `Retry-After` of hours, backoff without jitter, and truncated bodies stored as pages. The evidence is items 1.17, 1.18, 2.2, 2.3, 2.6 to 2.10, 2.12, 2.17 and 2.25 of the [common pitfalls research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md).

**Specs and leaders, re-read on 2026-10-16.**
- **Redirects.**
  - The [Fetch standard](https://fetch.spec.whatwg.org/#http-redirect-fetch) returns a network error "If request's redirect count is 20" (step 7).
  - Its step 12 turns a 301 or 302 POST, and a 303 with any method but GET or HEAD, into a GET with no body and no request-body headers.
  - Its step 13 removes `Authorization` "the moment another origin is seen".
  - Its step 6 fails a `Location` whose scheme is not HTTP(S).
  - Chromium has `kMaxRedirects = 20` ([url_request.h](https://raw.githubusercontent.com/chromium/chromium/main/net/url_request/url_request.h)) and returns `ERR_TOO_MANY_REDIRECTS` past it ([url_request_job.cc](https://raw.githubusercontent.com/chromium/chromium/main/net/url_request/url_request_job.cc)). A navigation of the pinned Chrome for Testing 156 counts its first request in those 20 and follows 19 redirects (#699, the note under "On a Collie page").
  - Playwright's [`maxRedirects`](https://playwright.dev/docs/api/class-apirequestcontext#api-request-context-fetch-option-max-redirects) defaults to 20, and 0 does not follow.
  - Scrapy's `REDIRECT_MAX_TIMES` is 20, and a status in `handle_httpstatus_list` reaches the spider instead of being followed ([downloader middleware](https://docs.scrapy.org/en/latest/topics/downloader-middleware.html)).
  - Googlebot follows 10, and Nutch follows none inline (research 1.17).
  - RFC 9110 [§15.4](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4) asks a client to "detect and intervene in cyclical redirections". [§10.2.3](https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.3) makes a `Retry-After` on a 3xx the least wait before the redirected request.
- **Refresh stubs.**
  - The HTML standard runs the same [shared declarative refresh steps](https://html.spec.whatwg.org/multipage/document-lifecycle.html#initialise-the-document-object) for a `Refresh` header as for `<meta http-equiv="refresh">`. A refresh with no URL reloads the document.
  - Scrapy follows one with up to 100 s of delay (`METAREFRESH_MAXDELAY`) at once, and ignores one inside `<noscript>` since 2.11.2.
- **Truncated bodies.**
  - RFC 9112 [§8](https://www.rfc-editor.org/rfc/rfc9112.html#section-8): a client "MUST record the message as incomplete".
  - Scrapy's `DOWNLOAD_FAIL_ON_DATALOSS`, on by default, raises a retryable `ResponseDataLossError`.
  - [WARC 1.1](https://iipc.github.io/warc-specifications/specifications/warc-format/warc-1.1/#warc-truncated) records a response cut by the network with `WARC-Truncated: disconnect`.
- **Transport failures.**
  - RFC 9113 [§6.8](https://www.rfc-editor.org/rfc/rfc9113.html#section-6.8): streams above a GOAWAY's last stream ID were not processed. [§8.7](https://www.rfc-editor.org/rfc/rfc9113.html#section-8.7): REFUSED_STREAM can be retried whatever the method.
  - RFC 9110 [§15.5.20](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.20): a 421 may be retried on another connection.
  - RFC 9112 [§9.3.1](https://www.rfc-editor.org/rfc/rfc9112.html#section-9.3.1) lets a client retry an idempotent request on a connection that closed under it.
  - RFC 9110 [§9.2.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-9.2.2): "A client SHOULD NOT automatically retry a failed automatic retry."
  - Chromium's [http_network_transaction.cc](https://raw.githubusercontent.com/chromium/chromium/main/net/http/http_network_transaction.cc) resends a request whose reused keep-alive connection resets or closes before any header ("we resend a request only if we reused a keep-alive connection"). It also resends on REFUSED_STREAM, on a 421 with pooling off, on `HTTP_1_1_REQUIRED` over HTTP/1.1, and on a 408 over a reused HTTP/1.1 connection, all inside one navigation.
- **Backoff and `Retry-After`.**
  - The [AWS Architecture Blog](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/) finds no jitter "the clear loser" and Equal Jitter "the loser". Between Full and Decorrelated Jitter, it finds the choice "less clear".
  - The [Builders' Library](https://builder.aws.com/content/3EumjoZascWd1oZiEgL8ORlv3qE/timeouts-retries-and-backoff-with-jitter) advises retrying "at a single point in the stack".
  - urllib3 caps `Retry-After` at 21,600 s, with no jitter by default.
  - Crawlee's unreleased 4.0 (its `master`) stops a crawl with `PersistentRateLimitError` after 900 s without one request getting through (research 2.8).
- **Resolution.**
  - RFC 8305 [§8](https://www.rfc-editor.org/rfc/rfc8305.html#section-8) recommends a 250 ms connection attempt delay and a 50 ms resolution delay. hyper-util's default is 300 ms ([`set_happy_eyeballs_timeout`](https://docs.rs/hyper-util/latest/hyper_util/client/legacy/connect/struct.HttpConnector.html#method.set_happy_eyeballs_timeout)).
  - hickory-resolver keeps answers for their TTL ([`ResolverOpts`](https://docs.rs/hickory-resolver/latest/hickory_resolver/config/struct.ResolverOpts.html)).
  - RFC 2308 [§7.1](https://www.rfc-editor.org/rfc/rfc2308.html#section-7.1) keeps a server failure five minutes at most.

Decided in [Redirect limits, truncated bodies and transport failures on the fetch tiers](https://github.com/goodfetch/goodfetch/issues/549), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), raised by the [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/549#issuecomment-6204511037) and its [first addendum](https://github.com/goodfetch/goodfetch/issues/549#issuecomment-6204849045). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). The two new codes are Collie's, in its client package, since a Collie navigation and `page.request` raise them too; a Foxhound crawl request uses them as it uses `navigation_failed` ([ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md)).

## Decision

### Redirects

- **What is followed.** One of Fetch's redirect statuses (301, 302, 303, 307, 308) with a `Location` that parses to an HTTP(S) URL. It is followed on every tier, so a handler gets the same document whatever its tier, and a sample comparison sees the same chain, as ADR 0093's one handler for every tier needs and as the browser does (S3).
  - **Any other 3xx fails at once with `response_not_ok`**, like a 4xx (ADR 0093). That covers a status Fetch does not follow (300, 305, 306, or a 304 that the Response cache did not ask for), a redirect with no `Location`, and one whose `Location` does not parse or names another scheme.
  - **A redirect status listed in `crawl.handle_statuses` is not followed.** The handler gets the 3xx as its response, as Scrapy's `handle_httpstatus_list` does. That is the one way not to follow a redirect (S2).
- **At most `fetch.redirect.max` redirects,** 19 by default, as many as the pinned browser follows in a navigation: 20 requests, its first included (owner, 2026-10-22). Fetch, Scrapy and Playwright's `maxRedirects` follow 20, one more than the browser tier can.
  - It is set down to a Label, and takes 1 to 19. Any other value is refused with `setting_invalid` when its scope opens (G3), since the browser cannot follow more than 19 and not following is said through `crawl.handle_statuses`.
  - Past it, the attempt fails with `too_many_redirects`, a `PageError` that is not retryable, since a loop answers the same way again (G5). The request then ends `failed` with it.
  - No cycle is detected before the count. A site that sets a cookie at A and redirects through B back to A, which then answers 200, visits a URL twice by design, and the count is how Fetch and Chromium intervene in a loop.
- **Method, body and headers follow Fetch,** so every tier sends what the browser sends:
  - a 301 or 302 turns a POST into a GET, and a 303 turns any method but GET or HEAD into a GET, both without the body and its request-body headers;
  - a 307 or 308 keeps the method and the body;
  - `Authorization` is dropped once the chain reaches another origin, and a Secret placeholder resolves only for its own domains on every hop (ADR 0092);
  - each hop's cookies come from the Session's jar for that hop's URL.
  - The crawl request keeps its Frontier identity, the method, URL and body hash it was admitted with (ADR 0093).
    > Amended by [ADR 0120](0120-the-frontier-parses-urls-as-browsers-do-fixes-only-safe-rules-learns-the-rest-on-evidence-and-caps-a-urls-shape.md): the identity's URL is its key, made by ADR 0120's rules. A hop that reaches a URL whose key the Frontier already admitted for another request ends the request `skipped` with `duplicate`, and its handler does not run. A hop back to the request's own key, or to a URL earlier in its own chain, is followed and counted as above.
- **Each hop goes through the navigation gate** (ADR 0087), on every tier:
  - the crawl's scope (`out_of_scope`), the domain policy (`domain_policy`) and robots.txt (`robots_txt`), as ADR 0093 already checks every redirect;
  - the hop's domain's limits and delay. Each hop takes its own permit, and the previous hop's 3xx releases its own. A loop therefore costs a site at most `fetch.redirect.max` + 1 requests, 20 by default, at the domain's pace, never a burst (G5).
    > Amended by [#762](https://github.com/goodfetch/goodfetch/issues/762): a loop sends `fetch.redirect.max` + 1 requests, its first request and one for each redirect followed; the count of at most `fetch.redirect.max` requests this ADR first gave left the first request out.
    > Note (#762, agent decision, pending owner review): on the `http` tier, the chain checks each hop after the first in this order: the count, the redirect's `Retry-After`, then the Frontier (the scope's `out_of_scope`, ADR 0120's `duplicate`) and the navigation gate (robots.txt and the domain's delay). A hop's skip carries no message, since an outcome's message is the handler's or a Hook's own text. The wait a redirect's `Retry-After` asks holds the request's lease and is named `waiting_for_permit`, as a permit's wait is, so it counts against no timeout; one past the bound ends the request `failed` with `response_unavailable`, its status the 3xx's, without backing the domain off, since a 3xx is not a failing site. Only the first hop is coalesced or hedged (ADR 0040), since a hedge's room is taken under the first hop's domain key. `fetch.redirect.max`, `fetch.refresh.max_delay` and `crawl.handle_statuses` are read from the Run's settings until a Label's scope exists, and the domain policy's `domain_policy` skip for a hop is #676's.
  - A permit's wait counts against no timeout (ADR 0093).
- **A `Retry-After` on a redirect** delays its next hop by at least that long (RFC 9110 §10.2.3), bounded like any other `Retry-After` (below).
- **The record.** `ctx.response` is the final document's response, with the redirect chain ADR 0093 gives it, a refresh hop marked as one. The Run trace records each hop and its wait.
- **On a Collie page.**
  - A navigation past the browser's 19 redirects raises `too_many_redirects`, where Chromium reports `ERR_TOO_MANY_REDIRECTS`. Any other network error still raises `navigation_failed` (ADR 0085).
    > Note (#699, accepted by the owner on 2026-10-22: every fetch tier follows at most 19 redirects (20 requests), as the browser does): the pinned Chromium (Chrome for Testing 156) follows 19 redirects in a navigation, not 20: the Proving ground saw 20 requests of the `redirects` family's chain of 20, `/hops/20` to `/hops/1`, and none to `/hops/0`, and a chain of 19 loads. Its count of 20 holds the chain's first request. A Collie navigation reports the browser's count, 20, and the 19 hops it followed.
  - `page.request` keeps Playwright's `maxRedirects` option, 19 by default where Playwright's is 20; `0` returns the 3xx. Past the count it raises `too_many_redirects`. A value above 19 is refused with `setting_invalid`, since the browser-fetch tier that sends it on a Session with a browser cannot go past 19, and every tier keeps that count (ADR 0083 makes a call's argument a run-layer value).
  - A Collie navigation ignores a `Retry-After` on a 3xx, as browsers do: it is the program's own navigation, outside Politeness (ADR 0086).

### Refresh stubs and script redirects

- **A crawl request's document that refreshes to another URL is followed as one more hop of its chain,** on every tier, when its delay is at most `fetch.refresh.max_delay`.
  - The refresh is the document's first refresh directive, as the HTML standard reads it: a `Refresh` header, or a `<meta http-equiv="refresh">` outside `<noscript>`. A browser that runs scripts ignores what `<noscript>` holds.
  - `fetch.refresh.max_delay` is 100 s by default, Scrapy's value, set down to a Label. `null` follows any delay, and `0` only an instant refresh. A longer delay usually belongs to a page with content of its own, such as a session-timeout refresh to a logout page.
  - The hop goes at once, without waiting the delay, as Scrapy's does. It counts toward `fetch.redirect.max` and goes through the gate like any hop. In the browser, the engine's own navigation replaces the stub before the stub's timer fires, so every tier hands the handler the same document (ADR 0093).
  - A refresh with no URL, or to the document's own URL, is a reload. It is never followed, and the handler gets the document.
    > Note (agent decision, pending owner review), 2026-10-30, deep review 579-8 (G23): a refresh to a URL that is no HTTP(S) URL, such as `javascript:` or `data:`, is not followed either, and the handler gets the stub: Chromium refuses such a refresh and keeps the document, so ending the request `skipped` at the hop's scope or egress check, as before, gave the document tiers another answer than the browser. The stub's markup is read on the blocking pool (STYLE.md).
- **A redirect by the page's own script is seen only where scripts run.** A document tier never sees one. A sample whose browser attempt ended on another URL than the document tier's attempt never matches, so that Label's tier choice stays the browser (ADR 0020).
- **This rule is the crawl request's document's.** A handler's own `goto` and a Collie program's navigation get the page their tier gives. The browser runs a refresh itself, as a navigation the page starts, gated and counted inside a handler (ADR 0087). A document tier runs none.
  > Note (agent decision, pending owner review), 2026-10-30, deep review 579-7 (G23): a served page's `goto`, `reload`, `goBack` and `goForward` follow their redirects with refresh following off (`Following::refresh`), so the handler is given the stub, as this rule says; before, they followed the crawl's `fetch.refresh.max_delay`.

### A body that broke off

- **What counts as cut.** A body that ends before its `Content-Length`, a chunked body that ends without its last chunk, an HTTP/2 stream reset after its headers or ended by its connection, and a compressed body whose stream stops before its end.
  - A body delimited by its connection's close, with no length and no chunking, is complete at the close, as browsers read it.
  - A body labelled with a coding it does not use is [Decoding a response the same way on every tier](https://github.com/goodfetch/goodfetch/issues/550)'s question, not a cut one.
- **A crawl request's cut document fails its attempt with `response_truncated`,** a `PageError` that is retryable, on every tier, counted against `crawl.retry.max`. It never reaches the handler (G1). When retries run out, the request ends `failed` with it. It does not back off the domain: like a 500, it usually belongs to one response.
  - Its details carry the URL, the bytes received and the bytes expected, `null` when the body had no length.
  - `crawl.handle_statuses` does not apply, since the status was not the failure.
- **On a Collie page.**
  - A navigation that waits for `domcontentloaded` or `load` raises `response_truncated` when its main document was cut. One that returned at `commit` leaves it to the body's read.
    > Note (#699, agent decision, accepted by the owner on 2026-10-22): the signal on the pinned Chromium (Chrome for Testing 156) is a network error after the main document's response arrived, other than the browser's own cancel (`net::ERR_ABORTED`) and `net::ERR_CONTENT_DECODING_FAILED`: the probes saw `net::ERR_CONTENT_LENGTH_MISMATCH`, `net::ERR_INCOMPLETE_CHUNKED_ENCODING`, `net::ERR_HTTP2_PROTOCOL_ERROR`, `net::ERR_CONNECTION_RESET` and `net::ERR_CONNECTION_CLOSED`, and Chromium fires no `load` for such a document. A gzip body cut before its `Content-Length` is a length mismatch, so it is `response_truncated`, never a decoding failure. A gzip stream cut within a whole transfer gives no signal: Chromium decodes its prefix and fires `load`, and CDP hands the engine only decoded bytes, so a Collie navigation answers that response as the browser shows it. The probe also found that CDP's `Fetch` interception hands over decoded bodies, so the browser tier of a crawl cannot read the stream's end there either. The bytes received are the decoded ones the browser reports.
    > Note (agent decision, pending owner review), 2026-10-28, deep-review finding V152-a: `net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG`, the pinned Chromium's refusal of a zstd window past RFC 9659's 8 MB, is a content coding that failed to decode, as `net::ERR_CONTENT_DECODING_FAILED` is, never a cut body: a `page.request` or navigation that meets it after the response fails with `navigation_failed` naming it.
  - `body()`, `text()` and `json()` of a cut response raise it, and so does a `page.request` whose body was cut. A cut body is never returned as a whole one.
    > Note (agent decision, pending owner review), 2026-10-28, fix-queue NEW-2: the pinned Chromium (Chrome for Testing 156) can lose a cut body's network error when the body is read at the response stage, as `page.request` reads it. `Fetch.getResponseBody` answers once the body's stream ends, and the network's failure reaches the paused response on another channel; when `Fetch.continueResponse` gets there first, the interception ends the load itself: `Network.loadingFinished` with `encodedDataLength` 0, no `loadingFailed` after it, and the page fires `load`. A CDP replay of `page.request`'s sequence against a server that closes its connection 50 to 100 ms after sending half a body saw it in 1 to 4 loads of 40 to 60, for a body shorter than its `Content-Length` and a chunked body without its last chunk alike, never when the close comes with the bytes. A load the network finished counts its headers at least (475 bytes for that whole page), and only one the disk cache answered counts 0, whole. So `page.request` now reads a body with no content coding that is shorter than its `Content-Length` as cut when the browser finished it with no bytes counted, a `HEAD` and a 1xx, 204 or 304 excepted. A chunked or coded body cut so still reads as whole: nothing the browser reports tells it from a whole one. Reading the body without pausing its end would close that gap, at the cost of the raw bytes this ADR's reads keep (ADR 0119).
  - The `requestfailed` event and `request.failure()` keep Playwright's meaning, the browser's network error.
- **Caches and recordings.** The Response cache never stores a cut response (ADR 0046). A Response recording keeps it with `WARC-Truncated: disconnect`, and a replay of it raises `response_truncated` as the live fetch did (ADR 0105), so a program can test its handling offline.

### Requests a site never processed

- **The engine's HTTP transports send a request again, once, on a new connection,** when one of these happens:
  - an HTTP/2 GOAWAY whose last stream ID is below the request's stream (RFC 9113 §6.8);
  - an HTTP/2 REFUSED_STREAM (RFC 9113 §8.7);
  - a 421, sent again on a connection of the origin's own, never coalesced (RFC 9110 §15.5.20);
  - an HTTP/2 `HTTP_1_1_REQUIRED`, sent again over HTTP/1.1, which that origin then keeps for the Run;
  - a reused idle connection that closes or resets before any byte of the response arrives, or answers 408 on HTTP/1.1 (RFC 9112 §9.3.1, §9.5).
- **Which methods.** The first four are sent again whatever the method: the site said it processed nothing. The last is sent again only for a read: a crawl request, which ADR 0106 makes a read whatever its method, or an idempotent method (RFC 9110 §9.2.2).
- **Once.** A resend that fails, or that a reused connection answers 408 again, is never resent (RFC 9110 §9.2.2). Its failure or its 408 is then the attempt's.
- **A resend counts as nothing.** It is not an attempt, not a retry under `crawl.retry.max`, and no new request start in Politeness: it holds its hop's permit, as the browser's own resends do inside one navigation. The Run trace records each one, and its bytes count in Usage. Counting them would fail and slow the HTTP tiers where the browser tier succeeds, and move a Label's tier choice to the browser for a server's routine draining of connections.
  > Note (agent decision, pending owner review), 2026-10-24, [#763](https://github.com/goodfetch/goodfetch/issues/763): on the `http` tier, a `GOAWAY` counts as below the request's stream only with the error code `NO_ERROR`, since an HTTP/2 library reports a `GOAWAY` carrying an error to the streams the site processed too. A reused connection is told from a new one by whether the hop opened a connection itself, so a hop that waited for another hop's new connection counts as new and is not resent: the doubt never resends. A resend that fails names `net::ERR_HTTP2_SERVER_REFUSED_STREAM`, `net::ERR_HTTP_1_1_REQUIRED` or, for a reused connection that closed, `net::ERR_EMPTY_RESPONSE`; and a cut body's `received` counts its bytes as sent, before any content coding is decoded, so it compares with `expected`. The Run trace's record of each resend and its bytes in Usage wait for the tier's Run trace, and the Response cache and Response recording lines for #793 and #1291.
  > Note (agent decision, pending owner review), 2026-10-28, step 7 review (A5): a transport remembers at most 256 origins that answered `HTTP_1_1_REQUIRED`. Past that, it forgets the one that answered longest ago. That origin's next request is then sent over HTTP/2 again and costs one resend over HTTP/1.1. A Run that reaches many such origins therefore holds no state that grows with them (STYLE.md, Memory and hot paths).
- **Every other transport failure ends the attempt with `navigation_failed`,** the network error in its details, retryable, on every tier, as ADR 0085 names a site that never answered. That includes a failed resend, a name that does not resolve, a refused or timed-out connection, a TLS failure and a reset on a fresh connection. A refused or timed-out connection also backs off the domain (ADR 0086).
  > Note (agent decision, pending owner review), 2026-10-23, [#759](https://github.com/goodfetch/goodfetch/issues/759): on the `http` tier a received head holding a NUL, a bare CR or a line without a colon fails with `navigation_failed` carrying `net::ERR_INVALID_HTTP_RESPONSE`, as ADR 0067's #384 note expects of a head the browser refuses, and a folded line is kept, its fold read as a space. The browser path is the known exception: Chromium keeps a bare CR or LF in a received head as a line break (ADR 0067's #384 note), so the `headers` family's `/bare-cr` page is refused on the `http` tier only, until Goodfetch's own check on the browser path is decided. A bare LF ending a line is accepted on every tier, as Chrome accepts it.
- **An HTTP/2 `ENHANCE_YOUR_CALM` from a site** backs off the domain as a 503 does (G5).
  > Note (agent decision, pending owner review), 2026-10-26, [#766](https://github.com/goodfetch/goodfetch/issues/766): Chromium names a reset or `GOAWAY` carrying `ENHANCE_YOUR_CALM` as it names every HTTP/2 error it has no network error of its own for, `net::ERR_HTTP2_PROTOCOL_ERROR`, so the browser tier cannot tell it apart. The HTTP tiers translate it to their own variant and name it the same way, so a `navigation_failed` naming `net::ERR_HTTP2_PROTOCOL_ERROR` backs off its domain on every tier: a site's other HTTP/2 protocol errors then slow its crawl too, the polite side of the doubt. The crawl now reads the site's answer to a request's own document itself, on the `http` and `browser` tiers, whatever the handler made of the page: its 429, 502, 503 or 504 backs off the domain and its `Retry-After`, read with `Date` as above, holds it, besides a handler's failure as #1541's note says. The request's own outcome is unchanged: a handler given such a page still decides it.
- **The browser tiers.** The browser, and `page.request` on a Session with a browser, which the browser-fetch tier sends, rely on the browser's own resends, which cover the same cases. On a Session with no browser, as a crawl handler's on a document tier, `page.request` goes through the Session's HTTP transport (ADR 0093), so the rules above are its own: a POST it sends is not resent after a reused connection closed.

### `Retry-After`

- **Its two forms** (RFC 9110 §10.2.3): a number of seconds, and an HTTP-date in each of RFC 9110 §5.6.7's three formats.
  - A date is measured against the response's own `Date` header when it has one, and against the engine's clock otherwise, so a clock skewed on either side changes nothing.
  - A value that does not parse, `0`, or a date already past adds no wait beyond the backoff. The Run trace notes a value that does not parse.
- **Where it is read.** On a 429, 502, 503 or 504, the responses that back off the domain (ADR 0086), and on a 3xx, where it delays that request's next hop.
- **How long the domain waits.** The longer of its backoff and the `Retry-After`, past `politeness.backoff.max`, up to `politeness.retry_after.max`. That is 15 minutes by default, the bound Crawlee's unreleased 4.0 sets on a crawl with no request getting through. ADR 0053 keeps the time per domain and shares it with every run on the key.
- **Past `politeness.retry_after.max`,** the engine still sends nothing to the domain before the time the site asked (G5). It does not wait for it either:
  - the request whose response asked ends as that response ends, with no retry spent: a 503, 502 or 504 `failed` with `response_unavailable`, a 429 by its Block cause (ADRs 0021, 0093);
  - a 3xx's request ends `failed` with `response_unavailable`, carrying that status;
  - each crawl request admitted to the domain while the site's time is further away than `politeness.retry_after.max` ends `failed` with `response_unavailable` too. Its status is `null`, since nothing was sent.
  - Each such error's details carry the time the site asked for (`retryAfter`), and its `fix` names `politeness.retry_after.max`. The Run finishes `incomplete`, and `retryFailed()` re-admits those requests once the site is back (ADR 0102).
  - This is ADR 0086's answer to a long Crawl-delay: neither waiting it out nor capping it silently.
- **`politeness.retry_after.max` presses no site harder,** whatever its value, since nothing is sent sooner. It is therefore not among ADR 0086's recorded values. Switching `politeness.backoff.enabled` off still ignores `Retry-After`, and the run records it (ADR 0023).
- **On a Collie page,** nothing waits on a `Retry-After`: the program directs its own navigation, and a site's 429 there is reported on the call that met it (ADR 0098). Its `BlockedError` for the Block cause `rate_limit` carries the parsed time as `retryAfter`, or `null` (G6).
  > Note (agent decision, pending owner review), 2026-10-22, [#1541](https://github.com/goodfetch/goodfetch/issues/1541): the backoff's jitter and the bound live in Foxhound's per-domain rule in the Politeness file (`goodfetch-frontier`'s `politeness`), each Run's gate drawing from an `Entropy` forked for it. The domain keeps its last wait in the file, so the "previous wait" is shared by every run on the key, and a success resets it. A run waits for the site's time up to the longer of `politeness.retry_after.max` and `politeness.backoff.max`, since a backoff is always waited out. Past it, the request whose response asked ends as its handler's failure ended, with no retry and the `fix` added, and each request admitted meanwhile ends `failed` with `response_unavailable`, unsent, its attempt counted so `run.failed_requests` lists it. A crawl that ends with a request `failed` with `response_unavailable` is `finished` and `incomplete`. An attempt's error keeps its `details` in its outcome, `retryAfter` and `fix` among them. A run with `politeness.backoff.enabled` off ignores every sharing run's backoff and `Retry-After`, not only its own. Until the fetch tiers read the header themselves, with the engine's `retry_after::read`, the time reaches Politeness as a handler failure's `retryAfter` (RFC 3339) on `blocked` for `rate_limit` or on `response_unavailable` for a 502, 503 or 504; the Run trace's note of a value that does not parse waits on the Run trace.
  > Note (agent decision, pending owner review), 2026-10-29, [#1737](https://github.com/goodfetch/goodfetch/issues/1737): the bound is judged when the site asked, never when the engine learned of it. A handler failure's `retryAfter` is measured from the moment its handler request went out, so the time the handler and the engine took to report it never shortens the site's wait, and a wait past the bound stays past it however late the failure arrives. The Run that judged it so then refuses its domain's requests until the site's time, each ending `failed` with `response_unavailable` unsent however late it is admitted, rather than waiting out the rest: nothing waits for it. Another run sharing the domain, which never saw the response, still judges the site's time when it admits a request, as above. Before this, both were judged when the failure was settled, so a handler or an engine slower than the bound's margin turned the refusal into a wait.

### Jitter

- **The domain's backoff is spread with decorrelated jitter.** Each wait is drawn from the `Entropy` handle (ADR 0059) between `politeness.backoff.initial` and three times the previous wait, the first between `initial` and three times `initial`, and none above `politeness.backoff.max`. It still starts from 2 s and grows toward 60 s, as ADR 0021 says, but the first waits are spread too. Domains behind one origin or one proxy that back off together then come back apart.
  - It never draws below `politeness.backoff.initial`. Full jitter can, and ADR 0086 counts a shorter wait as pressing a site harder.
- **A crawl retry gets no timer of its own.** It is admitted again under its domain's delay and backoff, the one place where waits are set (the Builders' Library's single retry layer).
- **A resend goes at once,** since the site processed nothing, as the browser's do.
- **The per-domain delay is not jittered.** It is a floor between two starts (ADR 0086).

### The engine's resolver

The engine resolves names itself on Linux, and for Sessions with a local address on every OS (ADRs 0024, 0044). Its answers are kept as the records say:
- an answer, for its TTL, never for a fixed time;
- an NXDOMAIN or an empty answer, for its negative TTL (RFC 2308);
- a server failure or a timeout, not at all, so a retry asks again.

An answer whose every address fails to connect is dropped before its TTL ends, so the next attempt resolves the name again (RFC 8305 §9).

**Happy Eyeballs.** The engine's HTTP transports race the address families the Session holds under RFC 8305. They query AAAA and A together, with its 50 ms resolution delay and its 250 ms delay between connection attempts.
- A Session that holds a local address of one family only uses that family; ADR 0044's `address_family_unavailable` stands.
- The browsers race their own.
- Behind a proxy, the race is to the proxy's address, since the proxy resolves the site.
- These are built-in values. Area 3's spec gives them keys under ADR 0083's rule, as ADR 0028 makes every parameter settable.
  > Note (agent decision, accepted by the owner on 2026-10-21; [#1542](https://github.com/goodfetch/goodfetch/issues/1542)): the keys are `resolver.resolution_delay` (50 ms) and `resolver.attempt_delay` (250 ms), durations settable at every layer and honoured by both Products. An attempt that fails starts the next at once, the families alternate from IPv6, and an answer whose every address failed is forgotten for both families. The engine's own resolver keeps its answers itself, read on the `Clock`, with `hickory-resolver`'s cache off; that resolver still asks a server that failed twice more (its default two retries) before a lookup fails.

### Settings and Error codes

| Key | Built-in | Scope |
|---|---|---|
| `fetch.redirect.max` | 19, from 1 to 19 | a crawl, down to a Label |
| `fetch.refresh.max_delay` | 100 s; `null` for any | a crawl, down to a Label |
| `politeness.retry_after.max` | 15 minutes | a crawl, on a connection or a Run |

The three keys are a crawl's (ADRs 0086, 0093): a Collie engine refuses them with `not_in_product`, and a Collie Run on a Foxhound engine with `setting_not_applicable`. Durations take each language's duration type (ADR 0080).

| Group | Code | Retryable | Package |
|---|---|---|---|
| `PageError` | `too_many_redirects` | no | Collie client |
| `PageError` | `response_truncated` | yes | Collie client |

Their classes are `TooManyRedirectsError` and `ResponseTruncatedError` under ADR 0079's naming rule. `too_many_redirects` carries the first URL, the hops followed and the count's setting or option. `response_truncated` carries the URL and the bytes received and expected. `ResponseUnavailableError` gains `retryAfter`, and its `status` may be `null`. `BlockedError` gains `retryAfter` too: the site's time for a `rate_limit`, or `null`.

## Considered Options

- **10 redirects, as Googlebot:** rejected. A document tier would fail where the browser goes on, and a Label's tier choice would change on a chain of eleven (ADR 0093).
- **A limit above 19, such as the 20 of Fetch, Scrapy and Playwright:** rejected. The pinned browser stops at 19 redirects in a navigation, so a chain of 20 would load on a document tier and fail on the browser tier.
- **Detecting a cycle by a repeated URL:** rejected. It breaks a site that sets a cookie and redirects back to the URL it started from, and Fetch, Chromium and Scrapy all rely on the count.
- **`fetch.redirect.max: 0` to stop following:** rejected by S2. `crawl.handle_statuses` already hands a status to the handler, as Scrapy's `handle_httpstatus_list` does, and two settings for one choice could disagree.
- **`too_many_redirects` retryable, or `navigation_failed` for it:** rejected. A loop answers the same way next time, and each retry costs a site 20 more requests (G5).
- **A redirect's hops sharing one permit:** rejected. A hop to another domain would hold the first domain's slot while waiting for the second's, and two such chains in opposite directions would wait on each other forever.
- **Following instant refreshes only:** rejected. It misses the common stub that redirects after a few seconds.
- **A refresh stub failing its comparison so it escalates to the browser:** rejected. Every stub would cost the browser, and a forced document tier would still hand the stub to its handler.
- **Waiting a refresh's delay:** rejected. It slows every stub for nothing, since the site asked for the target page, not for the wait.
- **Keeping a cut body, marked incomplete:** rejected by G1. Every handler would have to check a flag, and one that forgets reads half a page as a whole one.
- **A cut body backing off the domain:** rejected. Like a 500, it usually belongs to one response, and a failing site already backs off through its refused and timed-out connections and 5xx.
- **Counting each transport resend as an attempt and a Politeness start:** rejected. The browser's own resends count as nothing inside one navigation, so the HTTP tiers would fail where the browser tier succeeds.
- **A setting to switch resends off:** rejected. The browser tier's resends cannot be switched off, and the HTTP tiers would differ from it.
- **Resending a non-read after a reused connection closed:** rejected by RFC 9110 §9.2.2. The site may have processed it.
- **Waiting out any `Retry-After`** (urllib3, up to six hours): rejected. One site's maintenance window would hold a Run for hours, with nothing to do.
- **Capping a `Retry-After` at `politeness.backoff.max` and sending anyway:** rejected by G5. It ignores what the site asked, as capping a Crawl-delay would (ADR 0086).
- **Skipping the held requests with a new reason:** rejected. A skip is a decision, so `retryFailed()` would not re-admit them once the site is back, and the site did not refuse the pages.
- **Stopping the whole crawl** (Crawlee's `PersistentRateLimitError`): rejected. One domain's wait would end the work on every other domain.
- **No jitter:** rejected. Domains behind one origin or proxy would come back together, the waves of the AWS study.
- **Full jitter:** rejected. It draws waits shorter than `politeness.backoff.initial`, which ADR 0086 counts as pressing a site harder. **Equal jitter:** rejected as AWS's weakest jittered form.
- **A backoff timer per retried request:** rejected. The domain's delay and backoff already space its retries, and a second layer of waits is the multiplied retry the Builders' Library warns about.
- **DNS answers kept for a fixed time** (Scrapy's cache has no documented TTL handling): rejected. A fixed time either outlives a record or queries the resolver for nothing.
- **Trying IPv6, then IPv4, one after the other:** rejected. A broken IPv6 path stalls each connection for a full connect timeout, the failure RFC 8305 exists to avoid.

## Consequences

- **Amends ADR 0020.** "The engine sets … redirects" is this ADR's rules on every tier: Fetch's statuses, method and header rules, `fetch.redirect.max`, refresh stubs, cut bodies and transport resends. A sample comparison whose browser attempt ended on another URL never matches. Notes sit under its redirects consequence and its sample comparison.
- **Amends ADR 0021.** The backoff's waits are drawn with decorrelated jitter between its first and longest wait. A `Retry-After` is honoured past `politeness.backoff.max`, up to `politeness.retry_after.max`, in both its forms. A note sits under its Politeness defaults.
- **Amends ADR 0086.** `politeness.retry_after.max` joins its Settings table, and is not a value that presses harder. An HTTP/2 `ENHANCE_YOUR_CALM` backs off like a 503. Each redirect hop, a browser-tier fetch's included, is a request start under the domain's limits and delay. Notes sit under "A browser-tier fetch counts as one request", "A failing site slows the crawl" and its Settings table.
- **Amends ADR 0087.** The navigation gate's permit is held per hop: each redirect or refresh hop takes its own, and the 3xx or refresh stub that ends a hop releases its permit, so only the last hop's permit lasts until the response arrives, or the load event on a browser-tier page. The load of an admitted crawl request takes no permit for its first hop, which admission gave, and one for each hop after it. A note sits under its navigation gate.
- **Amends ADR 0093.** Which responses reach a handler gains the 3xx rules, `too_many_redirects`, `response_truncated` and `navigation_failed` for transport failures. `fetch.redirect.max` and `fetch.refresh.max_delay` join its settings, and `response_unavailable` covers a request a long `Retry-After` held. Notes sit under its responses list, its settings and its Error codes.
- **Amends ADR 0085.** A navigation past the browser's 19 redirects raises `too_many_redirects`, a cut main document or body `response_truncated`, and `page.request`'s `maxRedirects` stops at 19. `body()`, `text()` and `json()`, which never failed on a kept body, raise `response_truncated` for a cut one. Notes sit under its Bodies bullet, its Navigation section and its Error codes table.
- **Adds to ADRs 0046 and 0105.** The Response cache never stores a cut response, and a Response recording keeps one with `WARC-Truncated: disconnect`, replayed as `response_truncated`. Notes sit under ADR 0046's list of what is never stored and ADR 0105's recording.
- **Adds to ADR 0092.** The details of `blocked` also carry `retryAfter`, the time a `rate_limit`'s `Retry-After` asks for, read in either form. A note sits under its Block cause bullet.
- **Adds to ADR 0102.** The wait reason `backoff` also covers a `Retry-After`'s wait, its setting `politeness.retry_after.max` when the site's time is the longer. A note sits under `CrawlRunProgress`.
- **Adds to ADR 0044.** The engine's resolver keeps answers for their TTL, and its HTTP transports race the Session's address families under RFC 8305. A note sits under its DNS consequence.
- **Error codes.** `too_many_redirects` (not retryable) and `response_truncated` (retryable) join `goodfetch-protocol` in the `page` group through `xtask codegen`'s naming checks (ADR 0079). They are Collie's codes, so Rust has `ErrorCode::TOO_MANY_REDIRECTS` and `ErrorCode::RESPONSE_TRUNCATED` (ADR 0099).
- **`TESTING.md`** gains the fetch edge conformance cases, and the in-process cases of the backoff's jitter and the resolver's cache, under its Time section.
- **Who builds what.**
  - **Area 5 (Site access)** builds the redirect rules, refresh stubs, cut-body detection and transport resends in each transport and the browser tiers.
  - **Area 3 (Engine runtime)** builds the jittered backoff, the `Retry-After` bound, the resolver's cache and Happy Eyeballs, and names their keys.
  - **Area 11 (Client surfaces)** builds the two classes and the new details in each SDK.
  - Each lists this ADR.
- **ADR 0037's matrix:** SA-24 (redirect control and limits) cites this ADR for its rules, SA-01 (plain HTTP fetching) for cut bodies and resends, and CO-09 (per-domain politeness) for the `Retry-After` bound and the jitter. Their statuses do not change.
- **Other tickets.**
  - [Proving ground: ported HTTP-incident, access-policy and identity pages](https://github.com/goodfetch/goodfetch/issues/162) gains the pages the cases need: each kind of cut body, GOAWAY, REFUSED_STREAM, a 421, `HTTP_1_1_REQUIRED`, `ENHANCE_YOUR_CALM`, a keep-alive connection closed at reuse, `Retry-After` in each form and on a 3xx, refresh stubs, and a host whose IPv6 path drops packets.
  - [Decoding a response the same way on every tier](https://github.com/goodfetch/goodfetch/issues/550) keeps the mislabelled `Content-Encoding` of row H25.
- **The `prototype/sdk-api` sketch** holds the two classes in the Collie client's `page` files, `retryAfter` on `BlockedError` and `ResponseUnavailableError`, the three settings in Foxhound's `settings` and `politeness` files, and a snippet in each language. Rows U12, U13, H02, H05, H07, H15 and H24 of its pitfall catalog close as Handled.
