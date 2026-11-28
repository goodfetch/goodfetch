// What the Page check reads of the main frame's document (ADRs 0006 and
// 0092), called in the guard's isolated world, which no page script
// reaches: whether each of `matches`, a CSS selector, an XPath expression
// or a text, finds anything, `null` for a selector or an expression the
// document refuses; and whether the document shows one password field that
// takes no new password, the built-in detector's sign of a login form; and
// the `content` of each robots `meta` element, which says whether the
// document is marked `noindex` (ADR 0035). A
// text matches the body's text with runs of white space read as one space.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function pageCheck(argument) {
  const body = document.body === null ? "" : document.body.innerText.replace(/\s+/g, " ");
  const found = (match) => {
    try {
      switch (match.kind) {
        case "css":
          return document.querySelector(match.selector) !== null;
        case "xpath":
          return (
            document.evaluate(
              match.selector,
              document,
              null,
              XPathResult.FIRST_ORDERED_NODE_TYPE,
              null,
            ).singleNodeValue !== null
          );
        case "text":
          return body.includes(match.text.replace(/\s+/g, " ").trim());
        default:
          return null;
      }
    } catch {
      return null;
    }
  };
  const passwords = [...document.querySelectorAll("input[type=password]")];
  const password = passwords.length === 1 ? passwords[0] : null;
  const box = password === null ? null : password.getBoundingClientRect();

  // Composed Wall detection (ADR 0052; ticket #757) inside recovery's one read.
  const wall = (() => {
    const wallRules = argument.wall_rules;
    if (!wallRules) return null;
    let kind = null;
    let overlaySelector = null;
    for (const [candidateKind, selectors] of [
      ["paywall", wallRules.paywall_selectors || []],
      ["ad_blocker_wall", wallRules.ad_blocker_wall_selectors || []],
    ]) {
      for (const sel of selectors) {
        try {
          const el = document.querySelector(sel);
          if (!el) continue;
          const style = getComputedStyle(el);
          if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0") {
            continue;
          }
          const rect = el.getBoundingClientRect();
          if (rect.width > 0 && rect.height > 0) {
            kind = candidateKind;
            overlaySelector = sel;
            break;
          }
        } catch {
          // ignore invalid selector
        }
      }
      if (kind !== null) break;
    }
    let paidSectionEmpty = false;
    const paidSelectors = [];
    for (const script of document.querySelectorAll('script[type="application/ld+json"]')) {
      try {
        const data = JSON.parse(script.textContent || "");
        const stack = Array.isArray(data) ? [...data] : [data];
        while (stack.length > 0) {
          const node = stack.pop();
          if (!node || typeof node !== "object") continue;
          if (node.isAccessibleForFree === false || node.isAccessibleForFree === "False") {
            if (typeof node.cssSelector === "string" && node.cssSelector.length > 0) {
              paidSelectors.push(node.cssSelector);
            }
            if (kind === null) {
              kind = "paywall";
            }
          }
          if (Array.isArray(node.hasPart)) {
            for (const part of node.hasPart) stack.push(part);
          } else if (node.hasPart && typeof node.hasPart === "object") {
            stack.push(node.hasPart);
          }
          if (Array.isArray(node["@graph"])) {
            for (const item of node["@graph"]) stack.push(item);
          }
        }
      } catch {
        // ignore malformed JSON-LD
      }
    }
    for (const sel of paidSelectors) {
      try {
        const el = document.querySelector(sel);
        if (!el || (el.textContent || "").trim().length === 0) {
          paidSectionEmpty = true;
          break;
        }
      } catch {
        // ignore invalid selector
      }
    }
    if (kind === null) return null;
    const mainEl = document.querySelector("main, article") || document.body;
    const mainText = mainEl ? mainEl.innerText || mainEl.textContent || "" : "";
    const trimmed = mainText.trim();
    const mainWords = trimmed.length === 0 ? 0 : trimmed.split(/\s+/).length;
    return {
      kind,
      overlay_selector: overlaySelector,
      paid_section_empty: paidSectionEmpty,
      main_words: mainWords,
    };
  })();

  return {
    found: argument.matches.map(found),
    password:
      password !== null &&
      box.width > 0 &&
      box.height > 0 &&
      getComputedStyle(password).visibility === "visible" &&
      password.autocomplete !== "new-password",
    robots: [...document.querySelectorAll("meta[name]")]
      .filter((meta) => meta.name.toLowerCase() === "robots")
      .map((meta) => meta.content),
    wall,
  };
}
