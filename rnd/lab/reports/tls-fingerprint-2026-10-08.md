# TLS and HTTP/2 fingerprints of October 8, 2026

ADR-002 contract on identity consistency, executed in `local-dev` on committed sources. Retained run: `tls-fingerprint-2026-10-08T19-32-26-736Z-f42991`, after fixing the JA4 computation (see the end of the report). 8 clients, 3 blocks. A TCP relay reads the ClientHello of each connection and computes its JA4; the HTTP/2 server records the settings, the pseudo-header order and the header order. The contract: a client keeps the same TLS and HTTP/2 identity throughout a session.

| Client | JA4 | Like Chrome (version, count, ciphers) | Chrome's HTTP/2 settings | Consistent sessions |
| --- | --- | --- | --- | --- |
| node-cdp | `t13d1518h2_8daaf6152771_4980c97edce0` | yes | yes | 3/3 |
| rust-cdp | `t13d1518h2_8daaf6152771_4980c97edce0` | yes | yes | 3/3 |
| impit-http | `t13d1516h2_8daaf6152771_afc95f72a295` | identical ciphers | no | 3/3 |
| crawlee-http | `t13d1513h2_8daaf6152771_ff9cead5a15b` then `t13d2313h2_24fc43eb1c96_ff9cead5a15b` | identical ciphers, then another list | no | 2/3 |
| wreq-http | `t13d2811h2_257f3020b3a2_78e6aca7449b` | no | no | 3/3 |
| reqwest-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | no | no | 3/3 |
| spider-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | no | no | 3/3 |
| node-http | `t13d5212h1_b262b3658495_8e6e362c5eac` | no | — | 3/3 |
- **Chrome** presents a single JA4, the same for both CDP drivers, from one connection, context and launch to the next. The order of its 18 extensions is randomized on each connection, which JA4 neutralizes by sorting them.
- **Impit**, in Chrome emulation, presents the same ciphers as Chrome, but two fewer extensions, and a different HTTP/2 header table size: its profile predates this Chrome.
- **Crawlee** (got-scraping) imitates Chrome's ciphers, but **changes TLS fingerprint in the middle of a session** in one block out of three: it goes from 15 to 23 cipher suites. This is the inconsistency that ADR-002 forbids.
- **wreq** without configured emulation presents a generic BoringSSL fingerprint. reqwest and Spider share the same rustls fingerprint. node-http (fetch) stays on HTTP/1.1.

## Correction of the evening of October 8

The first version of this report relied on `tls-fingerprint-2026-10-08T17-42-02-124Z-1dec60`. It concluded that Chrome changes JA4 from one launch to the next under the effect of its experiments. That was a computation defect:
- Chrome places a GREASE value at the head of its signature algorithms, drawn again for each connection;
- the lab's JA4 ignored it in the ciphers and extensions, but not in the signature algorithms.

The defect is fixed (`fixtures/tls-fingerprint.ts`, with a test). The previous run is classified `superseded`. The finding on Crawlee holds: the cipher list itself changes.

The relay also computes a base fingerprint without `pre_shared_key`. A resumed handshake (reused TLS session) does add this extension, and it is compared with the first handshake of the same client.

