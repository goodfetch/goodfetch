// The page scripts shared with the Node drivers (adapters/browser-scripts), embedded at build time.
use serde_json::Value;
use std::sync::OnceLock;

const DEEP: &str = include_str!("../../browser-scripts/deep.js");
const WAIT: &str = include_str!("../../browser-scripts/wait.js");
const GUARD: &str = include_str!("../../browser-scripts/guard.js");
const POINT: &str = include_str!("../../browser-scripts/point.js");
const FOCUS: &str = include_str!("../../browser-scripts/focus.js");
const SELECT: &str = include_str!("../../browser-scripts/select.js");
const ARM: &str = include_str!("../../browser-scripts/arm.js");
const FRAME_OFFSET: &str = include_str!("../../browser-scripts/frame-offset.js");
pub const DISARM: &str = include_str!("../../browser-scripts/disarm.js");
pub const CLICK_EVENTS: &[&str] = &["pointerdown", "mousedown", "pointerup", "mouseup", "click"];
pub const DBLCLICK_EVENTS: &[&str] = &["pointerdown", "mousedown", "pointerup", "mouseup", "click", "dblclick"];
pub const CONTEXTMENU_EVENTS: &[&str] = &["pointerdown", "mousedown", "pointerup", "mouseup", "contextmenu"];
pub const DRAG_EVENTS: &[&str] = &["pointerdown", "mousedown"];
pub const WHEEL_EVENTS: &[&str] = &["wheel"];
pub const RECORDS: &str = include_str!("../../browser-scripts/records.js");
pub const NEXT: &str = include_str!("../../browser-scripts/next.js");
pub const SNAPSHOT: &str = include_str!("../../browser-scripts/snapshot.js");
const CONDITIONS: &str = include_str!("../../browser-scripts/conditions.json");

fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.trim().to_string();
    for (key, value) in values {
        text = text.replace(&format!("__{key}__"), value);
    }
    text
}
fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("string serializes")
}
pub fn condition(name: &str, values: &[(&str, &str)]) -> String {
    static PARSED: OnceLock<Value> = OnceLock::new();
    let all = PARSED.get_or_init(|| serde_json::from_str(CONDITIONS).expect("conditions.json is valid"));
    fill(all[name].as_str().unwrap_or_else(|| panic!("unknown condition {name}")), values)
}
pub fn wait(condition: &str, timeout_ms: u64) -> String {
    fill(WAIT, &[("TIMEOUT", &timeout_ms.to_string()), ("CONDITION", condition)])
}
pub fn guard(selector: &str, timeout_ms: u64) -> String {
    fill(GUARD, &[("DEEP", DEEP.trim()), ("TIMEOUT", &timeout_ms.to_string()), ("SELECTOR", &quoted(selector))])
}
pub fn point(selector: &str) -> String {
    fill(POINT, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector))])
}
pub fn arm(selector: &str, types: &[&str]) -> String {
    let types = serde_json::to_string(types).expect("types serialize");
    fill(ARM, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector)), ("TYPES", &types)])
}
pub fn frame_offset(selector: &str) -> String {
    fill(FRAME_OFFSET, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector))])
}
pub fn focus(selector: &str) -> String {
    fill(FOCUS, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector))])
}
pub fn select(selector: &str, value: &str) -> String {
    fill(SELECT, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector)), ("VALUE", &quoted(value))])
}
