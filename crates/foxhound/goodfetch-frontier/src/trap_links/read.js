// What a person sees of each element `selector` matches (ADR 0035; the
// Frontier's `trap_links`), called in the guard's isolated world of Collie's
// page: the document's URL, base URL and encoding, and for each element, in
// document order, its `href`, text and attributes, and the facts `judge`
// decides from. `cosmetic` holds the selectors of the cosmetic rules live
// hiding applies to the page (ADR 0047): an element one of them matches, or
// one holding it, is reported `cosmetic`. `opening` holds the elements and
// roles whose content a person opens to see, as `trap_links::opens` reads
// them. A selector the page refuses throws.
/* eslint-disable unicorn/consistent-function-scoping -- CDP sends the function alone */
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function trapLinks({ selector, cosmetic, opening }) {
  // The element holding `element`, through the shadow root it sits in.
  const holder = (element) => {
    if (element.parentElement) return element.parentElement;
    const root = element.getRootNode();
    return root instanceof ShadowRoot ? root.host : null;
  };
  const holders = (element) => {
    const all = [];
    for (let at = element; at; at = holder(at)) all.push(at);
    return all;
  };
  const matchesAny = (element, selectors) =>
    selectors.some((rule) => {
      try {
        return element.matches(rule);
      } catch {
        return false;
      }
    });
  // HTML's ASCII whitespace, which an attribute's value is read past.
  const whitespace = /[\t\n\f\r ]+/;
  // `aria-hidden` hides when it says `true`, past ASCII whitespace, in any
  // ASCII case, as `trap_links` reads it in markup.
  const hides = (value) => /^[\t\n\f\r ]*true[\t\n\f\r ]*$/i.test(value || "");
  // The ids another element names in `aria-controls`: what a button or a
  // tab shows and hides.
  const controlled = new Set();
  for (const control of document.querySelectorAll("[aria-controls]")) {
    for (const id of control.getAttribute("aria-controls").split(whitespace)) {
      if (id) controlled.add(id);
    }
  }
  // Whether `at` holds what it holds in a menu, a tab or a disclosure a
  // person opens to see.
  const opens = (at) =>
    opening.elements.includes(at.localName) ||
    (at.getAttribute("role") || "")
      .toLowerCase()
      .split(whitespace)
      .some((role) => opening.roles.includes(role)) ||
    at.hasAttribute("popover") ||
    (at.getAttribute("hidden") || "").toLowerCase() === "until-found" ||
    (at.id !== "" && controlled.has(at.id));
  const page = document.documentElement;
  const width = Math.max(page.scrollWidth, page.clientWidth);
  const height = Math.max(page.scrollHeight, page.clientHeight);
  // The overflow of `<html>`, and of `<body>` while `<html>`'s is visible,
  // goes to the viewport, which a person scrolls: it cuts nothing.
  const pageStyle = getComputedStyle(page);
  const toViewport = (at) =>
    at === page ||
    (at === document.body &&
      pageStyle.overflowX === "visible" &&
      pageStyle.overflowY === "visible");
  // Whether an element styled `its` is the containing block of a fixed
  // (`fixed`) or absolutely positioned descendant.
  const contains = (its, fixed) =>
    (!fixed && its.position !== "static") ||
    its.transform !== "none" ||
    its.translate !== "none" ||
    its.rotate !== "none" ||
    its.scale !== "none" ||
    its.perspective !== "none" ||
    its.filter !== "none" ||
    its.backdropFilter !== "none" ||
    its.containerType !== "normal" ||
    its.contentVisibility !== "visible" ||
    /\b(layout|paint|strict|content)\b/.test(its.contain) ||
    /\b(transform|perspective|filter)\b/.test(its.willChange);
  const seen = (element) => {
    const chain = holders(element);
    const style = getComputedStyle(element);
    // A `display: contents` element has no box of its own: what it holds
    // shows in its place.
    const contents = style.display === "contents";
    const range = contents ? document.createRange() : null;
    if (range) range.selectNodeContents(element);
    const measured = range || element;
    const rects = Array.from(measured.getClientRects()).filter(
      (rect) => rect.width > 0 && rect.height > 0,
    );
    const noBox = rects.length === 0;
    const visibilityHidden = style.visibility !== "visible";
    const transparent = chain.some((at) => Number.parseFloat(getComputedStyle(at).opacity) === 0);
    const ariaHidden = chain.some((at) => hides(at.getAttribute("aria-hidden")));
    // Clipped to nothing: a `clip` or `clip-path` on it or an element
    // holding it that leaves no area, or the cut overflow (`hidden` or
    // `clip`) of an element on its chain of containing blocks whose box
    // shares none of its own. A scrolling holder (`auto` or `scroll`) cuts
    // nothing, since a person scrolls what it holds into its box, and a
    // fixed or absolutely positioned element escapes every holder short of
    // its containing block.
    let clipped = false;
    // Its box, in viewport coordinates, brought into each scrolling
    // holder's box as a person would scroll it there.
    let [x0, y0, x1, y1] = [0, 0, 0, 0];
    if (!noBox) {
      const box = measured.getBoundingClientRect();
      [x0, y0, x1, y1] = [box.left, box.top, box.right, box.bottom];
      // The position of the element whose containing block is sought next.
      let position = style.position;
      for (const at of chain) {
        const its = at === element ? style : getComputedStyle(at);
        if (
          // oxlint-disable-next-line typescript/no-deprecated -- a trap link hides behind the obsolete `clip` too, so it must be read.
          its.clip === "rect(0px, 0px, 0px, 0px)" ||
          /^inset\((50|[5-9]\d|100)%\)$/.test(its.clipPath)
        ) {
          clipped = true;
          break;
        }
        if (at === element) continue;
        const onChain =
          position === "fixed"
            ? contains(its, true)
            : position === "absolute"
              ? contains(its, false)
              : true;
        if (!onChain) continue;
        position = its.position;
        if (toViewport(at)) continue;
        const holds = at.getBoundingClientRect();
        const along = (overflow, low, high, from, to) => {
          if (overflow === "visible") return [low, high];
          if (overflow === "hidden" || overflow === "clip") {
            return [Math.max(low, from), Math.min(high, to)];
          }
          if (low >= from && high <= to) return [low, high];
          return [from, from + Math.min(high - low, to - from)];
        };
        [x0, x1] = along(its.overflowX, x0, x1, holds.left, holds.right);
        [y0, y1] = along(its.overflowY, y0, y1, holds.top, holds.bottom);
        if (x1 - x0 <= 0 || y1 - y0 <= 0) {
          clipped = true;
          break;
        }
      }
    }
    // Page coordinates: the viewport's offset added.
    const outsidePage =
      !noBox &&
      !clipped &&
      (x1 + window.scrollX <= 0 ||
        y1 + window.scrollY <= 0 ||
        x0 + window.scrollX >= width ||
        y0 + window.scrollY >= height);
    const rel = (element.getAttribute("rel") || "").toLowerCase().split(whitespace);
    return {
      noBox,
      outsidePage,
      visibilityHidden,
      transparent,
      ariaHidden,
      clipped,
      nofollow: rel.includes("nofollow"),
      cosmetic: chain.some((at) => matchesAny(at, cosmetic)),
      revealable: chain.some(opens),
    };
  };
  return {
    url: document.URL,
    baseUrl: document.baseURI,
    encoding: document.characterSet,
    elements: Array.from(document.querySelectorAll(selector), (element) => ({
      href: element.getAttribute("href"),
      text: element.textContent || "",
      attributes: Array.from(element.attributes, (attribute) => [attribute.name, attribute.value]),
      seen: seen(element),
    })),
  };
}
