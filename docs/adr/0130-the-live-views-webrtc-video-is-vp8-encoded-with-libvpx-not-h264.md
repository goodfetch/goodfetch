# The Live view's WebRTC video is VP8, encoded with libvpx, not H.264

Every video the Live view sends over WebRTC is VP8, encoded by the engine with libvpx. This covers the virtual display captured at about 25 fps (ADR 0031's tier 3) and the CDP screencast relayed over WebRTC to a remote viewer (tier 2). H.264 is declined, because encoding it needs a patent licence that Goodfetch would have to sign and keep.

ADR 0031 named tier 3 "WebRTC H.264", after Steel, and left the encoder to the Console's spec. Writing that spec (V1 area 12) raised the open point of H.264's patents:
- **Cisco's coverage reaches only Cisco's binary, delivered on its terms.** Cisco pays the AVC pool's royalties for OpenH264 only for its own binary, downloaded separately to the end user's device and not combined with other software beforehand ([OpenH264 binary licence](https://www.openh264.org/BINARY_LICENSE.txt)). A build of OpenH264 compiled into Goodfetch's executable is not covered.
- **Downloading Cisco's binary at run time, as Firefox does, does not fit.** It would be a new download on ADR 0043's list, which needs a decision that changes ADR 0043, and Cisco's binary is a shared library, which Goodfetch's fully static musl Linux executable (ADR 0024) cannot load (ADR 0057 met the same limit with DuckDB's extensions).
- **A licence from the AVC patent pool (Via LA)** is a legal commitment, whatever royalty it charges at Goodfetch's volume. Goodfetch has no revenue and takes on nothing that may bill (ADR 0076).
- **Hardware and OS encoders do not cover tier 3's case.** Tier 3 runs in Linux containers, a Browser host and Goodfetch Cloud, which usually have no GPU.
- **VP8 asks no patent licence of Goodfetch.** libvpx is BSD-3-licensed and comes with Google's royalty-free patent grant ([LICENSE and PATENTS](https://chromium.googlesource.com/webm/libvpx/+/refs/heads/main)). RFC 7742 makes VP8 and H.264 the two video codecs a WebRTC browser must implement ([RFC 7742, section 5](https://www.rfc-editor.org/rfc/rfc7742#section-5)), so a browser that follows it decodes VP8.

Decided with the owner while grilling area 12's spec (Console) on 2026-10-16, as the answer to the open point on H.264 licensing.

## Decision

- **Codec.** The engine encodes every WebRTC video track of the Live view as VP8, with libvpx in its realtime mode. This covers tier 3's virtual display and tier 2's relay to a remote viewer.
- **Where it lives.** The encoder is part of the crate that implements the Live view's WebRTC side, which area 12's spec names. It runs its encoding threads through `tasks` (ADR 0060).
- **An encoder failure falls back as a capture failure does.** ADR 0031 falls back to tier 2 when capture fails or WebRTC cannot connect; a VP8 encoder that fails does the same in tier 3, and in tier 2's relay the remote viewer gets JPEG frames over the WebSocket.
- **Licences.** libvpx is listed in the third-party notices of the executables that link it, under ADR 0018. H.264 is recorded as declined: it needs a licence (ADR 0076).
- **Nothing else changes.** The tiers, their fallbacks, the frame rate and how a tier is chosen stay as ADR 0031 decides.

## Considered Options

- **H.264 from Goodfetch's own OpenH264 build:** declined. It needs the AVC pool's licence.
- **Cisco's OpenH264 binary downloaded at run time:** rejected. It needs a change to ADR 0043, fails offline, and a static musl executable cannot load it.
- **H.264 in a Private component (ADR 0018):** rejected. Keeping the encoder's code private changes nothing about the patent licence it needs.
- **OS and hardware encoders (VideoToolbox, Media Foundation, VA-API, NVENC):** rejected. They do not reach tier 3's GPU-less containers, and their licence coverage varies by vendor.
- **VP9 or AV1:** rejected for now. Both are royalty-free, but RFC 7742 does not require a WebRTC browser to implement them, so some viewers could not decode them.
- **No WebRTC tier until a licence is settled:** rejected. VP8 settles it.

## Consequences

- **ADR 0031** carries a note at tier 3.
- **ADR 0037.** Area 12 (Console) lists this ADR.
- **Size.** libvpx adds to the size of the executables that link the WebRTC side, counted against each Product's Size budget (ADR 0087). The first ticket that builds the encoder measures it.
- **Security.** libvpx is C running in the engine's process, and its VP8 encoder has had exploited flaws ([CVE-2023-5217](https://nvd.nist.gov/vuln/detail/CVE-2023-5217)). Its version is pinned and updated by the dependency updates every crate follows, and only frames the engine captured reach it.
- **ADR 0037's matrix:** CN-01 cites this ADR.
