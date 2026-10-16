# The Live view's WebRTC video is VP8, encoded with libvpx, not H.264

Every video the Live view sends over WebRTC is VP8, encoded by the engine with libvpx. This covers the virtual display captured at about 25 fps (ADR 0031's tier 3) and the CDP screencast relayed over WebRTC to a remote viewer (tier 2). H.264 is declined, because encoding it needs a patent licence that Goodfetch would have to sign and keep.

ADR 0031 named tier 3 "WebRTC H.264", after Steel, and left the encoder to the Console's spec. Writing that spec (V1 area 12) raised the open point of H.264's patents:
- **Cisco's coverage reaches only Cisco's binaries.** Cisco pays the AVC pool's royalties for OpenH264 only when users download the binary Cisco builds and hosts. A build of OpenH264 compiled into Goodfetch's executable is not covered.
- **Downloading Cisco's binary at run time is ruled out.** It is what Firefox does. For Goodfetch it would mean the engine fetching and running a binary from the internet, against ADR 0043. It would also fail offline, and Cisco's Linux builds link glibc, while Collie's Linux build is static with musl (ADR 0124).
- **A licence from the AVC patent pool (Via LA)** is a legal commitment, whatever royalty it charges at Goodfetch's volume. Goodfetch has no revenue and takes on nothing that may bill (ADR 0076).
- **Hardware and OS encoders do not cover tier 3's case.** Tier 3 runs in Linux containers, a Browser host and Goodfetch Cloud, which usually have no GPU.
- **VP8 needs no licence.** libvpx is BSD-3-licensed and comes with Google's royalty-free patent grant. RFC 7742 makes VP8 and H.264 the two video codecs every WebRTC browser must implement, so every browser that can open the Live view's WebRTC tier decodes VP8. At realtime settings, VP8 software encoding costs about the same CPU as H.264's, at close to the same quality for a screen.

Decided with the owner while grilling area 12's spec (Console) on 2026-10-16, as the answer to the open point on H.264 licensing.

## Decision

- **Codec.** The engine encodes every WebRTC video track of the Live view as VP8, with libvpx in its realtime mode. This covers tier 3's virtual display and tier 2's relay to a remote viewer.
- **Where it lives.** The encoder is part of the crate that implements the Live view's WebRTC side, which area 12's spec names. It runs its encoding threads through `tasks` (ADR 0060), and its failures, like the display capture's, lead to the fallback ADR 0031 already sets: tier 3 falls back to tier 2, and a remote viewer's WebRTC falls back to JPEG frames over the channel.
- **Licences.** libvpx is listed in the third-party notices of the executables that link it, under ADR 0018. H.264 is recorded as declined: it needs a licence (ADR 0076).
- **Nothing else changes.** The tiers, their fallbacks, the frame rate and how a tier is chosen stay as ADR 0031 decides.

## Considered Options

- **H.264 from Goodfetch's own OpenH264 build:** declined. It needs the AVC pool's licence.
- **Cisco's OpenH264 binary downloaded at run time:** rejected. It breaks ADR 0043, fails offline, and does not fit a static musl executable.
- **OS and hardware encoders (VideoToolbox, Media Foundation, VA-API, NVENC):** rejected. They do not reach tier 3's GPU-less containers, and their licence coverage varies by vendor.
- **VP9 or AV1:** rejected for now. Both are royalty-free, but software encoding at 25 fps costs several times VP8's CPU.
- **No WebRTC tier until a licence is settled:** rejected. VP8 settles it.

## Consequences

- **ADR 0031** carries a note at tier 3.
- **ADR 0037.** Area 12 (Console) lists this ADR.
- **Size.** libvpx adds to the size of the executables that link the WebRTC side, counted against each Product's Size budget (ADR 0087). The first ticket that builds the encoder measures it.
