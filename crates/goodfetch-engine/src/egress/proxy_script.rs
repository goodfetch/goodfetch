//! The domain policy as a browser's proxy auto-config script (ADR 0014): a
//! request no interception sees, a WebSocket's handshake or a WebRTC
//! connection over TCP, still asks the browser's proxy resolution where to
//! connect, so the browser itself sends one to a host the policy refuses to
//! a proxy that cannot be reached, and it never leaves; one to a host the
//! policy allows goes direct. The script judges a host as [`super::judge`]
//! does, on the host as the URL parser leaves it: its lists hold each entry
//! as [`Policy`] read it, a name matching itself and its subdomains, an
//! address only itself, one trailing dot dropped, a blocked entry winning.
//!
//! The script holds data and one fixed function, which throws on no input:
//! Chromium connects directly when a script fails, so nothing in it may.

use std::fmt::Write as _;

use super::{Entries, Policy};

/// Where a refused connection goes: a name under `.invalid`, which the
/// browser resolves to no address (RFC 6761, section 6.4), so it fails.
const REFUSED: &str = "PROXY egress-refused.invalid:9";

/// The script's fixed part: `ALLOWED` and `BLOCKED` are set before it.
const JUDGE: &str = r#"
function listed(list, host) {
  if (list.every) return true;
  if (list.addresses[host] === true) return true;
  var name = host;
  for (;;) {
    if (list.domains[name] === true) return true;
    var dot = name.indexOf(".");
    if (dot < 0) return false;
    name = name.substring(dot + 1);
  }
}
function FindProxyForURL(url, host) {
  var name = String(host).toLowerCase();
  if (name.charAt(0) === "[" && name.charAt(name.length - 1) === "]") {
    name = name.substring(1, name.length - 1);
  }
  if (name.charAt(name.length - 1) === ".") {
    name = name.substring(0, name.length - 1);
  }
  if (listed(BLOCKED, name)) return REFUSED;
  if (listed(ALLOWED, name)) return "DIRECT";
  return REFUSED;
}
"#;

impl Policy {
    /// The proxy auto-config script that holds the browser's own
    /// connections to this domain policy; `None` when it allows every host
    /// and blocks none, so nothing needs holding.
    #[must_use]
    pub fn proxy_script(&self) -> Option<String> {
        if self.allowed.every_host && self.blocked.is_empty() {
            return None;
        }
        let mut script = String::new();
        let _ = writeln!(script, "var REFUSED = {};", quoted(REFUSED));
        let _ = writeln!(script, "var ALLOWED = {};", self.allowed.as_script());
        let _ = writeln!(script, "var BLOCKED = {};", self.blocked.as_script());
        script.push_str(JUDGE);
        Some(script)
    }
}

impl Entries {
    /// Whether the list holds no entry.
    fn is_empty(&self) -> bool {
        !self.every_host && self.domains.is_empty() && self.addresses.is_empty()
    }

    /// The list as the script's object: `every`, and maps without a
    /// prototype from each name and each address, as the URL parser
    /// writes it, to `true`.
    fn as_script(&self) -> String {
        let mut domains: Vec<&str> = self.domains.iter().map(AsRef::as_ref).collect();
        domains.sort_unstable();
        let mut addresses: Vec<String> = self
            .addresses
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        addresses.sort_unstable();
        format!(
            "{{ every: {}, domains: {}, addresses: {} }}",
            self.every_host,
            set(&domains),
            set(&addresses),
        )
    }
}

/// A map without a prototype from each of `keys` to `true`, so no key such
/// as `__proto__` or `constructor` reads anything else.
fn set(keys: &[impl AsRef<str>]) -> String {
    let mut set = String::from("(function () { var set = Object.create(null);");
    for key in keys {
        let _ = write!(set, " set[{}] = true;", quoted(key.as_ref()));
    }
    set.push_str(" return set; })()");
    set
}

/// `text` as a JavaScript string literal.
fn quoted(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}
