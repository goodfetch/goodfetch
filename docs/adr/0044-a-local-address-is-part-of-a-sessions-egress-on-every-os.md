# A local address is part of a Session's egress, on every OS

A run can send its traffic from chosen local addresses: a list of IP addresses, a network interface by name, or an IPv6 prefix. Each Session binds one address per IP family for its life, exactly as it binds one proxy, so a change of address opens a new Session (ADR 0021). DNS lookups and WebRTC follow the same address, browsers reach it through the engine's egress relay, and a replacement after a block draws from a different network block. Spider and Scrapy let HTTP requests choose their source address, and no browser tool offers it (parity row SA-25). Decided in [Network defaults](https://github.com/Jbmanllr/automan/issues/118).

Spider wraps reqwest's `local_address` and `interface`, and its interface option silently does nothing where the OS lacks support ([website.rs](https://github.com/spider-rs/spider/blob/main/spider/src/website.rs)). Scrapy binds per request through `meta["bindaddress"]` or globally through `DOWNLOAD_BIND_ADDRESS` ([docs](https://docs.scrapy.org/en/latest/topics/request-response.html)). Crawlee documents nothing. Chromium has no switch for the source address, so Playwright, Stagehand and Browser Use users resort to a local proxy or network namespaces. reqwest's `interface` works on Linux, Android, Fuchsia, Apple platforms, Solaris and illumos, but not Windows ([ClientBuilder](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)). Cloudflare's rate limiting counts "an individual IPv4 address or IPv6 /64 IP range" as one client ([docs](https://developers.cloudflare.com/waf/reference/legacy/old-rate-limiting)).

## Considered Options

- **Excluding local addresses**: rejected. It is a Spider and Scrapy capability with a real use: spreading traffic over a server's own addresses without paying for proxies.
- **HTTP tiers only** (Spider, Scrapy): rejected. Sites that need the browser tier would get nothing.
- **Interface names on Unix only, ignored or refused on Windows** (Spider): rejected. Windows can bind by the interface's address with the same effect.
- **A network namespace per browser**: rejected. It would keep HTTP/3, but it works only on Linux and needs privileges, while the relay works on every OS.
- **Random addresses inside the prefix on every rotation** (freebind, the Invidious rotator): rejected. A replacement in the same /64 is the same client to Cloudflare.

## Consequences

**Configuration.**
- `local_addresses` takes a list of addresses, an interface name or, on Linux, an IPv6 prefix. It is a Session egress option beside proxy pools (ADR 0021); a Session uses either a proxy or a local address, and through a proxy the local address binds the connection to the proxy.
- Interface names are resolved to their addresses on every OS with `netdev` (MIT, maintained, reports gateways). Linux, macOS and illumos also bind the socket to the device (`SO_BINDTODEVICE`, `IP_BOUND_IF`). Windows binds by address, which forces the interface because Windows sends under the strong host model by default ([Cable Guy](https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc137807(v=msdn.10))); the docs note that the interface needs its own route and that an administrator can change this with `WeakHostSend`.
- An IPv6 prefix on Linux binds any address in it through `IP_FREEBIND` (through `socket2`), with the route the docs describe. macOS and Windows have no equivalent, so there a prefix covers only the addresses assigned to the host, and a prefix with none is refused with the Error code `local_address_unavailable`. Nothing is ever silently ignored.
- A Session binds one IPv4 and one IPv6 address when the pool has both. A target with no address in a family the Session holds fails with `address_family_unavailable`.

**Leaks.**
- DNS queries for a Session with a local address leave from that address through the engine's own resolver (ADR 0024), on every OS.
- **Changes ADR 0024:** the engine's own resolver, used on Linux by default, is also used on macOS and Windows for Sessions with a local address.
- Browsers hold WebRTC to `disable_non_proxied_udp` (`--force-webrtc-ip-handling-policy`), so it never reveals the host's other addresses; ADR 0014's interception of WebRTC still applies.

**Browsers.**
- A browser Session with a local address sends its traffic through the engine's egress relay (ADR 0017), bound to that address. The relay tunnels and leaves the TLS fingerprint unchanged; ADR 0042's terminating mode applies only to hosts with a certificate exception.
- Chrome uses no HTTP/3 through a proxy ([http_stream_factory_job.cc](https://chromium.googlesource.com/chromium/src/+/main/net/http/http_stream_factory_job.cc)), so such a Session speaks HTTP/2, like any proxied Session.

**Rotation.**
- When a block retires a Session (ADR 0021), its replacement prefers an address outside every /64 (IPv6) or /24 (IPv4) blocked on that domain during the run. When the pool has no other block, it rotates within the block and the run notes it.
- New Sessions with no block involved spread across blocks round-robin. Both block sizes are settings.
- Rotating local addresses on a block carries the legal rating of IP rotation on block ([legal exposure](https://github.com/Jbmanllr/automan/issues/65)) and follows ADR 0035's placement of it.

**Unchanged.** ADR 0017's resolved-IP check and ADR 0021's `per_ip_concurrency`, which is keyed by the target's IP, apply as before.
