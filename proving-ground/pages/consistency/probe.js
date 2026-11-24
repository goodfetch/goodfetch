// The consistency probes' reader (ADR 0074), the same classic script in a
// window and in a worker: `consistencyProbe(page)` reads each surface of the
// context it runs in, posts one report per surface to its own origin's
// `/reports?page=<page>`, and resolves to the values it read. A value is
// reported as JSON text: `null` when the context does not have the
// surface, `{"error": …}` when reading it threw.
"use strict";

/** The high-entropy user-agent client hints a site may ask for. */
const HINTS = ["architecture", "bitness", "fullVersionList", "model", "platformVersion", "wow64"];

/** A brand list as `brand/version` strings. */
const brands = (list) => list.map(({ brand, version }) => `${brand}/${version}`);

/** The date options this context resolves. */
const dates = () => Intl.DateTimeFormat().resolvedOptions();

/** The high-entropy hints, once asked for. */
let hints;
const hint = async (name) => {
  const data = navigator.userAgentData;
  if (data === undefined) return undefined;
  hints ??= data.getHighEntropyValues(HINTS);
  return (await hints)[name];
};

/** The WebGL parameter `name` of the debug renderer info, if any. */
const webgl = (name) => {
  if (typeof OffscreenCanvas === "undefined") return undefined;
  const context = new OffscreenCanvas(1, 1).getContext("webgl");
  if (context === null) return undefined;
  const info = context.getExtension("WEBGL_debug_renderer_info");
  return info === null ? undefined : context.getParameter(info[name]);
};

/** Each surface's reader; a window's screen is read in windows only. */
const SURFACES = {
  user_agent: () => navigator.userAgent,
  app_version: () => navigator.appVersion,
  platform: () => navigator.platform,
  language: () => navigator.language,
  languages: () => navigator.languages,
  hardware_concurrency: () => navigator.hardwareConcurrency,
  device_memory: () => navigator.deviceMemory,
  webdriver: () => navigator.webdriver,
  time_zone: () => dates().timeZone,
  locale: () => dates().locale,
  timezone_offset: () => new Date(Date.UTC(2026, 0, 1)).getTimezoneOffset(),
  ua_brands: () => navigator.userAgentData && brands(navigator.userAgentData.brands),
  ua_mobile: () => navigator.userAgentData?.mobile,
  ua_platform: () => navigator.userAgentData?.platform,
  ua_platform_version: () => hint("platformVersion"),
  ua_architecture: () => hint("architecture"),
  ua_bitness: () => hint("bitness"),
  ua_model: () => hint("model"),
  ua_wow64: () => hint("wow64"),
  ua_full_version_list: async () => {
    const list = await hint("fullVersionList");
    return list && brands(list);
  },
  webgl_vendor: () => webgl("UNMASKED_VENDOR_WEBGL"),
  webgl_renderer: () => webgl("UNMASKED_RENDERER_WEBGL"),
  screen_width: () => self.screen?.width,
  screen_height: () => self.screen?.height,
  screen_avail_width: () => self.screen?.availWidth,
  screen_avail_height: () => self.screen?.availHeight,
  screen_color_depth: () => self.screen?.colorDepth,
  device_pixel_ratio: () => self.devicePixelRatio,
};

/** Posts one surface's value to the site. */
const report = (page, check, observed) =>
  fetch(`/reports?page=${page}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ page, check, observed }),
  }).catch(() => {});

self.consistencyProbe = async (page) => {
  const values = {};
  for (const [surface, read] of Object.entries(SURFACES)) {
    let value;
    try {
      value = await read();
    } catch (error) {
      value = { error: String(error) };
    }
    values[surface] = JSON.parse(JSON.stringify(value ?? null));
  }
  await Promise.all(
    Object.entries(values).map(([check, value]) => report(page, check, JSON.stringify(value))),
  );
  return values;
};
