// The identity a page reads of its browser: its user agent and every
// client hint `navigator.userAgentData` gives, the high-entropy ones
// included. It reports that it ran, then each value under its own name;
// the requests it makes carry the client hints the page's `Accept-CH` asks
// for, which the server observes beside them. A headless browser that
// shows its window's identity reports the same values as with a window.

// A report holds no empty value.
const shown = (value) =>
  value === undefined || value === null || value === "" ? "none" : String(value);
const brands = (list) => (list ?? []).map((brand) => `${brand.brand}/${brand.version}`).join(", ");

(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const page = window.location.pathname;

  const send = (reported, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check: reported, observed }),
      keepalive: true,
    }).catch(() => {});

  const run = async () => {
    const data = navigator.userAgentData;
    let high = {};
    try {
      high = data
        ? await data.getHighEntropyValues([
            "architecture",
            "bitness",
            "formFactors",
            "fullVersionList",
            "model",
            "platformVersion",
            "wow64",
          ])
        : {};
    } catch {
      // No high-entropy values to report.
    }
    const values = [
      ["user_agent", navigator.userAgent],
      ["brands", brands(data?.brands)],
      ["full_version_list", brands(high.fullVersionList)],
      ["platform", data?.platform],
      ["platform_version", high.platformVersion],
      ["architecture", high.architecture],
      ["bitness", high.bitness],
      ["mobile", data?.mobile],
      ["model", high.model],
      ["wow64", high.wow64],
      ["form_factors", (high.formFactors ?? []).join(", ")],
    ];
    await send("ran", "content");
    await Promise.all(values.map(([name, value]) => send(name, shown(value))));
  };
  void run();
})();
