# The Live view's WebRTC video is VP8, encoded with libvpx, not H.264

Every video the Live view sends over WebRTC is VP8, encoded by the engine with libvpx. This covers the virtual display captured at about 25 fps (ADR 0031's tier 3) and the CDP screencast relayed over WebRTC to a remote viewer (tier 2). H.264 is declined, because encoding it needs a patent licence that Goodfetch would have to sign and keep.

ADR 0031 named tier 3 "WebRTC H.264", noting that Steel moved from screencast to it, and chose no encoder. Writing the Console's spec (V1 area 12) raised the open point of H.264's patents:

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.

- **Cisco's coverage reaches only Cisco's binary, delivered on its terms.** Cisco pays the AVC pool's royalties for OpenH264 only for its own binary, downloaded separately to the end user's device and not combined with other software beforehand ([OpenH264 binary licence](https://www.openh264.org/BINARY_LICENSE.txt)). A build of OpenH264 compiled into Goodfetch's executable is not covered.
- **Downloading Cisco's binary at run time, as Firefox does, does not fit.** It would be a new download on ADR 0043's list, which needs a decision that changes ADR 0043, and Cisco's binary is a shared library, which Goodfetch's fully static musl Linux executable (ADR 0024) cannot load (ADR 0057 met the same limit with DuckDB's extensions).
- **A licence from the AVC patent pool (Via LA)** is a legal commitment, whatever royalty it charges at Goodfetch's volume. ADR 0076 creates no account or service that bills until the owner declares revenue.
- **OS and hardware encoders differ per target.** VideoToolbox, Media Foundation, VA-API and NVENC would each be a different encoder on a different target, where one codec on every target is wanted.
- **libvpx's licensors grant what VP8 needs.** libvpx is BSD-3-licensed, and Google grants a royalty-free licence to its patents that libvpx's VP8 implementation needs ([LICENSE](https://chromium.googlesource.com/webm/libvpx/+/refs/heads/main/LICENSE), [PATENTS](https://chromium.googlesource.com/webm/libvpx/+/refs/heads/main/PATENTS)). RFC 7742 makes VP8 and H.264 the two video codecs a WebRTC browser must implement ([RFC 7742, section 5](https://www.rfc-editor.org/rfc/rfc7742#section-5)), so a browser that follows it decodes VP8.

Decided with the owner while grilling area 12's spec (Console) on 2026-10-16, as the answer to the open point on H.264 licensing.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

## Decision

- **Codec.** The engine encodes every WebRTC video track of the Live view as VP8, with libvpx. This covers tier 3's virtual display and tier 2's relay to a remote viewer.
- **An encoder failure falls back as a capture failure does.** ADR 0031 falls back to tier 2 when capture fails or WebRTC cannot connect; a VP8 encoder that fails does the same in tier 3, and in tier 2's relay the remote viewer gets JPEG frames over the WebSocket.
- **Licences.** libvpx is listed in the third-party notices of the executables that link it, under ADR 0018. H.264 is recorded as declined: it needs a licence (ADR 0076).
- **Nothing else changes.** The tiers, their fallbacks, the frame rate and how a tier is chosen stay as ADR 0031 decides.

## Considered Options

- **H.264 from Goodfetch's own OpenH264 build:** declined. It needs the AVC pool's licence.
- **Cisco's OpenH264 binary downloaded at run time:** rejected. It needs a change to ADR 0043, fails offline, and a static musl executable cannot load it.
- **H.264 in a Private component (ADR 0018):** rejected. Keeping the encoder's code private changes nothing about the patent licence it needs.
- **OS and hardware encoders:** rejected. Each target would have its own encoder.
- **VP9 or AV1:** rejected for now. RFC 7742 does not require a WebRTC browser to implement them, so some viewers could not decode them.
- **No WebRTC tier until a licence is settled:** rejected. VP8 settles it.

## Consequences

- **ADR 0031** carries a note at tier 3.
- **ADR 0037.** Area 12 (Console) lists this ADR, and the matrix's CN-01 cites it.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Size.** libvpx adds to the size of the executables that link the WebRTC side, counted against each Product's Size budget (ADR 0087). The first ticket that builds the encoder measures it.
- **For the encoder's ticket.** libvpx is C in the engine's process and is fed pixels a page controls; its VP8 encoder has had an exploited flaw ([CVE-2023-5217](https://nvd.nist.gov/vuln/detail/CVE-2023-5217)). It meets `STYLE.md`'s bar for dependencies and `cargo-deny`'s checks like any other. libvpx starts threads of its own, which ADR 0060's `tasks` does not track, so the ticket keeps libvpx single-threaded unless a later decision allows its threads.
