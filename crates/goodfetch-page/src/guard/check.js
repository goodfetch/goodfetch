// The guard's check and arm (ADR 0019), called in the guard's isolated world
// with `Runtime.callFunctionOn`: one round trip that observes the target once
// a frame until it is unique and actionable for the action, or until `slice`
// milliseconds pass, and answers what it saw last. A ready pointer target gets
// the interception listener armed on the window, which cancels every event of
// the gesture that reaches another element; a ready text target gets the
// focus, and for `fill` its text selected.
async function check({ selector, action, events, slice }) {
  const world = globalThis;
  // An arm a cancelled call left behind never outlives the next check.
  world.__goodfetchArm?.disarm();
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
  const FILLABLE = [
    "",
    "email",
    "number",
    "password",
    "search",
    "tel",
    "text",
    "url",
    "date",
    "datetime-local",
    "month",
    "time",
    "week",
  ];
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
    const text = clip((element.textContent ?? "").replace(/\s+/g, " ").trim(), 50);
    return `${opening}${text}</${element.localName}>`;
  };
  // CSS, inside open shadow roots too, as Playwright's CSS engine matches.
  const query = () => {
    const found = [];
    const visit = (root) => {
      found.push(...root.querySelectorAll(selector));
      for (const element of root.querySelectorAll("*"))
        if (element.shadowRoot) visit(element.shadowRoot);
    };
    visit(document);
    return found;
  };
  const disabled = (element) =>
    element.matches(":disabled") || element.closest('[aria-disabled="true"]') !== null;
  const editable = (element) => {
    if (element.isContentEditable) return true;
    if (element.localName === "textarea") return !element.readOnly;
    if (element.localName === "input") return FILLABLE.includes(element.type) && !element.readOnly;
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
  const observe = () => {
    let found;
    try {
      found = query();
    } catch (error) {
      return { state: "invalid", message: String(error?.message ?? error) };
    }
    if (found.length === 0) return waiting("attached", "no element matches");
    if (found.length > 1)
      return {
        ...waiting("unique", `${found.length} elements match`),
        matches: found.map(describe),
      };
    const element = found[0];
    let box = element.getBoundingClientRect();
    if (!(box.width > 0 && box.height > 0) || getComputedStyle(element).visibility !== "visible") {
      return waiting("visible", `${describe(element)} is not visible`);
    }
    if (disabled(element)) return waiting("enabled", `${describe(element)} is disabled`);
    if (action === "fill" && !editable(element))
      return waiting("editable", `${describe(element)} is not editable`);
    if (action !== "pointer") {
      element.focus();
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
      return { state: "ready" };
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
        if (world.__goodfetchArm === arm) delete world.__goodfetchArm;
      },
    };
    // The gesture follows at once; an arm whose call was dropped expires.
    const expiry = setTimeout(() => arm.disarm(), 10000);
    world.__goodfetchArm = arm;
    return { state: "ready", x, y };
  };
  for (;;) {
    const seen = observe();
    if (seen.state !== "waiting" || performance.now() >= end) return seen;
    // A frame or 20 ms, whichever comes first: a page in the background
    // gets no frames.
    await new Promise((resolve) => {
      requestAnimationFrame(resolve);
      setTimeout(resolve, 20);
    });
  }
}
