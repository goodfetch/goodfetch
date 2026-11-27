//! Input with no target (ADR 0085): `page.keyboard`, `page.mouse` and
//! `page.touchscreen`, for a canvas, a map or a key for whatever has focus.
//! Each call is a Gesture of its page, sent in its turn of the page's
//! Gesture order, with what the page's keyboard and mouse hold from the
//! calls before. No guard checks it, since it names no element: its
//! caller marks it unguarded in Provenance. It marks the page's send
//! window before its first input (ADR 0106) and waits for no navigation
//! it starts, as Playwright's do.

use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::page::input::MouseButton;

use crate::cdp::driver::PageDriver;
use crate::cdp::keys::{self, Key};
use crate::cdp::{CdpPage, Press};
use crate::errors::Call;
use crate::guard::SentMark;
use crate::page_owner::PageOwner;

/// One call's input, its keys named and its counts checked.
pub(crate) enum Input {
    /// `page.keyboard.down` and `page.keyboard.up`: one key pressed down,
    /// or released.
    Key(Press, Key),
    /// `page.keyboard.press`: keys pressed down in order and released in
    /// the reverse order, `delay` apart.
    Press { keys: Vec<Key>, delay: Duration },
    /// `page.keyboard.type`: a text typed a character at a time, each key
    /// held `delay`.
    Type { text: String, delay: Duration },
    /// `page.keyboard.insert_text`: a text inserted whole.
    InsertText(String),
    /// `page.mouse.move`: the mouse moved to a point in some steps.
    Move { to: (f64, f64), steps: u32 },
    /// `page.mouse.down` and `page.mouse.up`: a button pressed down or
    /// released where the mouse is, the nth of a run of clicks.
    Button {
        press: Press,
        button: MouseButton,
        click_count: u32,
    },
    /// `page.mouse.click` and `page.mouse.dblclick`: clicks at a point,
    /// each button held `delay`, and `delay` between two.
    Click {
        at: (f64, f64),
        button: MouseButton,
        click_count: u32,
        delay: Duration,
    },
    /// `page.mouse.wheel`: the wheel turned where the mouse is.
    Wheel((f64, f64)),
    /// `page.touchscreen.tap`: a tap at a point.
    Tap((f64, f64)),
}

/// Sends `input`, the input of the call `operation`, to the page `page`
/// drives, in its turn of the Gesture order `owner` keeps, after `sent`
/// marks the page's send window; a delay waits on `clock`.
///
/// # Errors
///
/// The page's owner's failure while it waits its turn, a failed mark, or
/// the browser's failure as the call's code.
///
/// # Cancel safety
///
/// Dropping the future between two of a delayed input's sends leaves what
/// went out so: a key or a button pressed down stays down, as the page's
/// keyboard and mouse record.
pub(crate) async fn send(
    (page, owner, sent): (&PageDriver, &PageOwner, SentMark<'_>),
    (operation, input): (&'static str, Input),
    clock: &Clock,
) -> Result<(), CallError> {
    let mut turn = owner.turn().await?;
    sent.mark(operation).await?;
    let cdp = CdpPage::of(page);
    let call = Call::Action { operation };
    let devices = turn.devices();
    match input {
        Input::Key(press, key) => cdp.keys_going((press, &[key]), devices, &call).await,
        Input::Press { keys, delay } if delay.is_zero() => {
            cdp.press_held(&keys, devices, &call).await
        }
        Input::Press { mut keys, delay } => {
            cdp.keys_going((Press::Down, &keys), devices, &call).await?;
            clock.sleep(delay).await;
            keys.reverse();
            cdp.keys_going((Press::Up, &keys), devices, &call).await
        }
        Input::Type { text, delay } if delay.is_zero() => {
            // The keys of each run of characters a key types go out as
            // one burst; any other character is inserted between two.
            let mut run = Vec::new();
            for character in text.chars() {
                if let Some(key) = keys::typed(character) {
                    run.push(key);
                    continue;
                }
                if !run.is_empty() {
                    cdp.press_each(&run, devices, &call).await?;
                    run.clear();
                }
                cdp.insert_text(character.encode_utf8(&mut [0; 4]), &call)
                    .await?;
            }
            if run.is_empty() {
                Ok(())
            } else {
                cdp.press_each(&run, devices, &call).await
            }
        }
        Input::Type { text, delay } => {
            for character in text.chars() {
                if let Some(key) = keys::typed(character) {
                    let key = std::slice::from_ref(&key);
                    cdp.keys_going((Press::Down, key), devices, &call).await?;
                    clock.sleep(delay).await;
                    cdp.keys_going((Press::Up, key), devices, &call).await?;
                } else {
                    clock.sleep(delay).await;
                    cdp.insert_text(character.encode_utf8(&mut [0; 4]), &call)
                        .await?;
                }
            }
            Ok(())
        }
        Input::InsertText(text) => cdp.insert_text(&text, &call).await,
        Input::Move { to, steps } => cdp.mouse_move((to, steps), devices, &call).await,
        Input::Button {
            press,
            button,
            click_count,
        } => {
            cdp.mouse_button((press, button, click_count), devices, &call)
                .await
        }
        Input::Click {
            at,
            button,
            click_count,
            delay,
        } if delay.is_zero() => {
            cdp.mouse_click((at, button, click_count), devices, &call)
                .await
        }
        Input::Click {
            at,
            button,
            click_count,
            delay,
        } => {
            cdp.mouse_move((at, 1), devices, &call).await?;
            for count in 1..=click_count {
                if count > 1 {
                    clock.sleep(delay).await;
                }
                cdp.mouse_button((Press::Down, button, count), devices, &call)
                    .await?;
                clock.sleep(delay).await;
                cdp.mouse_button((Press::Up, button, count), devices, &call)
                    .await?;
            }
            Ok(())
        }
        Input::Wheel(delta) => cdp.mouse_wheel(delta, devices, &call).await,
        Input::Tap(at) => cdp.touch_tap(at, devices, &call).await,
    }
}
