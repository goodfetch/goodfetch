// The self-hosted bot probe: the signals open-source detectors such as BotD
// and CreepJS read, checked in the page, written here rather than copied.
// It reports that it ran, then each signal it found; a browser that passes
// for a person's reports none.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const check = script.getAttribute("data-check");
  const page = window.location.pathname;
  const signals = [];

  const send = (reported, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check: reported, observed }),
      keepalive: true,
    }).catch(() => {});
  const test = (signal, found) => {
    try {
      if (found()) {
        signals.push(signal);
      }
    } catch {
      signals.push(`${signal}_threw`);
    }
  };
  const userAgent = navigator.userAgent;
  const claimsChrome = /Chrome\//.test(userAgent);

  test("webdriver", () => navigator.webdriver);
  test("headless_user_agent", () => /HeadlessChrome/.test(userAgent));
  test("user_agent_data_mismatch", () => {
    if (!claimsChrome || !navigator.userAgentData) {
      return false;
    }
    const major = (/Chrome\/(\d+)/.exec(userAgent) ?? [])[1];
    return !navigator.userAgentData.brands.some((brand) => brand.version === major);
  });
  test("no_plugins", () => claimsChrome && Object.keys(navigator.plugins).length === 0);
  test("no_languages", () => !navigator.languages || navigator.languages.length === 0);
  test("missing_window_chrome", () => claimsChrome && !("chrome" in window));
  test("zero_outer_size", () => window.outerWidth === 0 || window.outerHeight === 0);
  test("automation_globals", () => {
    const names = [
      "callPhantom",
      "_phantom",
      "__nightmare",
      "domAutomation",
      "domAutomationController",
      "_selenium",
      "__webdriver_evaluate",
      "__selenium_unwrapped",
      "__fxdriver_unwrapped",
    ];
    return (
      names.some((name) => name in window) ||
      Object.keys(document).some((key) => /^\$cdc_|^\$wdc_/.test(key))
    );
  });
  test("patched_navigator", () => {
    const names = ["webdriver", "userAgent", "languages", "plugins", "hardwareConcurrency"];
    return names.some((name) => {
      // A getter a script replaced is no longer the browser's own code.
      const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, name)?.["get"];
      return (
        Object.getOwnPropertyDescriptor(navigator, name) !== undefined ||
        (typeof getter === "function" &&
          !/\{\s*\[native code\]\s*\}\s*$/.test(Function.prototype.toString.call(getter)))
      );
    });
  });
  test("software_renderer", () => {
    const gl = document.createElement("canvas").getContext("webgl");
    if (!gl) {
      return false;
    }
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
    return /SwiftShader|llvmpipe|Software/i.test(String(renderer));
  });

  const run = async () => {
    if (navigator.permissions && "Notification" in window) {
      try {
        const status = await navigator.permissions.query({ name: "notifications" });
        if (Notification.permission === "denied" && status.state === "prompt") {
          signals.push("notification_permission_mismatch");
        }
      } catch {
        // No permission to compare.
      }
    }
    await send("ran", "content");
    await Promise.all(signals.map((signal) => send(check, signal)));
  };
  void run();
})();
