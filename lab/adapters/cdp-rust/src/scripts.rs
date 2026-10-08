// The page scripts shared with the Node drivers (adapters/browser-scripts), embedded at build time.
use serde_json::Value;
use std::sync::OnceLock;

const DEEP: &str = include_str!("../../browser-scripts/deep.js");
const WAIT: &str = include_str!("../../browser-scripts/wait.js");
const GUARD: &str = include_str!("../../browser-scripts/guard.js");
const POINT: &str = include_str!("../../browser-scripts/point.js");
const FOCUS: &str = include_str!("../../browser-scripts/focus.js");
const SELECT: &str = include_str!("../../browser-scripts/select.js");
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
pub fn focus(selector: &str) -> String {
    fill(FOCUS, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector))])
}
pub fn select(selector: &str, value: &str) -> String {
    fill(SELECT, &[("DEEP", DEEP.trim()), ("SELECTOR", &quoted(selector)), ("VALUE", &quoted(value))])
}
