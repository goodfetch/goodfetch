// The origin-escape page's script (ADR 0031): shown inside the Console's
// sandboxed frame on its content origin, it tries to reach past that frame,
// and reports to the Proving ground each attempt that worked. A page shown
// on its own, as the top document, has nothing to escape and reports nothing.
(function () {
  "use strict";
  var script = document.currentScript;
  var reports = script.getAttribute("data-reports");
  var landing = script.getAttribute("data-landing");
  if (window.top === window) {
    return;
  }
  function report(attempt) {
    var body = JSON.stringify({ page: "/escape", check: "escape", observed: attempt });
    try {
      navigator.sendBeacon(reports, body);
    } catch (error) {
      fetch(reports, { method: "POST", body: body, mode: "no-cors", keepalive: true });
    }
  }
  function attempt(name, reach) {
    try {
      if (reach()) {
        report(name);
      }
    } catch (error) {
      // Refused, as it must be.
    }
  }
  attempt("top_location", function () {
    return typeof window.top.location.href === "string";
  });
  attempt("parent_document", function () {
    return typeof window.parent.document.title === "string";
  });
  attempt("cookie_access", function () {
    document.cookie = "escape=1; path=/";
    return document.cookie.indexOf("escape=1") !== -1;
  });
  attempt("storage_access", function () {
    window.localStorage.setItem("escape", "1");
    return window.localStorage.getItem("escape") === "1";
  });
  attempt("popup", function () {
    // A popup that opens loads the landing page, which records it.
    window.open(landing + "?attempt=popup", "_blank");
    return false;
  });
  attempt("top_navigation", function () {
    // Last: a navigation that works leaves this page, and the landing page
    // records it.
    window.top.location.href = landing + "?attempt=top_navigation";
    return false;
  });
})();
