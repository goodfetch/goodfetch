//! MIT-licensed scriptlet templates for `filter_lists.page_scriptlets` (ADR
//! 0047): registered with each `adblock::Engine` as [`ResourceType::Template`]
//! resources so `+js(...)` filters resolve into main-world scripts only when
//! `filter_lists.page_scriptlets: true` is enabled. Each resource uses the
//! function-style syntax `adblock`'s resource assembler recognises and wraps
//! each invocation in `try { … } catch ( e ) { }`.

use adblock::resources::{PermissionMask, Resource, ResourceType};
use data_encoding::BASE64;

/// One scriptlet template.
struct Template {
    /// Its primary `.js` resource name, as `adblock` looks up `+js(<name>)`.
    name: &'static str,
    /// Alternative names lists use for the same scriptlet.
    aliases: &'static [&'static str],
    /// Its JavaScript function definition.
    body: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "set-constant.js",
        aliases: &["set.js", "ubo-set-constant.js", "ubo-set.js"],
        body: r#"function gfSetConstant(chain, raw) {
  let val;
  if (raw === undefined || raw === "undefined") { val = undefined; }
  else if (raw === "false") { val = false; }
  else if (raw === "true") { val = true; }
  else if (raw === "null") { val = null; }
  else if (raw === "''" || raw === "") { val = ""; }
  else if (raw === "emptyObj") { val = {}; }
  else if (raw === "emptyArr") { val = []; }
  else if (raw === "noopFunc") { val = function() {}; }
  else if (raw === "trueFunc") { val = function() { return true; }; }
  else if (raw === "falseFunc") { val = function() { return false; }; }
  else if (/^-?\d+(\.\d+)?$/.test(raw)) { val = Number(raw); }
  else { val = raw; }
  const parts = String(chain || "").split(".").filter(Boolean);
  if (parts.length === 0) return;
  let obj = window;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (obj[key] == null || typeof obj[key] !== "object") { obj[key] = {}; }
    obj = obj[key];
  }
  const leaf = parts[parts.length - 1];
  try {
    Object.defineProperty(obj, leaf, {
      configurable: true,
      get() { return val; },
      set() {},
    });
  } catch (_) {
    obj[leaf] = val;
  }
}"#,
    },
    Template {
        name: "abort-on-property-read.js",
        aliases: &["aopr.js", "ubo-abort-on-property-read.js", "ubo-aopr.js"],
        body: r#"function gfAbortOnPropertyRead(chain) {
  const parts = String(chain || "").split(".").filter(Boolean);
  if (parts.length === 0) return;
  const tag = "gf-aopr-" + Math.random().toString(36).slice(2);
  let obj = window;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (obj[key] == null || typeof obj[key] !== "object") { obj[key] = {}; }
    obj = obj[key];
  }
  const leaf = parts[parts.length - 1];
  Object.defineProperty(obj, leaf, {
    configurable: true,
    get() { throw new ReferenceError(tag); },
    set() {},
  });
}"#,
    },
    Template {
        name: "abort-on-property-write.js",
        aliases: &["aopw.js", "ubo-abort-on-property-write.js", "ubo-aopw.js"],
        body: r#"function gfAbortOnPropertyWrite(chain) {
  const parts = String(chain || "").split(".").filter(Boolean);
  if (parts.length === 0) return;
  const tag = "gf-aopw-" + Math.random().toString(36).slice(2);
  let obj = window;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (obj[key] == null || typeof obj[key] !== "object") { obj[key] = {}; }
    obj = obj[key];
  }
  const leaf = parts[parts.length - 1];
  Object.defineProperty(obj, leaf, {
    configurable: true,
    get() { return undefined; },
    set() { throw new ReferenceError(tag); },
  });
}"#,
    },
    Template {
        name: "abort-current-inline-script.js",
        aliases: &[
            "acis.js",
            "ubo-abort-current-inline-script.js",
            "ubo-acis.js",
        ],
        body: r#"function gfAbortCurrentInlineScript(chain, needle) {
  const parts = String(chain || "").split(".").filter(Boolean);
  if (parts.length === 0) return;
  const tag = "gf-acis-" + Math.random().toString(36).slice(2);
  let obj = window;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (obj[key] == null || typeof obj[key] !== "object") { obj[key] = {}; }
    obj = obj[key];
  }
  const leaf = parts[parts.length - 1];
  let current = obj[leaf];
  Object.defineProperty(obj, leaf, {
    configurable: true,
    get() {
      const s = document.currentScript;
      if (s && s.src === "" && (!needle || (s.textContent || "").includes(needle))) {
        throw new ReferenceError(tag);
      }
      return current;
    },
    set(v) { current = v; },
  });
}"#,
    },
    Template {
        name: "no-setTimeout-if.js",
        aliases: &["nostif.js", "ubo-no-setTimeout-if.js", "ubo-nostif.js"],
        body: r#"function gfNoSetTimeoutIf(needle, delay) {
  const orig = window.setTimeout;
  const matchDelay = delay !== undefined && delay !== "" ? Number(delay) : null;
  window.setTimeout = function(fn, ms, ...rest) {
    const text = typeof fn === "function" ? fn.toString() : String(fn);
    const hitNeedle = !needle || text.includes(needle);
    const hitDelay = matchDelay === null || Number(ms) === matchDelay;
    if (hitNeedle && hitDelay) return 0;
    return orig.call(this, fn, ms, ...rest);
  };
}"#,
    },
    Template {
        name: "no-setInterval-if.js",
        aliases: &["nosiif.js", "ubo-no-setInterval-if.js", "ubo-nosiif.js"],
        body: r#"function gfNoSetIntervalIf(needle, delay) {
  const orig = window.setInterval;
  const matchDelay = delay !== undefined && delay !== "" ? Number(delay) : null;
  window.setInterval = function(fn, ms, ...rest) {
    const text = typeof fn === "function" ? fn.toString() : String(fn);
    const hitNeedle = !needle || text.includes(needle);
    const hitDelay = matchDelay === null || Number(ms) === matchDelay;
    if (hitNeedle && hitDelay) return 0;
    return orig.call(this, fn, ms, ...rest);
  };
}"#,
    },
    Template {
        name: "remove-attr.js",
        aliases: &["ra.js", "ubo-remove-attr.js", "ubo-ra.js"],
        body: r#"function gfRemoveAttr(attrs, selector) {
  const names = String(attrs || "").split("|").filter(Boolean);
  if (names.length === 0) return;
  const sel = selector || names.map(a => "[" + a + "]").join(",");
  const apply = () => {
    for (const el of document.querySelectorAll(sel)) {
      for (const a of names) el.removeAttribute(a);
    }
  };
  apply();
  document.addEventListener("DOMContentLoaded", apply, { once: true });
}"#,
    },
    Template {
        name: "remove-class.js",
        aliases: &["rc.js", "ubo-remove-class.js", "ubo-rc.js"],
        body: r#"function gfRemoveClass(classes, selector) {
  const names = String(classes || "").split("|").filter(Boolean);
  if (names.length === 0) return;
  const sel = selector || names.map(c => "." + CSS.escape(c)).join(",");
  const apply = () => {
    for (const el of document.querySelectorAll(sel)) {
      for (const c of names) el.classList.remove(c);
    }
  };
  apply();
  document.addEventListener("DOMContentLoaded", apply, { once: true });
}"#,
    },
    Template {
        name: "set-attr.js",
        aliases: &["ubo-set-attr.js"],
        body: r#"function gfSetAttr(selector, attr, value) {
  if (!selector || !attr) return;
  const apply = () => {
    for (const el of document.querySelectorAll(selector)) {
      el.setAttribute(attr, value === undefined ? "" : String(value));
    }
  };
  apply();
  document.addEventListener("DOMContentLoaded", apply, { once: true });
}"#,
    },
    Template {
        name: "set-local-storage-item.js",
        aliases: &["ubo-set-local-storage-item.js"],
        body: r#"function gfSetLocalStorageItem(key, value) {
  if (!key) return;
  if (value === "$remove$") { window.localStorage.removeItem(key); }
  else { window.localStorage.setItem(key, value === undefined ? "" : String(value)); }
}"#,
    },
    Template {
        name: "set-session-storage-item.js",
        aliases: &["ubo-set-session-storage-item.js"],
        body: r#"function gfSetSessionStorageItem(key, value) {
  if (!key) return;
  if (value === "$remove$") { window.sessionStorage.removeItem(key); }
  else { window.sessionStorage.setItem(key, value === undefined ? "" : String(value)); }
}"#,
    },
    Template {
        name: "set-cookie.js",
        aliases: &["ubo-set-cookie.js"],
        body: r#"function gfSetCookie(name, value, path) {
  if (!name) return;
  const p = path ? "; path=" + path : "; path=/";
  document.cookie = name + "=" + (value === undefined ? "" : value) + p;
}"#,
    },
    Template {
        name: "remove-cookie.js",
        aliases: &["cookie-remover.js", "ubo-remove-cookie.js"],
        body: r#"function gfRemoveCookie(needle) {
  if (!needle) return;
  for (const part of document.cookie.split(";")) {
    const name = part.split("=")[0].trim();
    if (name && name.includes(needle)) {
      document.cookie = name + "=; max-age=0; path=/";
    }
  }
}"#,
    },
    Template {
        name: "noopfunc.js",
        aliases: &["ubo-noopfunc.js"],
        body: "function gfNoopFunc() {}",
    },
];

/// The MIT scriptlet templates as `adblock` [`Resource`] values.
pub(crate) fn resources() -> Vec<Resource> {
    TEMPLATES
        .iter()
        .map(|template| Resource {
            name: template.name.to_owned(),
            aliases: template
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
            kind: ResourceType::Template,
            content: BASE64.encode(template.body.as_bytes()),
            dependencies: Vec::new(),
            permission: PermissionMask::default(),
        })
        .collect()
}

/// Extracts each scriptlet invocation `adblock` emitted inside its top-level
/// `try {\n…\n} catch ( e ) { }` blocks, for Provenance recording.
pub(crate) fn invocations(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = script.lines();
    while let Some(line) = lines.next() {
        if line == "try {" {
            let mut body = Vec::new();
            for inner in lines.by_ref() {
                if inner == "} catch ( e ) { }" {
                    break;
                }
                let trimmed = inner.trim();
                if !trimmed.is_empty() {
                    body.push(trimmed);
                }
            }
            if !body.is_empty() {
                out.push(body.join(" "));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use adblock::Engine;
    use adblock::lists::{FilterSet, ParseOptions};

    use super::*;

    #[test]
    fn scriptlet_names_do_not_collide_with_each_other_or_stand_ins() {
        let mut seen = BTreeSet::new();
        for resource in crate::stand_ins::resources().into_iter().chain(resources()) {
            assert!(seen.insert(resource.name.clone()), "{}", resource.name);
            for alias in resource.aliases {
                assert!(seen.insert(alias.clone()), "{alias}");
            }
        }
    }

    #[test]
    fn engine_expands_set_constant_scriptlet_and_records_invocation() {
        let mut set = FilterSet::new(false);
        set.add_filter_list(
            "clean-up.test##+js(set, __gfMarker, patched)\n".to_owned(),
            ParseOptions::default(),
        );
        let mut engine = Engine::new_with_filter_set(set);
        engine.use_resources(resources());

        let script = engine
            .url_cosmetic_resources("https://clean-up.test/scriptlet")
            .injected_script;

        assert!(script.contains("function gfSetConstant("), "{script}");
        assert_eq!(
            invocations(&script),
            vec![r#"gfSetConstant("__gfMarker", "patched")"#.to_owned()]
        );
    }

    #[test]
    fn extra_lists_block_run_scriptlets_only_when_allowed_and_isolate_cookie_sections() {
        use std::sync::Arc;

        use crate::cosmetic::PageScriptlets;
        use crate::list::List;
        use crate::matching::{self, Choices, Request, ResourceKind, Verdict};
        use crate::never_block::NeverBlock;
        use crate::rules::{ExtraLists, FilterLists, ListTexts};

        let base = FilterLists::compile(&ListTexts {
            ads: "clean-up.test##+js(set, __builtIn, yes)\n",
            trackers: "",
            annoyances: "",
            unbreak: "",
        });
        let extra_text = concat!(
            "/js/extra-only.js\n",
            "clean-up.test##+js(set, __extraMarker, from_extra)\n",
            "! *** easylist:easylist_cookie/easylist_cookie_general_hide.txt ***\n",
            "/js/cookie-section-url.js\n",
            "clean-up.test##+js(set, __cookieScriptlet, leaked)\n",
        );
        let lists = base.with_extra(Arc::new(ExtraLists::compile(&[extra_text], "")));
        let choices = Choices {
            ads: true,
            extra: true,
            ..Choices::default()
        };
        let never_block = NeverBlock::new();

        // Extra list blocks its live rule, never its cookie-section network rule.
        assert_eq!(
            matching::judge(
                &Request {
                    url: "https://clean-up.test/js/extra-only.js",
                    source_url: "https://clean-up.test/extra",
                    kind: ResourceKind::Script,
                    method: "GET",
                },
                &never_block,
                Some(&lists),
                &choices,
            ),
            Verdict::Blocked(List::Extra)
        );
        assert_eq!(
            matching::judge(
                &Request {
                    url: "https://clean-up.test/js/cookie-section-url.js",
                    source_url: "https://clean-up.test/extra",
                    kind: ResourceKind::Script,
                    method: "GET",
                },
                &never_block,
                Some(&lists),
                &choices,
            ),
            Verdict::Allowed
        );

        // With allow_extra = false, only built-in scriptlets run.
        let untrusted = PageScriptlets::of(&lists, &choices, "https://clean-up.test/page", false);
        assert_eq!(
            untrusted.built_in.invocations,
            vec![r#"gfSetConstant("__builtIn", "yes")"#.to_owned()]
        );
        assert!(untrusted.extra.is_empty());

        // With allow_extra = true, extra live scriptlets run and cookie-section scriptlets never do.
        let trusted = PageScriptlets::of(&lists, &choices, "https://clean-up.test/page", true);
        assert_eq!(
            trusted.extra.invocations,
            vec![r#"gfSetConstant("__extraMarker", "from_extra")"#.to_owned()]
        );
        assert!(!trusted.extra.script.contains("__cookieScriptlet"));
    }
}
