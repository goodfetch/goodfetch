// The consent pages' own script (ADR 0048; spec #577's Clean-up conformance
// cases), loaded first by every page on `consent.test` and
// `consent-frame.test`. Each thing it sees is a request the Proving ground
// records (`GET /observations`), named by the page's `data-vendor`:
//
// - `/consent/answered?answer=…&trusted=…`: a button with `data-answer` was
//   clicked, and whether the click was trusted; the elements its
//   `data-closes` names close, as a vendor's prompt does once answered, and a
//   framed prompt tells its parent;
// - `/consent/hooked`: the page's own `HTMLElement.prototype.click` hook was
//   called, as a site that watches its buttons would see a script's click;
// - `/consent/leftover?id=…`: an element whose ID or class starts with
//   `autoconsent` was put in the page.
(() => {
  "use strict";
  const vendor = document.currentScript.dataset.vendor;
  const beacon = (path, params) =>
    fetch(`${path}?${new URLSearchParams({ vendor, ...params })}`, { keepalive: true }).catch(
      () => {},
    );
  window.consentBeacon = beacon;

  // eslint-disable-next-line typescript/unbound-method -- called with `this` below
  const nativeClick = HTMLElement.prototype.click;
  HTMLElement.prototype.click = function click() {
    void beacon("/consent/hooked", {});
    return nativeClick.call(this);
  };

  const leftovers = (node) => {
    if (!(node instanceof Element)) return;
    for (const element of [node, ...node.querySelectorAll("*")]) {
      const named = [element.id, ...element.classList].find((name) =>
        name.startsWith("autoconsent"),
      );
      if (named !== undefined) void beacon("/consent/leftover", { id: named });
    }
  };
  new MutationObserver((records) => {
    for (const record of records) for (const node of record.addedNodes) leftovers(node);
  }).observe(document.documentElement, { childList: true, subtree: true });

  document.addEventListener(
    "click",
    (event) => {
      const button = event
        .composedPath()
        .find((node) => node instanceof Element && node.hasAttribute("data-answer"));
      if (button === undefined) return;
      void beacon("/consent/answered", {
        answer: button.dataset.answer,
        trusted: String(event.isTrusted),
      });
      const closes = button.dataset.closes;
      if (closes) for (const element of document.querySelectorAll(closes)) element.remove();
      if (window.parent !== window) window.parent.postMessage({ consent: "closed" }, "*");
    },
    true,
  );
})();
