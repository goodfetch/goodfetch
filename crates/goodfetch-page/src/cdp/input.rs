//! The primitives of input with no target (ADR 0085): the trusted events
//! a page's keyboard, mouse and touchscreen send for `page.keyboard`,
//! `page.mouse` and `page.touchscreen`, each carrying what the page's
//! [`Devices`] hold, which they update as they go out.

use goodfetch_cdp::generated::input::{
    DispatchMouseEventParams, DispatchMouseEventType, DispatchTouchEventParams,
    DispatchTouchEventType, MouseButton as Button, TouchPoint,
};
use goodfetch_engine::CallError;
use goodfetch_protocol::page::input::MouseButton;

use super::CdpPage;
use super::keys::{self, Key};
use crate::errors::Call;
use crate::gesture_order::Devices;

/// Which way a mouse button goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Press {
    /// Down, held until it comes up.
    Down,
    /// Up.
    Up,
}

impl CdpPage {
    /// Presses `keys` down in order, then up in the reverse order, as one
    /// keyboard press, with the keys `devices` hold: one burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it.
    pub(crate) async fn press_held(
        &self,
        keys: &[Key],
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let press = keys::press(keys, &mut devices.keys);
        self.press_keys(&press, call).await
    }

    /// Presses each of `keys` in turn, down then up, as typing them does,
    /// with the keys `devices` hold: one burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it.
    pub(crate) async fn press_each(
        &self,
        keys: &[Key],
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let events: Vec<_> = keys
            .iter()
            .flat_map(|key| keys::press(std::slice::from_ref(key), &mut devices.keys).0)
            .collect();
        self.page.burst(&events, call).await
    }

    /// Presses each of `keys` down, or releases it, in order, as
    /// [`Self::press_held`] does its half: one burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it.
    pub(crate) async fn keys_going(
        &self,
        (press, keys): (Press, &[Key]),
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let events: Vec<_> = keys
            .iter()
            .map(|key| match press {
                Press::Down => keys::down(key, &mut devices.keys),
                Press::Up => keys::up(key, &mut devices.keys),
            })
            .collect();
        self.page.burst(&events, call).await
    }

    /// Moves the mouse from where it is to `to` in `steps` moves, evenly
    /// spaced, the last on `to`, with the buttons and keys held down: one
    /// burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it.
    pub(crate) async fn mouse_move(
        &self,
        (to, steps): ((f64, f64), u32),
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let moves = moves(devices.mouse, to, steps)
            .map(|point| mouse(DispatchMouseEventType::MouseMoved, point, devices, None))
            .collect::<Vec<_>>();
        devices.mouse = to;
        self.page.burst(&moves, call).await
    }

    /// Presses `button` down where the mouse is, or releases it, as the
    /// `click_count`th of a run of clicks.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn mouse_button(
        &self,
        (press, button, click_count): (Press, MouseButton, u32),
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let event = pressing(press, button, click_count, devices);
        self.page.burst(&[event], call).await
    }

    /// Moves the mouse to `to` and clicks `button` there `click_count`
    /// times, each click's press and release counting from 1, as
    /// `UIEvent.detail` does: one burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it, so it
    /// leaves no button down.
    pub(crate) async fn mouse_click(
        &self,
        (to, button, click_count): ((f64, f64), MouseButton, u32),
        devices: &mut Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        devices.mouse = to;
        let mut events = vec![mouse(DispatchMouseEventType::MouseMoved, to, devices, None)];
        for count in 1..=click_count {
            events.push(pressing(Press::Down, button, count, devices));
            events.push(pressing(Press::Up, button, count, devices));
        }
        self.page.burst(&events, call).await
    }

    /// Turns the mouse wheel by `delta`, right and down in CSS pixels,
    /// where the mouse is.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn mouse_wheel(
        &self,
        delta: (f64, f64),
        devices: &Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let wheel = DispatchMouseEventParams {
            r#type: DispatchMouseEventType::MouseWheel,
            x: devices.mouse.0,
            y: devices.mouse.1,
            modifiers: held_modifiers(devices),
            timestamp: None,
            button: None,
            buttons: None,
            click_count: None,
            force: None,
            tangential_pressure: None,
            tilt_x: None,
            tilt_y: None,
            twist: None,
            delta_x: Some(delta.0),
            delta_y: Some(delta.1),
            pointer_type: None,
        };
        self.page.burst(&[wheel], call).await
    }

    /// Taps `at` with one finger: a touch starts there and ends, with the
    /// keys held down: one burst.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it, so it
    /// leaves no touch under way.
    pub(crate) async fn touch_tap(
        &self,
        at: (f64, f64),
        devices: &Devices,
        call: &Call,
    ) -> Result<(), CallError> {
        let modifiers = held_modifiers(devices);
        let touch = |kind, touch_points| DispatchTouchEventParams {
            r#type: kind,
            touch_points,
            modifiers,
            timestamp: None,
        };
        let point = TouchPoint {
            x: at.0,
            y: at.1,
            radius_x: None,
            radius_y: None,
            rotation_angle: None,
            force: None,
            tangential_pressure: None,
            tilt_x: None,
            tilt_y: None,
            twist: None,
            id: None,
        };
        let tap = [
            touch(DispatchTouchEventType::TouchStart, vec![point]),
            touch(DispatchTouchEventType::TouchEnd, Vec::new()),
        ];
        self.page.burst(&tap, call).await
    }
}

/// The points of a move from `from` to `to` in `steps` moves, evenly
/// spaced, the last on `to`; `0` moves once, as `1` does.
pub(crate) fn moves(
    from: (f64, f64),
    to: (f64, f64),
    steps: u32,
) -> impl Iterator<Item = (f64, f64)> {
    let steps = steps.max(1);
    (1..=steps).map(move |step| {
        let done = f64::from(step) / f64::from(steps);
        (
            (to.0 - from.0).mul_add(done, from.0),
            (to.1 - from.1).mul_add(done, from.1),
        )
    })
}

/// The bit of `button` in CDP's `buttons`.
const fn bit(button: MouseButton) -> i64 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

/// The press or release of `button` where the mouse is, the
/// `click_count`th of a run, which it records in `devices`.
fn pressing(
    press: Press,
    button: MouseButton,
    click_count: u32,
    devices: &mut Devices,
) -> DispatchMouseEventParams {
    let kind = match press {
        Press::Down => {
            devices.buttons |= bit(button);
            DispatchMouseEventType::MousePressed
        }
        Press::Up => {
            devices.buttons &= !bit(button);
            DispatchMouseEventType::MouseReleased
        }
    };
    let mut event = mouse(kind, devices.mouse, devices, Some(button));
    event.click_count = Some(i64::from(click_count));
    event
}

/// A mouse event of `kind` at `at`, with the buttons and keys `devices`
/// hold; `button` is the one it presses or releases, else the first held.
fn mouse(
    kind: DispatchMouseEventType,
    at: (f64, f64),
    devices: &Devices,
    button: Option<MouseButton>,
) -> DispatchMouseEventParams {
    let held = [MouseButton::Left, MouseButton::Right, MouseButton::Middle]
        .into_iter()
        .find(|held| devices.buttons & bit(*held) != 0);
    let button = match button.or(held) {
        Some(MouseButton::Left) => Button::Left,
        Some(MouseButton::Right) => Button::Right,
        Some(MouseButton::Middle) => Button::Middle,
        None => Button::None,
    };
    DispatchMouseEventParams {
        r#type: kind,
        x: at.0,
        y: at.1,
        modifiers: held_modifiers(devices),
        timestamp: None,
        button: Some(button),
        buttons: Some(devices.buttons),
        click_count: None,
        force: None,
        tangential_pressure: None,
        tilt_x: None,
        tilt_y: None,
        twist: None,
        delta_x: None,
        delta_y: None,
        pointer_type: None,
    }
}

/// CDP's `modifiers` of the keys `devices` hold, none when none is a
/// modifier.
fn held_modifiers(devices: &Devices) -> Option<i64> {
    Some(keys::modifiers(&devices.keys)).filter(|bits| *bits != 0)
}

#[cfg(test)]
mod tests;
