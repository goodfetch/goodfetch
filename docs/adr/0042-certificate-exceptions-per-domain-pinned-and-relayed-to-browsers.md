# Certificate exceptions are per domain, pinned on first use, and reach browsers through the engine's TLS relay

Every tier checks TLS certificates by default. Three settings make exceptions, each scoped to the domains it names: `trusted_cas` (extra certificate authorities), `client_certificates` (mutual TLS) and `accept_invalid_certs` (a certificate that fails verification). An accepted invalid certificate is pinned: its public key is remembered per user, as SSH's `known_hosts` remembers hosts, and a different key stops that host until a person accepts it. Browsers cannot scope these exceptions per host, so the engine's egress relay terminates TLS for the listed hosts only and connects upstream with the Session's own browser fingerprint. ADR 0016 locks every security control that costs nothing at run time; this decision keeps TLS checks locked everywhere except on the domains a user names, because fully locked verification would lose the broken-certificate sites that Crawlee and Scrapy collect without asking (parity row SC-06). Decided in [Network defaults](https://github.com/Jbmanllr/automan/issues/118).

Competitors either verify nothing by default or offer one switch for everything:

- Crawlee JS ignores TLS errors by default in its HTTP crawlers (`ignoreSslErrors: true` in 3.x, `ignoreTlsErrors` in 4.0; [http-crawler.ts](https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts)). So does Scrapy (`DOWNLOAD_VERIFY_CERTIFICATES=False`, which since 2.16 also skips HTTPS proxy certificates; [security docs](https://docs.scrapy.org/en/latest/topics/security.html)), and so does Firecrawl unless a request sets headers or actions (`skipTlsVerification`; [types.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/controllers/v2/types.ts)).
- Crawlee Python, Spider, Playwright, Stagehand and Browser Use verify by default and turn verification off for a whole client or browser context. Browser Use's switch also disables web security ([profile.py](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/profile.py)).
- No surveyed tool scopes an exception to a domain, pins what it accepted, or logs that verification was off.

Browsers offer no stable per-host control. Chromium's `--ignore-certificate-errors-spki-list` is browser-wide and skips all verification once any certificate the server sends matches ([ignore_errors_cert_verifier.cc](https://chromium.googlesource.com/chromium/src/+/main/services/network/ignore_errors_cert_verifier.cc)); CDP's per-error decisions (`Security.setOverrideCertificateErrors`, `handleCertificateError`) still work but are deprecated ([protocol](https://github.com/ChromeDevTools/devtools-protocol/blob/master/json/browser_protocol.json)). Playwright gives Firefox and WebKit one switch per browser context ([protocol.d.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/webkit/protocol.d.ts)). No browser selects a client certificate except from the OS store (Chromium's `AutoSelectCertificateForUrls` policy). Playwright's `clientCertificates` works through a local proxy that terminates TLS in Node, so the site sees Node's ClientHello, and it turns verification off for the whole browser context ([socksClientCertificatesInterceptor.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/socksClientCertificatesInterceptor.ts), [browser.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/browser.ts)).

## Considered Options

- **TLS checks locked, with extra trusted CAs only**, as ADR 0016's rule for controls with no run-time cost implies: rejected. Sites with expired certificates or missing intermediates would be lost to Goodfetch while Crawlee JS and Scrapy collect them by default.
- **One switch per run** (Spider, Playwright): rejected. A switch flipped for one broken site would expose every Session's credentials and forms to a man in the middle.
- **Accepting any invalid certificate from a listed host**: rejected. It accepts an attacker on that host's path as readily as the broken site; trust on first use stops a later substitution.
- **CDP per-error decisions for Chromium**: rejected. The methods are deprecated, they cover neither client certificates nor Firefox and WebKit, and a second mechanism would still be needed for those.
- **Playwright's approach for client certificates** (terminate TLS in a runtime with its own TLS stack): rejected. The site sees a different fingerprint from the browser's, and the browser stops verifying every other host of the browser context.
- **Installing CAs or client certificates in the OS store**: rejected. It changes the user's machine for every application and outlives the run.

## Consequences

**Settings.**
- `trusted_cas`, `client_certificates` and `accept_invalid_certs` are maps from a domain (a host name, or a registrable domain as in ADR 0004 with its subdomains) to a value: certificate files or Secret references for the first two (ADR 0015), a pin or `any` for the third. They are settings through ADR 0028's layers, so Operator policy can forbid or lock them.
- They apply only to addresses a page can reach (ADR 0017). Endpoints the operator configures (Sinks, Model providers, vaults, proxies, webhooks) take only their own trusted CA and client certificate and never accept an invalid certificate.
- No Plugin can set or widen them (ADR 0033), and a Plugin's own connections never benefit from them.

**Pinning invalid certificates.**
- The first invalid certificate accepted for a listed host is pinned by the SHA-256 hash of its public key in a per-user known-certificates store, beside the Usage ledger (ADR 0041), and recorded in the Audit log.
- Later connections to that host, in any run of that user, accept only that key. A different key stops that host with the Error code `certificate_changed`, which carries both hashes, until a person accepts the new key with `goodfetch certs accept` or in the Console. An agent cannot accept one.
- A pin can be given in advance (`accept_invalid_certs: { "intranet.example": "sha256/…" }`), which skips trust on first use. The value `any` accepts every certificate from that host, for test fixtures that regenerate theirs, and is recorded in the Audit log on each run that uses it.
- The store partitions per person on a shared Daemon, as ADR 0031's accounts do.

**Guards.**
- The engine refuses to type a secret into, or send a Session's credentials or cookies to, a host reached through `accept_invalid_certs`, as ADR 0017 refuses with raw CDP attached.
- Each Record fetched under an exception is marked in Provenance with the exception's kind and the certificate hash.
- The Audit log records each run's exceptions, every first pin, every accepted change and every use of `any`.

**HTTP tiers.**
- reqwest runs a rustls certificate verifier that decides per host: the default roots, the domain's trusted CAs, or the domain's pin.
- wreq cannot hand each connection's verification to the caller, but takes a certificate store, a client identity and a verification switch per client ([ClientBuilder](https://docs.rs/wreq/latest/wreq/struct.ClientBuilder.html)). The engine keeps one wreq client per trust scope of a Session and routes each request by host. The emulation profile is the same in every scope, so a site cannot tell them apart. A pin cannot be expressed as a certificate store, so Goodfetch adds per-client custom verification to wreq on top of BoringSSL's custom verification (`SSL_CTX_set_custom_verify`), which checks the presented key against the pin during the handshake, before any request is sent. The spec settles whether wreq takes the change upstream or Goodfetch carries it.

**The TLS relay for browsers.**
- **Changes ADR 0017:** the engine-owned egress proxy, which only tunnels, gains a terminating mode used only for hosts with an exception. Every other host is still tunnelled, and the browser verifies it itself.
- Upstream, the relay applies the domain's rule (trusted CAs, client certificate, pin) and connects through wreq with the TLS ClientHello and HTTP/2 settings captured from the Session's own browser build (ADR 0035), so the site sees that browser's fingerprint, unlike Playwright's.
- Toward the browser, the relay presents certificates from a CA generated at each browser launch, held only in the engine's memory and name-constrained to the listed hosts:
  - **Chromium** trusts it through `--ignore-certificate-errors-spki-list` with the CA's key hash. Since a match skips all verification, the key is never written to disk and changes with every launch.
  - **Firefox** trusts it through the `Certificates.Install` policy: Playwright's Firefox reads `PLAYWRIGHT_FIREFOX_POLICIES_JSON` ([bootstrap.diff](https://github.com/microsoft/playwright/blob/main/browser_patches/firefox/patches/bootstrap.diff)), and for Camoufox the entry is merged into its own `distribution/policies.json` in the browser cache copy. NSS enforces the name constraints ([pkixbuild.cpp](https://github.com/mozilla-firefox/firefox/blob/main/security/nss/lib/mozpkix/lib/pkixbuild.cpp)). The policy installs asynchronously, so the engine confirms the CA is trusted before the Session's first navigation.
  - **WebKit** cannot trust an added CA on macOS or Windows, and on Linux only in developer builds. A WebKit Session with any exception therefore sends every host through the relay, which verifies each one upstream (the default roots for unlisted hosts) with the Safari ClientHello and HTTP/2 settings captured from Goodfetch's WebKit build. The browser's certificate switch then covers only its loopback link to the relay.
- Chrome uses no HTTP/3 through a proxy, so listed hosts are reached over HTTP/2, as with any proxied Session.
- A Session with an exception reports which hosts the relay terminated, and the V1 benchmark (ADR 0025) publishes the relay's cost.
