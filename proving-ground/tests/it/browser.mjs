// The programs the Proving ground's browser tests run (spec #152, Seam 1
// for pages whose state only a browser reaches): each drives a stock
// Chromium through Playwright, never an engine crate, the way a tool that
// is right, or one that makes a known mistake, would, and prints the
// result it would submit to the oracle as one JSON line. The test that
// started it submits that result and asserts the oracle's answer.
//
// It reads one JSON request on stdin: `program`, `url`, the browser's extra
// launch `args`, if any, and the program's own options. Every host name of the Proving ground maps to the loopback
// address, as the `resolve` setting does (ADR 0059).
//
// It is a program the oracle judges, not a served test of a Goodfetch
// surface, so it opens its own browser and contexts rather than taking
// test support's Playwright fixtures (ADR 0068), as a competitor's adapter
// would.
import { chromium } from "playwright";

/** How long a program waits for the page before it reports a failure. */
const DEADLINE_MS = 10_000;

/** The quiet time a network must keep to count as idle. */
const IDLE_MS = 500;

/** Reads the whole of stdin. */
const readStdin = async () => {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
};

/** Waits until the page's script has no report to the site in flight. */
const reportsSent = (page) =>
  page.waitForFunction(() => document.documentElement.dataset.posting === "0", undefined, {
    timeout: DEADLINE_MS,
  });

/**
 * Waits until no request has been in flight for `IDLE_MS`, WebSockets and
 * server-sent events left out, as the `live` tasks define `networkidle`;
 * false when `deadline` milliseconds pass first. The definition is written
 * here, since Playwright's own `networkidle` does not promise to leave
 * server-sent events out.
 */
const networkIdle = async (page, started, deadlineMs) => {
  const inFlight = new Set();
  let quietSince = Date.now();
  page.on("request", (request) => {
    if (request.resourceType() !== "eventsource") {
      inFlight.add(request);
    }
  });
  const done = (request) => {
    if (inFlight.delete(request) && inFlight.size === 0) {
      quietSince = Date.now();
    }
  };
  page.on("requestfinished", done);
  page.on("requestfailed", done);
  const deadline = Date.now() + deadlineMs;
  await started();
  while (inFlight.size > 0 || Date.now() - quietSince < IDLE_MS) {
    if (Date.now() > deadline) {
      return false;
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 50);
    });
  }
  return true;
};

/** The board's Record, as the page shows it now. */
const board = (page) =>
  page.evaluate(() => {
    const [state, updates, reading] = ["state", "updates", "reading"].map(
      (id) => document.getElementById(id).textContent,
    );
    return { id: document.querySelector(".board").dataset.id, state, updates, reading };
  });

/** The bounding box's middle of the pixels of `rgb` on the canvas. */
const middleOf = (page, rgb) =>
  page.evaluate((colour) => {
    const canvas = document.getElementById("canvas");
    const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    let [left, top, right, bottom] = [canvas.width, canvas.height, -1, -1];
    for (let y = 0; y < canvas.height; y += 1) {
      for (let x = 0; x < canvas.width; x += 1) {
        const at = 4 * (y * canvas.width + x);
        if (
          pixels[at] === colour[0] &&
          pixels[at + 1] === colour[1] &&
          pixels[at + 2] === colour[2]
        ) {
          left = Math.min(left, x);
          top = Math.min(top, y);
          right = Math.max(right, x);
          bottom = Math.max(bottom, y);
        }
      }
    }
    return right < 0 ? null : { x: (left + right) / 2, y: (top + bottom) / 2 };
  }, rgb);

/** Clicks the canvas at `point`, in CSS pixels from its top-left corner. */
const clickCanvas = async (page, point) => {
  const box = await page.locator("canvas").boundingBox();
  await page.mouse.click(box.x + point.x, box.y + point.y);
  await reportsSent(page);
  return { status: "success" };
};

/** The two points the surface page names. */
const strokePoints = (page) =>
  page.evaluate(() => {
    const [start, end] = ["#stroke .start", "#stroke .end"].map((selector) => {
      const [x, y] = document.querySelector(selector).textContent.match(/\d+/g).map(Number);
      return { x, y };
    });
    return { start, end };
  });

/** The surface's colour at each point, as `#rrggbb`. */
const readBack = (page, points) =>
  page.evaluate(({ start, end }) => {
    const context = document.getElementById("surface").getContext("2d");
    const colour = ({ x, y }) =>
      `#${Array.from(context.getImageData(x, y, 1, 1).data.slice(0, 3))
        .map((value) => value.toString(16).padStart(2, "0"))
        .join("")}`;
    return { id: "stroke", start: colour(start), end: colour(end) };
  }, points);

/** The report's entries the page shows under its current media. */
const shownEntries = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll(".entry")]
      .filter((entry) => entry.checkVisibility())
      .map((entry) => ({
        id: entry.dataset.id,
        title: entry.querySelector(".title").textContent,
      })),
  );

/** The layout's Record, as the page shows it. */
const layout = (page) =>
  page.evaluate(() => {
    const [plan, density] = [".plan", ".density"].map((selector) =>
      [...document.querySelectorAll(selector)].find((element) => element.checkVisibility()),
    );
    return {
      id: "layout",
      breakpoint: plan.dataset.breakpoint,
      plan: plan.querySelector(".value").textContent,
      density: density.querySelector(".value").textContent,
    };
  });

/** The text the key recorder asks for. */
const askedText = (page) => page.locator("#ask .text").textContent();

/** The physical key a French AZERTY layout types each character with. */
const AZERTY = { a: "KeyQ", q: "KeyA", z: "KeyW", w: "KeyZ", m: "Semicolon", " ": "Space" };

const programs = {
  /** Reads the page's links once its script ran: each `<a>`'s URL. */
  links: async (page, { url }) => {
    await page.goto(url);
    return {
      links: await page.evaluate(() =>
        [...document.querySelectorAll("a")].map((link) => link.href),
      ),
    };
  },
  /** Waits for the network to be idle, then reads the board. */
  "board-when-idle": async (page, { url, deadline_ms: deadlineMs = DEADLINE_MS }) => {
    const idle = await networkIdle(page, () => page.goto(url, { waitUntil: "load" }), deadlineMs);
    if (!idle) {
      return { status: "failure" };
    }
    return { status: "success", records: [await board(page)] };
  },
  /** Reads the board as soon as its document has loaded: a stale idle. */
  "board-at-load": async (page, { url }) => {
    await page.goto(url, { waitUntil: "domcontentloaded" });
    return { status: "success", records: [await board(page)] };
  },
  /** Clicks the middle of the button the canvas asks for, found by its pixels. */
  "canvas-button": async (page, { url }) => {
    await page.goto(url);
    return clickCanvas(page, await middleOf(page, [0x1d, 0x4e, 0xd8]));
  },
  /** Clicks the decoy instead. */
  "canvas-decoy": async (page, { url }) => {
    await page.goto(url);
    return clickCanvas(page, await middleOf(page, [0x4b, 0x55, 0x63]));
  },
  /** Clicks the canvas's corner, on neither button: a missed click. */
  "canvas-corner": async (page, { url }) => {
    await page.goto(url);
    return clickCanvas(page, { x: 4, y: 4 });
  },
  /** Draws the stroke the page names, then reads the surface back. */
  "surface-stroke": async (page, { url }) => {
    await page.goto(url);
    const points = await strokePoints(page);
    const box = await page.locator("canvas").boundingBox();
    await page.mouse.move(box.x + points.start.x, box.y + points.start.y);
    await page.mouse.down();
    await page.mouse.move(box.x + points.end.x, box.y + points.end.y, { steps: 12 });
    await page.mouse.up();
    await reportsSent(page);
    return { status: "success", records: [await readBack(page, points)] };
  },
  /** Reads the surface back without drawing on it. */
  "surface-unread": async (page, { url }) => {
    await page.goto(url);
    return { status: "success", records: [await readBack(page, await strokePoints(page))] };
  },
  /** Reads the entries the page shows under `media`. */
  entries: async (page, { url, media }) => {
    await page.goto(url);
    await page.emulateMedia({ media });
    return { status: "success", records: await shownEntries(page) };
  },
  /** Saves the page as MHTML and lists the paths of its parts. */
  mhtml: async (page, { url }) => {
    await page.goto(url);
    const session = await page.context().newCDPSession(page);
    const { data } = await session.send("Page.captureSnapshot", { format: "mhtml" });
    const paths = [...data.matchAll(/^Content-Location: (\S+)/gmu)].map(
      ([, location]) => new URL(location).pathname,
    );
    return { status: "success", records: paths.map((id) => ({ id })) };
  },
  /**
   * Reads the layout the page shows in the viewport the context has, then
   * leaves the page. Closing the browser ends the context's emulated device
   * pixel ratio while a page it shows still runs, and that page picks its
   * image again at the screen's own ratio of 1 and fetches it, which the
   * oracle rightly counts against any other ratio. On `about:blank` the
   * layout's page has been unloaded (Playwright turns the back-forward cache
   * off), so nothing is left to fetch.
   */
  layout: async (page, { url }) => {
    await page.goto(url);
    const record = await layout(page);
    await page.goto("about:blank");
    return { status: "success", records: [record] };
  },
  /** Types the asked text key by key, as a US QWERTY keyboard sends it. */
  "type-keys": async (page, { url, swap }) => {
    await page.goto(url);
    let text = await askedText(page);
    if (swap) {
      text = text[1] + text[0] + text.slice(2);
    }
    await page.locator("#field").click();
    await page.keyboard.type(text);
    await reportsSent(page);
    return { status: "success" };
  },
  /** Fills the field with the asked text in one go, with no key press. */
  fill: async (page, { url }) => {
    await page.goto(url);
    await page.locator("#field").fill(await askedText(page));
    await reportsSent(page);
    return { status: "success" };
  },
  /** Types the asked text key by key, as a French AZERTY keyboard sends it. */
  "type-azerty": async (page, { url }) => {
    await page.goto(url);
    const text = await askedText(page);
    await page.locator("#field").click();
    const session = await page.context().newCDPSession(page);
    for (const character of text) {
      const code = AZERTY[character] ?? `Key${character.toUpperCase()}`;
      const key = { key: character, code };
      await session.send("Input.dispatchKeyEvent", { type: "keyDown", ...key, text: character });
      await session.send("Input.dispatchKeyEvent", { type: "keyUp", ...key });
    }
    await reportsSent(page);
    return { status: "success" };
  },
  /**
   * Waits until every context of the consistency probes has reported, then
   * reads the values the main page shows for each.
   */
  probe: async (page, { url }) => {
    await page.goto(url);
    await page.waitForFunction(() => document.documentElement.dataset.probe === "done", undefined, {
      timeout: DEADLINE_MS,
    });
    return {
      contexts: JSON.parse(await page.locator("#contexts").textContent()),
    };
  },
  /** Composes the asked text through an input method, then commits it. */
  compose: async (page, { url }) => {
    await page.goto(url);
    const text = await askedText(page);
    await page.locator("#field").click();
    const session = await page.context().newCDPSession(page);
    const characters = [...text];
    for (let count = 1; count <= characters.length; count += 1) {
      const composing = characters.slice(0, count).join("");
      await session.send("Input.imeSetComposition", {
        text: composing,
        selectionStart: composing.length,
        selectionEnd: composing.length,
      });
    }
    await session.send("Input.insertText", { text });
    await reportsSent(page);
    return { status: "success" };
  },
};

const request = await readStdin();
const program = programs[request.program];
if (program === undefined) {
  throw new Error(`no program ${request.program}`);
}
const browser = await chromium.launch({
  args: ["--host-resolver-rules=MAP *.test 127.0.0.1", ...(request.args ?? [])],
});
try {
  const context = await browser.newContext(request.context ?? {});
  const page = await context.newPage();
  const result = await program(page, request);
  process.stdout.write(`${JSON.stringify(result)}\n`);
} finally {
  await browser.close();
}
