//! The page crate's boundary with the browser's protocol (ADR 0146): a
//! concrete type, [`CdpPage`], whose narrow primitives a Page's logic
//! calls, in ADR 0129's shape, with no trait until a second implementation
//! is built. Spec #2256 moves the page crate's calls behind it ticket by
//! ticket, until this is the one module that names the CDP crate; until
//! then a primitive sends through the page's driver, whose lease check,
//! counted rows, Run trace and deadlines it keeps.
//!
//! Each primitive is shaped so that it fits `WebDriver BiDi` and Playwright's
//! driver wherever either has a counterpart, and one that neither has is
//! marked Chromium only (ADR 0146's note of 2026-11-11). The list, one row
//! a primitive:
//!
//! | Primitive | CDP | `WebDriver BiDi` | Playwright's driver |
//! |---|---|---|---|
//! | [`CdpPage::pointer_click`]: a left click at a point of the page's viewport, as trusted input | `Input.dispatchMouseEvent` ×3: `mouseMoved`, `mousePressed` and `mouseReleased` | one `input.performActions` with a `pointer` source: `pointerMove`, `pointerDown` and `pointerUp` | `mouseClick` |

use goodfetch_engine::CallError;

use crate::driver::{PageDriver, pointer_click};
use crate::errors::Call;

/// One page of a browser as the page's logic sees the protocol: the
/// primitives it calls on that page.
#[derive(Clone)]
pub(crate) struct CdpPage {
    /// The page's driver, as the call that sends counts and checks its
    /// commands.
    page: PageDriver,
}

impl CdpPage {
    /// The page `page` drives, its commands counted, leased and traced as
    /// `page`'s are.
    pub(crate) fn of(page: &PageDriver) -> Self {
        Self { page: page.clone() }
    }

    /// Clicks at (`x`, `y`) in the page's viewport with the pointer's left
    /// button: the pointer moves there, presses and releases, input the
    /// page receives as trusted. One primitive for the whole burst, which
    /// CDP sends as three events, each within `call`'s deadline and raising
    /// its codes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends none of the burst's later events; an event
    /// already written is the browser's.
    pub(crate) async fn pointer_click(&self, x: f64, y: f64, call: &Call) -> Result<(), CallError> {
        for params in pointer_click(x, y) {
            self.page.send(&params, call).await?;
        }
        Ok(())
    }
}
