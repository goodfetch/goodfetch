// The origin-escape page's script: shown inside a sandboxed frame on an
// origin of its own, it tries to reach past that frame,
// and reports to the Proving ground each attempt that worked. A page shown
// on its own, as the top document, has nothing to escape and reports nothing.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const landing = script.getAttribute("data-landing");
  if (window.top === window) {
    return;
  }
  const report = (attempt) => {
    const body = JSON.stringify({ page: "/escape", check: "escape", observed: attempt });
    if (!navigator.sendBeacon(reports, body)) {
      fetch(reports, { method: "POST", body, mode: "no-cors", keepalive: true }).catch(() => {});
    }
  };
  const attempt = (name, reach) => {
    try {
      if (reach()) {
        report(name);
      }
    } catch {
      // Refused, as it must be.
    }
  };
  attempt("top_location", () => typeof window.top.location.href === "string");
  attempt("parent_document", () => typeof window.parent.document.title === "string");
  attempt("cookie_access", () => {
    document.cookie = "escape=1; path=/";
    return document.cookie.includes("escape=1");
  });
  attempt("storage_access", () => {
    window.localStorage.setItem("escape", "1");
    return window.localStorage.getItem("escape") === "1";
  });
  attempt("popup", () => {
    // A popup that opens loads the landing page, which records it.
    window.open(`${landing}?attempt=popup`, "_blank");
    return false;
  });
  attempt("top_navigation", () => {
    // Last: a navigation that works leaves this page, and the landing page
    // records it.
    window.top.location.href = `${landing}?attempt=top_navigation`;
    return false;
  });
})();
