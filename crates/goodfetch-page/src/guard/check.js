// The guard's check and arm (ADR 0019), called in the guard's isolated world
// with `Runtime.callFunctionOn`: one round trip that observes the target once
// a frame until it is unique and actionable for the action, or until `slice`
// milliseconds pass, and answers what it saw last. A ready pointer target gets
// the interception listener armed on the window, which cancels every event of
// the gesture that reaches another element; a ready text target gets the
// focus, and for `fill` its text selected. A locator wait (ADR 0080) passes a
// state for `action` instead, `attached`, `detached`, `visible` or `hidden`,
// or `count` with its `min` and `max`, and the check only observes the
// matches until they are in it, answering their count for `count`. The target is a CSS selector's one
// match, or the element another engine script handed the guard by `handle` in
// the world's registry (the Consent prompt runner's clicks). CDP sends this one
// function, so its helpers live inside it.
/* eslint-disable unicorn/consistent-function-scoping -- CDP sends the function alone */
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
async function check({ selector, handle, action, events, slice, min, max }) {
  const world = globalThis;
  // The arm's key, in the guard's world's own registry, which no page sees.
  const ARM = Symbol.for("goodfetch.guard.arm");
  // The locator waits' states, whose checks arm nothing and leave a
  // gesture's arm in place.
  const WAITS = ["attached", "detached", "visible", "hidden", "count"];
  // An arm a cancelled call left behind never outlives the next check.
  if (!WAITS.includes(action)) world[ARM]?.disarm();
  const end = performance.now() + slice;
  const VOID = [
    "area",
    "base",
    "br",
    "col",
    "embed",
    "hr",
    "img",
    "input",
    "link",
    "meta",
    "source",
    "track",
    "wbr",
  ];
  const FILLABLE = ["", "email", "number", "password", "search", "tel", "text", "url"];
  // The inputs whose value a person picks rather than types: `fill` sets it.
  const PICKED = ["color", "date", "datetime-local", "month", "range", "time", "week"];
  const clip = (text, length) => (text.length > length ? `${text.slice(0, length - 1)}…` : text);
  const describe = (element) => {
    if (!element) return "nothing";
    const attributes = [...element.attributes]
      .filter((attribute) => attribute.name !== "style")
      .map((attribute) =>
        attribute.value
          ? ` ${attribute.name}="${clip(attribute.value, 50)}"`
          : ` ${attribute.name}`,
      )
      .join("");
    const opening = `<${element.localName}${attributes}>`;
    if (VOID.includes(element.localName)) return opening;
    // Cut before it is normalized: a large element's text is read once.
    const text = clip((element.textContent ?? "").slice(0, 200).replace(/\s+/g, " ").trim(), 50);
    return `${opening}${text}</${element.localName}>`;
  };
  // CSS, inside open shadow roots too, as Playwright's CSS engine matches.
  const query = () => {
    if (handle !== undefined) {
      const element = world[Symbol.for("goodfetch.guard.targets")]?.get(handle);
      return element?.isConnected ? [element] : [];
    }
    const found = [];
    const visit = (root) => {
      found.push(...root.querySelectorAll(selector));
      for (const element of root.querySelectorAll("*"))
        if (element.shadowRoot) visit(element.shadowRoot);
    };
    visit(document);
    return found;
  };
  const shown = (element) => {
    const box = element.getBoundingClientRect();
    return box.width > 0 && box.height > 0 && getComputedStyle(element).visibility === "visible";
  };
  const disabled = (element) =>
    element.matches(":disabled") || element.closest('[aria-disabled="true"]') !== null;
  const editable = (element) => {
    if (element.isContentEditable) return true;
    if (element.localName === "textarea") return !element.readOnly;
    if (element.localName === "input")
      return (
        (FILLABLE.includes(element.type) || PICKED.includes(element.type)) && !element.readOnly
      );
    return false;
  };
  const moving = (element) =>
    document.getAnimations().some((animation) => {
      const target = animation.effect?.target;
      return (
        animation.playState === "running" &&
        target instanceof Element &&
        (target === element || target.contains(element))
      );
    });
  let previous = null;
  const waiting = (step, observed) => {
    if (step !== "stable") previous = null;
    return { state: "waiting", step, observed };
  };
  // A locator wait's state, which acts on nothing.
  const reached = (found) => {
    const count = found.length;
    if (action === "count") {
      return (min === undefined || count >= min) && (max === undefined || count <= max)
        ? { state: "ready", count }
        : waiting("count", `${count} elements match`);
    }
    if (action === "detached") {
      return count === 0 ? { state: "ready" } : waiting("detached", `${count} elements match`);
    }
    if (count > 1) return { ...waiting("unique", `${count} elements match`), matches: found };
    const element = found[0];
    if (action === "hidden") {
      return element === undefined || !shown(element)
        ? { state: "ready" }
        : waiting("hidden", `${describe(element)} is visible`);
    }
    if (element === undefined) return waiting("attached", "no element matches");
    if (action === "visible" && !shown(element))
      return waiting("visible", `${describe(element)} is not visible`);
    return { state: "ready" };
  };
  const observe = () => {
    let found;
    try {
      found = query();
    } catch (error) {
      return { state: "invalid", message: String(error?.message ?? error) };
    }
    if (WAITS.includes(action)) return reached(found);
    if (found.length === 0) return waiting("attached", "no element matches");
    if (found.length > 1)
      return {
        ...waiting("unique", `${found.length} elements match`),
        matches: found,
      };
    const element = found[0];
    if (!shown(element)) return waiting("visible", `${describe(element)} is not visible`);
    let box = element.getBoundingClientRect();
    if (disabled(element)) return waiting("enabled", `${describe(element)} is disabled`);
    if (action === "fill" && !editable(element))
      return waiting("editable", `${describe(element)} is not editable`);
    if (action !== "pointer") {
      element.focus();
      if (action === "fill" && element.localName === "input" && PICKED.includes(element.type)) {
        world[Symbol.for("goodfetch.guard.fill")] = element;
        return { state: "focused", set_value: true };
      }
      if (action === "fill") {
        if (element.isContentEditable) {
          const range = document.createRange();
          range.selectNodeContents(element);
          const selection = getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
        } else {
          element.select();
        }
      }
      return { state: "focused" };
    }
    const centre = () => ({ x: box.left + box.width / 2, y: box.top + box.height / 2 });
    let { x, y } = centre();
    if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) {
      element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
      box = element.getBoundingClientRect();
      ({ x, y } = centre());
    }
    // Stability is waited for only while an animation or a transition runs.
    const at = [box.left, box.top, box.width, box.height].join();
    if (moving(element) && at !== previous) {
      previous = at;
      return waiting("stable", `${describe(element)} is moving`);
    }
    previous = at;
    const root = element.getRootNode();
    const hit = (typeof root.elementFromPoint === "function" ? root : document).elementFromPoint(
      x,
      y,
    );
    if (hit === null || !(hit === element || element.contains(hit))) {
      return waiting("receives_events", `${describe(hit)} would receive the event`);
    }
    const state = { intercepted: null, reached: false };
    const listener = (event) => {
      // The page's own synthetic events are not the gesture's: they go on
      // as the page sent them, as Playwright's listener lets them.
      if (!event.isTrusted) return;
      if (event.composedPath().includes(element)) {
        state.reached = true;
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      state.intercepted ??= `${describe(event.target)} would have received the ${event.type}`;
    };
    for (const type of events) window.addEventListener(type, listener, true);
    const arm = {
      state,
      disarm() {
        clearTimeout(expiry);
        for (const type of events) window.removeEventListener(type, listener, true);
        if (world[ARM] === arm) delete world[ARM];
      },
    };
    // The gesture follows at once; an arm whose call was dropped expires.
    const expiry = setTimeout(() => arm.disarm(), 10000);
    world[ARM] = arm;
    return { state: "armed", x, y };
  };
  for (;;) {
    const seen = observe();
    if (seen.state !== "waiting" || performance.now() >= end) {
      // The matches are described once, for the answer, never each frame.
      if (seen.matches) seen.matches = seen.matches.map(describe);
      return seen;
    }
    // A frame or 20 ms, whichever comes first: a page in the background
    // gets no frames.
    await new Promise((resolve) => {
      requestAnimationFrame(resolve);
      setTimeout(resolve, 20);
    });
  }
}
