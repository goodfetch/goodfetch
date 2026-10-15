# Lightpanda (D-light-engine), October 8, 2026

Lightpanda 1.0.0 (AGPL-3.0, pinned binary, always out of process), subjected to the same drivers, scenarios and oracles as Chromium, in `local-dev` on committed sources. The four drivers (Rust CDP, Node CDP, Puppeteer, Playwright) connect to it over CDP; their results are identical, so the failures come from the engine.

Retained runs: `light-engine-compat-native-2026-10-08T18-16-40-008Z-f659d1`, `light-engine-compat-guarded-2026-10-08T18-17-15-329Z-6d44f1`, `light-engine-actionability-2026-10-08T18-17-53-360Z-1d5ca3`, `light-engine-primitives-2026-10-08T18-19-11-980Z-d7f0e3`, `light-engine-navigation-2026-10-08T18-21-06-891Z-318afe`.

| Area | Works | Fails |
| --- | --- | --- |
| Reading | HTTP catalog, JavaScript-rendered catalog | — |
| Forms and clicks | form, repeated clicks, `isTrusted` events | shadow DOM (no driver finishes) |
| Primitives | double click, right click, wheel, Enter, Unicode input, moved target | hover, drag and drop (no pointer movement), `contenteditable` |
| Navigation | SPA route, file upload | history back, popup, same-origin and cross-origin iframe, download |
| Actionability | normal target; delayed, hidden or disabled target in most cells | **overlay, covered target, ambiguous target, moving target** |

- **False success under the guard.** Without a stacking model or hit-testing, `elementFromPoint` returns the target even under an overlay, and a duplicated target is not seen as ambiguous: the click goes through. The Collie guard cannot guarantee actionability on this engine.
- `retarget` fails because Lightpanda does not emit `pointerover`: the page never replaces the target, and the test proves nothing in this case.
- Contract conclusion: Lightpanda is suitable for reading simple pages, not for actions that require guarantees, nor for portals with iframes, popups or downloads. Its speed and resources remain to be measured on a quiet host.
