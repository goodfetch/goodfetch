# Collie's TLS relay links wreq's BoringSSL but not wreq, and forwards the browser's own HTTP/2

Collie's executable links BoringSSL for one purpose: the TLS relay that ADR 0042 builds for certificate exceptions in browser Sessions. It links BoringSSL through `btls`, `btls-sys` and `tokio-btls`, the BoringSSL fork that `wreq` itself uses, at `wreq`'s version, and only the relay's crate may depend on them. `wreq` stays banned from Collie, since the impersonated HTTP fetch tiers stay Foxhound's (ADR 0088). Upstream, the relay sends a ClientHello rebuilt from the one the browser sent it, offering the browser's own ALPN list. It then forwards the browser's own HTTP/2 (or HTTP/1.1) bytes unchanged, so it links no HTTP stack.

ADR 0042 built the relay "through wreq", ADR 0088 gave the relay to Collie, and ADR 0087 banned "`wreq` and its BoringSSL" from Collie's executable without weighing the relay. The three could not all hold. Primary sources show:
- **rustls cannot replace BoringSSL here.** It sends its own ClientHello: no GREASE, no ALPS, no extension permutation, and only its providers' ciphers and groups. The lab measured reqwest's rustls fingerprint as distinct from every browser's ([TLS fingerprint report](../../rnd/lab/reports/tls-fingerprint-2026-10-08.md)).
- **Cloudflare's `boring` alone cannot mirror every pinned browser.** Read in its published source (5.3.0), it sets ciphers, curves, signature algorithms, ALPN, GREASE, random extension permutation, certificate compression, OCSP and SCTs. It cannot set a fixed extension order (Firefox, Camoufox, Safari), the key shares sent, `record_size_limit` or delegated credentials (Firefox), or keep a TLS 1.3 cipher order as given. ALPS is reachable only through raw FFI. `btls` 0.5.6, "a hard fork of boring" by `wreq`'s author ([README](https://github.com/0x676e67/btls)), exposes all of these. Its `btls-sys` patches BoringSSL to add `SSL_CTX_set_extension_order`, `SSL_CTX_set_record_size_limit`, `SSL_CTX_set_delegated_credentials` and `SSL_CTX_set_preserve_tls13_cipher_list`.
- **One build cannot hold both.** `boring-sys` and `btls-sys` both declare `links = "boringssl"`, and Cargo "requires that there is at most one package per `links` value" ([build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html#the-links-manifest-key)). Foxhound links Collie's code and `wreq`, so a relay on `boring` would make Foxhound's build fail to resolve.
- **Upstream `h2` cannot rebuild a browser's HTTP/2 settings.** `h2` 0.4.20 sets SETTINGS values only. `wreq`'s fork `http2` adds `settings_order`, `headers_pseudo_order` and `priorities`. Forwarding the browser's own frames reproduces all of these exactly and needs neither crate.
- **The size fits.** `wreq` with `btls` and every emulation profile adds +5.49 MB raw ([binary-size research](https://github.com/goodfetch/goodfetch/blob/research/binary-size-budget/rnd/research/2026-10-12-binary-size-limits-and-costs.md) §3.1, quoted by [multi-product builds](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §7.1 and ADR 0088). BoringSSL alone is part of that.

Decided in [Collie's TLS relay needs BoringSSL, which ADR 0087 bans from Collie](https://github.com/goodfetch/goodfetch/issues/566), by the agent on the owner's delegation. The choice of BoringSSL over the rejected options was made by the orchestrator on the owner's delegation, and the owner was told and did not object. Each question, its options and the sources are in the ticket's [grilling record](https://github.com/goodfetch/goodfetch/issues/566#issuecomment-6211932547). It builds on [ADR 0042](0042-certificate-exceptions-per-domain-pinned-and-relayed-to-browsers.md), [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md) and [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md), decided in [#454](https://github.com/goodfetch/goodfetch/issues/454) and [#455](https://github.com/goodfetch/goodfetch/issues/455), and follows G5 of [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md): a certificate exception weakens nothing else. The Session keeps its TLS and HTTP/2 fingerprint toward the site, and the browser still verifies every other host.

## Decision

### What Collie links

- **BoringSSL, through `btls`, `btls-sys` and `tokio-btls`, for the TLS relay only.** The relay's crate is their one direct parent in Collie. Their version is the one `wreq` uses, so the workspace builds one BoringSSL and Foxhound links it once.
- **Not `wreq`, nor the crates only it uses** (`wreq-util`, `wreq-proto`, `wreq-rt`, `http2`). The HTTP and discovered-API fetch tiers and impersonated HTTP stay Foxhound's (ADR 0088). Collie sends a Session's HTTP only from inside the Session's browser.
- **A version bump of `wreq` that moves `btls` moves the relay's dependency in the same pull request.** The `links` rule leaves no other way to build.

### How the relay connects upstream

- **The ClientHello.** For each connection, the relay reads the browser's ClientHello before answering it, and sets the same values on its upstream connection through `btls`: cipher list (TLS 1.3 order kept), groups and the key shares sent, signature algorithms, ALPN, ALPS and its codepoint, certificate compression, OCSP and SCT requests, `record_size_limit` and delegated credentials when present, and GREASE. It sets the extension order the browser used, through `set_extension_permutation`. GREASE values are drawn fresh, as each browser draws its own.
- **The domain's rule.** The relay applies the trusted CAs, client certificate and pin (ADR 0042) through `btls`'s connector and its custom verify callback (`SSL_CTX_set_custom_verify`). The callback runs the same per-host decision as the engine's rustls verifier, so one implementation decides for both.
- **HTTP/2 and HTTP/1.1.** The relay finishes the browser's handshake with the protocol the site chose, then copies the decrypted bytes both ways. The site sees the browser's own SETTINGS, WINDOW_UPDATE, PRIORITY frames, pseudo-header order and HPACK use. The relay links no HTTP stack, and needs none: the rule, the pin and ADR 0017's address check all apply per connection.
- **The arbiter is ADR 0042's conformance comparison.** It compares the JA4 and HTTP/2 fingerprints of the browser and of the relay, for every pinned browser on every OS, now through Collie's executable. When a browser sends something BoringSSL cannot reproduce, that comparison fails, and the relay's spec or a later decision must deal with it. It is never waived.

### Size

- **The relay adds at most about +5.5 MB raw and +2.6 MB compressed to Collie.** The bound is the whole of `wreq` with `btls`. The compressed figure uses the ratio measured for rustls with aws-lc-rs (+2.82 MB raw, 1.33 MB compressed).
- **No ceiling moves.** After the relay, Collie's projection leaves about 10–17 MB compressed and 21–41 MB raw (with musl, as ADR 0087 counts its 13–20 MB and 27–47 MB) for the engine's own code, under its 50 MB and 120 MB ceilings.
- **The pull request that adds the relay trips the +1 MB increase check.** It passes as ADR 0057 says: with a "Size increase" section, giving the measured delta per target, that the fresh review accepts on its exact head.
- **`btls-sys`'s 5.16 MB `.crate` is its own crate.** It counts against crates.io's limit, not against the 8 MiB ceiling for `goodfetch-collie`'s tarball (ADR 0087).

## Considered Options

- **rustls in the relay:** rejected. The site would see a Rust TLS fingerprint instead of the browser's, which breaks ADR 0042 and changes a Session's identity whenever it has a certificate exception (ADR 0088).
- **A Foxhound-only relay:** rejected. It breaks ADR 0088. Collie would lose certificate exceptions in browser Sessions (parity row SC-06), and a Browser host run from Collie's executable would lose its relay (ADR 0096).
- **Linking `wreq` in Collie:** rejected. It breaks ADR 0088's split, in which impersonated HTTP is Foxhound's. It would also add `wreq`'s whole client to Collie's size when the relay needs only its TLS library.
- **Cloudflare's `boring` alone,** as area 4's draft spec proposed: rejected. It cannot mirror Firefox's, Camoufox's or WebKit's ClientHello, all of which the relay must send for a fully relayed Session (ADR 0042). Its `boring-sys` cannot share a build with `wreq`'s `btls-sys`.
- **Goodfetch's own BoringSSL patches:** rejected. They would be a third fork to keep current, beside `btls`, which Foxhound already depends on.
- **Rebuilding the HTTP/2 frames with `http2`:** rejected. It would put a second fork in Collie, and a rebuild only approaches what forwarding reproduces.
- **Upstream `h2`:** rejected. It cannot set the order of SETTINGS or of pseudo-headers, nor send PRIORITY frames.

## Consequences

- **Amends ADR 0087.** Collie's ban list keeps `wreq` and the crates only it uses. `btls`, `btls-sys` and `tokio-btls` are deny entries whose `wrappers` name the relay's crate as their only allowed direct parent; cargo-deny's `wrappers` "allows specific crates to have a direct dependency on the banned crate but denies all transitive dependencies on it" ([bans config](https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html)). A note sits at the ban list.
- **Amends ADR 0088.** Collie's ban list keeps out `wreq` but not its BoringSSL, which the relay links. A note sits at "Collie's `cargo-deny` ban list".
- **Amends ADR 0042.** The relay connects upstream through `btls`, not through `wreq`. It rebuilds the ClientHello from the browser's, and forwards the browser's own HTTP/2 frames instead of rebuilding them. A note sits at the relay's upstream item. ADR 0042's change to `wreq` for pins stays: it is for Foxhound's HTTP tiers.
- **Building Collie's engine from source** (`cargo install goodfetch-collie`, the Rust SDK crate; ADRs 0024 and 0094) now needs CMake, a C and C++ compiler, libclang and git, which `btls-sys`'s build script uses. Building Foxhound already needs them, through `wreq`. The Rust SDK's install docs list them.
- **ADR 0037.** SC-06 cites this ADR. Areas 1 and 4 list it.
- **Areas.**
  - Area 1 (Repository, CI and release) builds the ban list and its `wrappers`, and the Size budget check that records the relay's increase.
  - Area 4 (Security and egress) builds the relay's upstream connection: ClientHello mirroring through `btls`, the domain's rule in the custom verify callback, and the forwarding of the browser's HTTP bytes.
- **Glossary.** No term changes.
