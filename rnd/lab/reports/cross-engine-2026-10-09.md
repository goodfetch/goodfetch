# The Collie guard on Chromium, WebKit and Firefox, October 9, 2026

**Purpose.** Verify that the Collie guard protects actions on engines other than Chromium, going through Playwright for engines without CDP (Firefox, WebKit). This is the condition for Collie to drive these engines behind the same contract as its CDP driver. It is a test of action correctness, not of fingerprinting.

**Experimental design.**
- Three matrices: actionability (9 variants), primitives (10) and navigation, frames and files (7).
- Two policies: with the guard and native, that is, with Playwright's own checks only.
- Three blocks.
- Engines pinned by Playwright 1.63 under `lab/.browsers`: Chromium 153, WebKit 26.6 and Firefox 155.

**Retained runs.**

| System | Actionability | Primitives | Navigation |
| --- | --- | --- | --- |
| Linux container (arm64) | `cross-engine-actionability-2026-10-09T14-05-56-552Z-677518` | `cross-engine-primitives-2026-10-09T14-09-14-677Z-63256a` | `cross-engine-navigation-2026-10-09T14-12-41-527Z-cd6a35` |
| macOS 27.0.1 | `cross-engine-actionability-2026-10-09T13-59-29-588Z-e0a4f2` | `cross-engine-primitives-2026-10-09T14-02-07-047Z-53b744` | `cross-engine-navigation-2026-10-09T14-04-46-125Z-4d8f3c` |

## Results

| Engine | With the guard, Linux | With the guard, macOS | Native, Linux | Native, macOS |
| --- | --- | --- | --- | --- |
| Chromium | 78/78 | 78/78 | 78/78 | 78/78 |
| WebKit | 78/78 | 78/78 | 78/78 | 77/78 |
| Firefox | 78/78 | does not start | 78/78 | does not start |

- **The guard never let a wrong action through**, on all three engines and both systems: 390 cells out of 390. It is made of page scripts on the DOM and wraps Playwright's gestures without modification.
- **Natively, Playwright sometimes lets a click through on a target that is still moving.**
  - On these runs, it happened once with WebKit on macOS.
  - During development, it also happened with WebKit on Linux and with Chromium on Linux.
  - The defect is intermittent: its stability check can be outpaced by the movement.
  - The guard, which checks the target at the moment of the gesture and then intercepts the event, was never wrong in this case.
- **Firefox does not start on macOS 27.0.1.** Playwright's Firefox build stops on "Could not find profile folder", whatever the profile location, including outside the command sandbox. It works on Linux.

## Defects found along the way

- **Wheel oracle.** The fixture required 300 pixels of scrolling after a 600 wheel delta, which is Chromium's scale. Firefox scrolls 180 pixels per notch; WebKit reaches 600, but progressively. The oracle now judges the contract itself: a trusted `wheel` event on the area, followed by real scrolling. The runs made with the old oracle are classified `superseded`.
- **Native library in the image.** The Linux image does not contain the native SQLite library, which the lab loads at startup. The campaign script builds it first.

## What this changes for D-browser-driver

The browser contract can rely on two backends, with the same guard:
- **our CDP driver** for Chromium;
- **Playwright** for Firefox and WebKit.

For a product, the guard remains necessary even on top of Playwright: its own checks are not always enough on a moving target.
