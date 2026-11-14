// The Consent prompt runner (ADR 0048; spec #577's `consent`), Goodfetch's own
// code under the framework's licence. The engine evaluates it in the guard's
// isolated world of every frame at every new document, handed autoconsent's
// library and heuristics (MPL-2.0, unmodified), autoconsent's rules and
// Goodfetch's snippet table, and starts it once it added the world's binding:
//
// 1. autoconsent's vendor rules, driven through `GuardActions`, which sends
//    every click to the guard's trusted input and every `hide`, `prehide` and
//    `stylesheet` step to the engine's invisible style sheet;
// 2. else autoconsent's `reject` heuristics, called here in every frame, the
//    top one included: a prompt they find with a refusal button is refused;
// 3. a prompt without one goes to the Decision provider (none yet);
// 4. else the wall test: a prompt that leaves the content readable and asks
//    for no payment is hidden; any other in the top frame is a consent wall,
//    which the engine has accepted where a person approved its domain
//    (`consent.domains`) and otherwise reports as `consent_required`.
//
// With the engine only detecting (`consent.handling: detect`) the runner only
// looks: autoconsent detects without acting, nothing is clicked or hidden, and
// each prompt is reported left, but the wall test still runs.
//
// Its overrides live in this world alone, so no page sees them: the
// `HTMLElement.prototype.click` override sends the clicks coded vendors make
// to the guard, and the `autoconsent-*` elements autoconsent's code writes are
// held back from the page, their text forwarded to the style sheet. Detection
// starts again whenever a DOM mutation matches a rule's detection or pre-hide
// selectors, or text the heuristics recognise, and on every route change the
// engine reports; nothing polls.
/* eslint-disable unicorn/consistent-function-scoping -- the bundle holds the runner as one function, its helpers inside */
// eslint-disable-next-line no-unused-vars -- the engine's bundle calls it
function runner({ autoconsent, heuristics, rules, worlds }) {
  const world = globalThis;
  const KEY = Symbol.for("goodfetch.consent");
  if (world[KEY]) return;
  // The elements the engine's guard clicks, by handle: the world's registry
  // of guard targets, which no page sees.
  const elements = new Map();
  world[Symbol.for("goodfetch.guard.targets")] = elements;
  const AutoConsent = autoconsent.default;
  const top = window.top === window;
  // A rule that names a snippet the table drops, or none it classes, never
  // runs: dropped by the snippets it names, here as when the bundle is built.
  const named = (steps) =>
    (steps ?? []).flatMap((step) => [
      ...(step.eval === undefined ? [] : [step.eval]),
      ...named(step.if === undefined ? [] : [step.if]),
      ...named(step.then),
      ...named(step.else),
      ...named(step.any),
    ]);
  const runs = (rule) =>
    [6, 7, 8, 9].every((steps) =>
      named(rule[steps]).every((snippet) => ["isolated", "main"].includes(worlds[snippet])),
    );
  const kept = { v: rules.v, s: rules.s, r: rules.r.filter(runs) };
  // Whether the engine answers prompts, which `start` sets.
  let answering = true;
  const config = () => ({
    enabled: true,
    autoAction: answering ? "optOut" : null,
    disabledCmps: [],
    enablePrehide: answering,
    enableCosmeticRules: answering,
    enableGeneratedRules: true,
    detectRetries: 20,
    isMainWorld: false,
    prehideTimeout: 2000,
    // Its heuristic runs in the top frame only, and clicks an only "accept"
    // in `tier2`: the runner calls the `reject` classification itself.
    enableHeuristicDetection: false,
    heuristicMode: "off",
    enablePopupMutationObserver: true,
    visualTest: false,
    logs: {
      lifecycle: false,
      rulesteps: false,
      detectionsteps: false,
      evals: false,
      errors: false,
      messages: false,
      waits: false,
    },
  });

  // Reporting through the binding the engine added to this world, by name.
  let binding = null;
  const queued = [];
  const waiting = new Map();
  let requests = 0;
  const send = (message) => {
    const text = JSON.stringify(message);
    if (binding === null) queued.push(text);
    else binding(text);
  };
  const request = (message) =>
    new Promise((resolve) => {
      requests += 1;
      waiting.set(requests, resolve);
      send({ ...message, id: requests });
    });
  const report = (outcome) => send({ kind: "outcome", url: location.href, ...outcome });

  // The document's first detection pass and wall test, which the engine
  // waits on (ADR 0048): `detected` once the pass found a vendor's prompt, a
  // pre-hide rule's or a prompt the heuristics know, and `settled` once the
  // pass ended, so a navigation's call waits for the verdict and live hiding
  // starts only after it.
  let firstDetected = false;
  let firstSettled = false;
  const detected = () => {
    if (firstDetected || firstSettled) return;
    firstDetected = true;
    send({ kind: "detected" });
  };
  const settle = () => {
    if (firstSettled) return;
    firstSettled = true;
    send({ kind: "settled" });
  };

  // The invisible style sheet the engine keeps for this document: pre-hiding
  // while a prompt is being looked for, and every rule that hides one.
  const hiding = { prehide: "", pending: "", rules: new Map(), synced: "" };
  const sync = async () => {
    const text = [hiding.prehide, ...hiding.rules.values()].filter(Boolean).join("\n");
    if (text === hiding.synced) return;
    hiding.synced = text;
    await request({ kind: "sheet", text });
  };
  const hide = (key, rule) => {
    hiding.rules.set(key, rule);
    return sync();
  };
  const unhide = (key) => {
    hiding.rules.delete(key);
    return sync();
  };
  const hidingStyle = (method) =>
    `${method === "opacity" ? "opacity: 0" : "display: none"} !important; z-index: -1 !important; pointer-events: none !important;`;
  const matches = (selector) => {
    try {
      return selector !== "" && document.querySelector(selector) !== null;
    } catch {
      return false;
    }
  };
  // Pre-hiding reaches the sheet only once a selector matches, so a page with
  // no prompt is never waited on.
  const prehide = (selector) => {
    hiding.pending = selector;
    if (matches(selector)) {
      detected();
      hiding.pending = "";
      hiding.prehide = `${selector} { ${hidingStyle("opacity")} }`;
      void sync();
    }
  };
  const unprehide = () => {
    hiding.pending = "";
    hiding.prehide = "";
    return sync();
  };

  // Trusted clicks, one at a time, through the engine's guard. The pre-hide
  // is lifted first, since a transparent prompt that lets clicks through
  // cannot be clicked; a checkbox a styled toggle hides or covers is clicked
  // through its label.
  let clicking = Promise.resolve(true);
  let handles = 0;
  const labelOf = (element) =>
    element instanceof HTMLInputElement ? (element.labels?.[0] ?? element.closest("label")) : null;
  const clickNow = async (element) => {
    if (!(element instanceof Element) || !element.isConnected) return false;
    if (hiding.prehide !== "" || hiding.pending !== "") await unprehide();
    let target = element;
    for (let attempt = 0; attempt < 2 && target !== null; attempt += 1) {
      handles += 1;
      const handle = handles;
      elements.set(handle, target);
      const clicked = await request({ kind: "click", handle });
      elements.delete(handle);
      if (clicked === true) return true;
      const label = labelOf(target);
      target = label === target ? null : label;
    }
    return false;
  };
  const trusted = (element) => {
    const clicked = clicking.then(() => clickNow(element));
    clicking = clicked.catch(() => false);
    return clicked;
  };
  HTMLElement.prototype.click = function click() {
    void trusted(this);
  };

  // The `autoconsent-*` elements autoconsent's code writes are never put in
  // the page; the text written into one goes to the style sheet instead.
  const held = new Set();
  const ours = (node) => node instanceof Element && node.id.startsWith("autoconsent-");
  const replace = (prototype, name, wrap) => {
    prototype[name] = wrap(prototype[name]);
  };
  replace(
    Node.prototype,
    "appendChild",
    (original) =>
      function appendChild(node) {
        if (!ours(node)) return original.call(this, node);
        held.add(node);
        return node;
      },
  );
  replace(
    Node.prototype,
    "insertBefore",
    (original) =>
      function insertBefore(node, child) {
        if (!ours(node)) return original.call(this, node, child);
        held.add(node);
        return node;
      },
  );
  for (const name of ["append", "prepend"]) {
    replace(
      Element.prototype,
      name,
      (original) =>
        function inserting(...nodes) {
          for (const node of nodes.filter(ours)) held.add(node);
          return original.apply(
            this,
            nodes.filter((node) => !ours(node)),
          );
        },
    );
  }
  replace(
    Element.prototype,
    "remove",
    (original) =>
      function remove() {
        if (!held.has(this)) return original.call(this);
        held.delete(this);
        void unhide(this);
        return undefined;
      },
  );
  for (const [prototype, name] of [
    [HTMLElement.prototype, "innerText"],
    [Node.prototype, "textContent"],
  ]) {
    const property = Object.getOwnPropertyDescriptor(prototype, name);
    Object.defineProperty(prototype, name, {
      ...property,
      set(value) {
        property.set.call(this, value);
        if (held.has(this)) void hide(this, String(value));
      },
    });
  }

  // A pass: one autoconsent instance with its rules, then the heuristics. A
  // later pass makes an earlier one's actions stale.
  let passes = 0;
  let started = 0;
  let acting = false;
  let selectors = null;
  const answered = new WeakSet();
  const guarded = (Base, generation) =>
    class GuardActions extends Base {
      live() {
        return generation === passes;
      }
      async clickElement(element) {
        return element instanceof HTMLElement && this.live() && trusted(element);
      }
      async click(selector, all = false) {
        const found = this.elementSelector(selector);
        if (found.length === 0 || !this.live()) return false;
        let landed = false;
        for (const element of all ? found : found.slice(0, 1)) {
          landed = (await trusted(element)) || landed;
        }
        return landed;
      }
      hide(selector, method) {
        if (selector !== "")
          void hide(`hide ${selector}`, `${selector} { ${hidingStyle(method)} }`);
        return selector.length > 0;
      }
      stylesheet(rule, id) {
        if (rule !== "") void hide(`sheet ${id ?? rule}`, rule);
        return rule.length > 0;
      }
      prehide(selector) {
        prehide(selector);
        return true;
      }
      undoPrehide() {
        void unprehide();
      }
      setStyle(selector, css) {
        // A style that hides the prompt itself is hiding, not finishing a
        // refusal.
        if (
          typeof selector === "string" &&
          /display\s*:\s*none|visibility\s*:\s*hidden/i.test(css)
        ) {
          void hide(`style ${selector}`, `${selector} { ${css} }`);
          return this.elementExists(selector);
        }
        return super.setStyle(selector, css);
      }
    };

  const evaluate = async (message) => {
    const snippet = message.snippetId;
    const where = worlds[snippet];
    if (where === "isolated") {
      try {
        return Boolean(autoconsent.evalSnippets[snippet].call(world));
      } catch {
        return false;
      }
    }
    // The main world is reached only to answer a prompt: a runner that only
    // looks never goes there.
    if (where === "main" && answering)
      return (await request({ kind: "eval", snippet, code: message.code })) === true;
    return false;
  };

  const pass = () => {
    passes += 1;
    started = performance.now();
    const generation = passes;
    let found = false;
    let heuristicsRan = false;
    const fallBack = () => {
      if (heuristicsRan || generation !== passes) return;
      heuristicsRan = true;
      void heuristicPass(generation);
    };
    const fromAutoconsent = async (message) => {
      if (generation !== passes) return;
      switch (message.type) {
        case "eval": {
          const result = await evaluate(message);
          void instance.receiveMessageCallback({ type: "evalResp", id: message.id, result });
          break;
        }
        case "popupFound":
          found = true;
          detected();
          if (answering) acting = true;
          else {
            report({ action: "left", rule: message.cmp });
            settle();
          }
          break;
        case "optOutResult":
          acting = false;
          if (!message.result) {
            report({ action: "failed", rule: message.cmp });
            fallBack();
          }
          break;
        case "autoconsentDone":
          acting = false;
          report({ action: message.isCosmetic ? "hidden" : "refused", rule: message.cmp });
          settle();
          break;
        case "report":
          if (
            !found &&
            (message.state.findCmpAttempts >= 2 || message.state.lifecycle === "nothingDetected")
          ) {
            fallBack();
          }
          break;
        default:
          break;
      }
    };
    const instance = new AutoConsent(fromAutoconsent, null, null);
    instance.domActions = new (guarded(instance.domActions.constructor, generation))(instance);
    void instance.receiveMessageCallback({
      type: "initResp",
      config: config(),
      rules: { autoconsent: [], compact: kept },
    });
    // A vendor detected without its prompt showing falls back too.
    setTimeout(() => {
      if (!found) fallBack();
    }, 3000);
    selectors ??= detectionSelectors(instance);
  };

  // Every selector by which a rule detects or pre-hides its prompt, in
  // unions of 100, those the browser does not parse left out.
  const detectionSelectors = (instance) => {
    const steps = (list) =>
      (list ?? []).flatMap((step) =>
        [step.exists, step.visible].filter((s) => typeof s === "string"),
      );
    const all = new Set(
      instance.rules.flatMap((rule) => [
        ...(rule.prehideSelectors ?? []),
        ...steps(rule.rule?.detectCmp),
      ]),
    );
    const scratch = document.createElement("div");
    const valid = [...all].filter((selector) => {
      try {
        scratch.matches(selector);
        scratch.querySelector(selector);
        return true;
      } catch {
        return false;
      }
    });
    const unions = [];
    for (let start = 0; start < valid.length; start += 100)
      unions.push(valid.slice(start, start + 100).join(","));
    return unions;
  };

  // Text, buttons and light-DOM roots, through open shadow roots too.
  const textOf = (element) => {
    let text = element.innerText ?? "";
    for (const host of element.querySelectorAll("*")) {
      if (host.shadowRoot)
        for (const child of host.shadowRoot.children) text += `\n${textOf(child)}`;
    }
    return text;
  };
  const BUTTONS = 'button, input[type="button"], input[type="submit"], a, [role="button"]';
  const buttonsOf = (root) => {
    const found = [...root.querySelectorAll(BUTTONS)];
    for (const host of root.querySelectorAll("*"))
      if (host.shadowRoot) found.push(...buttonsOf(host.shadowRoot));
    return found;
  };
  const visible = (element) =>
    element.checkVisibility({ visibilityProperty: true }) &&
    element.getBoundingClientRect().width > 0 &&
    element.getBoundingClientRect().height > 0;
  const promptLike = (element) => {
    if (element.matches('dialog[open], [role="dialog"], [role="alertdialog"], [aria-modal="true"]'))
      return visible(element);
    const position = getComputedStyle(element).position;
    return (position === "fixed" || position === "sticky") && visible(element);
  };
  // The outermost prompt-like element of the document; a frame's whole body.
  const findPrompt = () => {
    if (!top) {
      const body = document.body;
      return body && visible(body) ? body : null;
    }
    const deadline = performance.now() + 100;
    const candidates = [];
    const visit = (root) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {
        acceptNode(node) {
          if (node.shadowRoot) visit(node.shadowRoot);
          if (answered.has(node) || !promptLike(node)) return NodeFilter.FILTER_SKIP;
          candidates.push(node);
          return NodeFilter.FILTER_REJECT;
        },
      });
      while (walker.nextNode() !== null && performance.now() < deadline);
    };
    if (document.body) visit(document.body);
    return (
      candidates.find((candidate) => {
        const text = textOf(candidate);
        return (
          text.trim() !== "" &&
          !heuristics.isExcludedPopup(text) &&
          heuristics.checkHeuristicPatterns(text).patterns.length > 0
        );
      }) ?? null
    );
  };
  // The light-DOM element that holds `element`, its shadow hosts climbed.
  const lightOf = (element) => {
    let node = element;
    while (node.getRootNode() instanceof ShadowRoot) node = node.getRootNode().host;
    return node;
  };
  const selectorOf = (element) => {
    const path = [];
    for (let node = element; node instanceof Element; node = node.parentElement) {
      if (node.id !== "" && document.querySelectorAll(`#${CSS.escape(node.id)}`).length === 1) {
        path.unshift(`#${CSS.escape(node.id)}`);
        break;
      }
      const index = [...(node.parentElement?.children ?? [])]
        .filter((sibling) => sibling.localName === node.localName)
        .indexOf(node);
      path.unshift(`${node.localName}:nth-of-type(${index + 1})`);
    }
    return path.join(" > ");
  };
  // A prompt leaves the content readable when it covers less than half the
  // viewport and neither it nor a full-viewport backdrop lies at its centre.
  const readable = (element) => {
    const area = (box) =>
      Math.max(0, Math.min(box.right, innerWidth) - Math.max(box.left, 0)) *
      Math.max(0, Math.min(box.bottom, innerHeight) - Math.max(box.top, 0));
    const viewport = innerWidth * innerHeight;
    if (area(element.getBoundingClientRect()) >= viewport / 2) return false;
    const centre = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
    if (centre === null) return true;
    if (element.contains(centre)) return false;
    return !(
      getComputedStyle(centre).position === "fixed" &&
      area(centre.getBoundingClientRect()) >= 0.9 * viewport
    );
  };
  // A prompt offers payment instead of a refusal when one of its buttons asks
  // for a subscription or a payment.
  const PAYING =
    /subscri|abonn|\bpay\b|\bpaid\b|bezahl|payer|pagar|pagare|zahlen|\bpur\b|[€$£]\s?\d|\d\s?[€$£]/i;
  const paying = (buttons) =>
    buttons.some((button) => button.regexClassification !== "accept" && PAYING.test(button.text));
  const gone = async (element) => {
    for (let waited = 0; waited < 2000; waited += 100) {
      if (!element.isConnected || !visible(element)) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  };

  const heuristicPass = async (generation) => {
    try {
      await heuristicLook(generation);
    } finally {
      settle();
    }
  };
  const heuristicLook = async (generation) => {
    const prompt = findPrompt();
    if (prompt === null || generation !== passes) return;
    detected();
    answered.add(prompt);
    acting = true;
    try {
      const text = textOf(prompt);
      if (
        heuristics.isExcludedPopup(text) ||
        heuristics.checkHeuristicPatterns(text).patterns.length === 0
      )
        return;
      const buttons = buttonsOf(prompt)
        .filter(visible)
        .map((element) => ({ element, text: element.innerText || element.value || "" }));
      heuristics.classifyButtons(buttons);
      const refusal = buttons.find((button) => button.regexClassification === "reject");
      if (!answering && refusal !== undefined) {
        report({ action: "left", rule: "heuristic" });
        return;
      }
      if (refusal !== undefined) {
        const landed = await trusted(refusal.element);
        if (landed) {
          report({ action: "refused", rule: "heuristic", verified: await gone(prompt) });
          return;
        }
        report({ action: "failed", rule: "heuristic" });
      }
      // No refusal, or one that did not go through: the Decision provider is
      // asked when the Run has one; then the wall test. A framed prompt's
      // cover is not measured from its frame, so it is left.
      const light = lightOf(prompt);
      if (!top) {
        report({ action: "left", rule: "heuristic" });
        return;
      }
      const pays = refusal === undefined && paying(buttons);
      if (!pays && readable(light)) {
        if (answering) {
          await hide(light, `${selectorOf(light)} { display: none !important; }`);
          report({ action: "hidden", rule: "heuristic" });
        } else {
          report({ action: "left", rule: "heuristic" });
        }
        return;
      }
      // A prompt whose refusal did not go through offers one: no wall.
      if (refusal !== undefined) {
        report({ action: "left", rule: "heuristic" });
        return;
      }
      // A consent wall: accepted only where the engine says a person approved
      // its domain, else `consent_required`.
      const reason = pays ? "pays" : "covers";
      if ((await request({ kind: "wall", reason })) === "accept") {
        const acceptance = buttons.find((button) => button.regexClassification === "accept");
        if (acceptance !== undefined && (await trusted(acceptance.element))) {
          report({ action: "accepted", rule: "heuristic", reason, verified: await gone(prompt) });
          return;
        }
      }
      report({ action: "wall", rule: "heuristic", reason });
    } finally {
      acting = false;
    }
  };

  // Detection starts again when the DOM gains what a rule or the heuristics
  // would detect, once no pass is acting.
  let added = new Set();
  let scheduled = 0;
  const recheck = () => {
    const nodes = [...added].filter((node) => node instanceof Element && node.isConnected);
    added = new Set();
    if (hiding.pending !== "") prehide(hiding.pending);
    if (acting || nodes.length === 0 || performance.now() - started < 1000) return;
    const ruleDetects = (node, union) => {
      try {
        return node.matches(union) || node.querySelector(union) !== null;
      } catch {
        return false;
      }
    };
    const found = nodes.some(
      (node) =>
        (selectors ?? []).some((union) => ruleDetects(node, union)) ||
        heuristics.checkHeuristicPatterns(textOf(node).slice(0, 10000)).patterns.length > 0,
    );
    if (found) pass();
  };
  const watch = () => {
    new MutationObserver((records) => {
      for (const record of records) for (const node of record.addedNodes) added.add(node);
      for (const record of records) if (record.type === "attributes") added.add(record.target);
      clearTimeout(scheduled);
      scheduled = setTimeout(recheck, 300);
    }).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style", "hidden", "open"],
    });
  };

  const begin = () => {
    pass();
    if (document.documentElement !== null) watch();
    else document.addEventListener("DOMContentLoaded", watch, { once: true });
  };
  let running = false;
  world[KEY] = {
    // Starts the runner, reporting through the binding `name`, answering
    // prompts unless `answers` is false.
    start(name, answers) {
      if (running) return "started";
      running = true;
      answering = answers !== false;
      binding = world[name];
      for (const text of queued.splice(0)) binding(text);
      begin();
      return "started";
    },
    // A route change of a single-page app: detection starts again.
    restart() {
      if (running && !acting) pass();
      return "restarted";
    },
    // Where the document's first pass stands, which a navigation's call asks
    // once its document loaded: `settled` once it ended; `detecting` while it
    // acts on a prompt, or when the page shows one a rule or the heuristics
    // would detect; `none` otherwise, and before the runner started.
    detecting() {
      if (!running) return "none";
      if (firstSettled) return "settled";
      if (firstDetected || acting) return "detecting";
      const vendor = (selectors ?? []).some((union) => matches(union));
      return vendor || (top && findPrompt() !== null) ? "detecting" : "none";
    },
    // The engine's answer to request `id`.
    answer(id, value) {
      const resolve = waiting.get(id);
      waiting.delete(id);
      resolve?.(value);
      return true;
    },
  };
}
