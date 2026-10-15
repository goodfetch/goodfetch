# Browser drivers: contracts of October 8, 2026

Contract matrices of D-browser-driver, executed in `local-dev` on committed sources. These are correctness results (class A): they do not depend on machine noise. No speed is compared here.

## Extended actionability matrix

Retained run: `browser-actionability-extended-2026-10-08T16-46-59-186Z-c5a02a` (it replaces `browser-actionability-extended-2026-10-08T12-05-27-529Z-49432b`, same results). 8 drivers, 9 variants, 2 policies, 3 blocks.

- All **guarded** drivers pass the 27 cells: Rust CDP, Node CDP, Puppeteer, Playwright, Stagehand, Stagehand batch, Crawlee Playwright and Crawlee Stagehand.
- **Natively**, Playwright and both Crawlee drivers pass 27 out of 27.
- Native Puppeteer fails on overlay, covered target, ambiguous target and re-render.
- The raw CDP drivers and both native Stagehand drivers have no actionability guarantee, and fail on all variants except `normal`.

## Primitives

Retained run: `browser-driver-primitives-2026-10-08T16-56-23-563Z-6a0bbd` (it replaces `browser-driver-primitives-2026-10-08T12-14-55-485Z-4b6074`, same results). 4 drivers, 10 gestures, 2 policies, 3 blocks.

- The guarded drivers and native Playwright succeed on all 30 cells.
- The native drivers without interception (Rust CDP, Node CDP and Puppeteer) fail on `moved` and `retarget`.

## Navigation, iframes and files

Retained run: `browser-navigation-frames-files-2026-10-08T16-44-06-166Z-d35973`. 6 drivers, 7 situations, 2 policies, 3 blocks.

- Rust CDP, Node CDP, Puppeteer and Playwright succeed on all 21 cells, natively as well as guarded: SPA route, history back, popup, same-origin iframe, cross-origin iframe, file upload and download.
- Stagehand and Crawlee Playwright do not offer these operations: their cells are unsupported.
- The first execution (`browser-navigation-frames-files-2026-10-08T16-26-23-841Z-410d88`) revealed a fragility in headless Chromium, which also affects Puppeteer on the same browser. Routed through the page, a click toward a freshly created cross-origin iframe can land on the parent `<iframe>` element. The CDP drivers now send this input to the iframe's own session, and the guard detects a pointer gesture that reached nothing.

## Two defects in the Collie guard, found and fixed

1. **No check during the gesture.** In `browser-driver-primitives-2026-10-08T11-42-40-378Z-c8b654`, guard v1 checked the target only before the gesture. When the page moved the target or replaced it with a "Delete" button as the pointer arrived, all guarded drivers clicked the decoy or the destructive control. Guard v2 arms a capture-phase listener on the verified element: an event that reaches anything else is canceled, then the gesture is replayed until the deadline.
2. **Frame pause after navigation.** Headless Chromium can suspend `requestAnimationFrame` for about one second after a navigation, while timers keep running. A guard paced by frames then exceeded its deadline on the moving target. It now samples on one frame or 20 ms, whichever comes first.

## What these results enable

The `actionability-conformance` and `false-success-count` gates of D-browser-driver now have evidence for the four compared options. The speed verdict awaits the RTT campaigns on a quiet machine.
