// What a person sees of each element `selector` matches (ADR 0035; the
// Frontier's `trap_links`), called in the guard's isolated world of Collie's
// page: the document's URL, base URL and encoding, and for each element, in
// document order, its `href`, text and attributes, and the facts `judge`
// decides from. `cosmetic` holds the selectors of the cosmetic rules live
// hiding applies to the page (ADR 0047): an element one of them matches, or
// one holding it, is reported `cosmetic`. A selector the page refuses throws.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function trapLinks({ selector, cosmetic }) {
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
  const page = document.documentElement;
  const width = Math.max(page.scrollWidth, page.clientWidth);
  const height = Math.max(page.scrollHeight, page.clientHeight);
  const seen = (element) => {
    const chain = holders(element);
    const rects = Array.from(element.getClientRects()).filter(
      (rect) => rect.width > 0 && rect.height > 0,
    );
    const noBox = rects.length === 0;
    // Page coordinates: the viewport's offset added.
    const left = (rect) => rect.left + window.scrollX;
    const top = (rect) => rect.top + window.scrollY;
    const outsidePage =
      !noBox &&
      rects.every(
        (rect) =>
          left(rect) + rect.width <= 0 ||
          top(rect) + rect.height <= 0 ||
          left(rect) >= width ||
          top(rect) >= height,
      );
    const style = getComputedStyle(element);
    const visibilityHidden = style.visibility !== "visible";
    const transparent = chain.some((at) => Number.parseFloat(getComputedStyle(at).opacity) === 0);
    const ariaHidden = chain.some((at) => at.getAttribute("aria-hidden") === "true");
    // Clipped to nothing: a `clip` or `clip-path` that leaves no area, or an
    // element holding it whose overflow is cut and whose box shares none of
    // the element's.
    let clipped = false;
    if (!noBox) {
      let box = element.getBoundingClientRect();
      let [x0, y0, x1, y1] = [box.left, box.top, box.right, box.bottom];
      for (const at of chain) {
        const its = getComputedStyle(at);
        if (
          its.clip === "rect(0px, 0px, 0px, 0px)" ||
          /^inset\((50|[5-9]\d|100)%\)$/.test(its.clipPath)
        ) {
          clipped = true;
          break;
        }
        if (at !== element && its.overflow !== "visible") {
          box = at.getBoundingClientRect();
          x0 = Math.max(x0, box.left);
          y0 = Math.max(y0, box.top);
          x1 = Math.min(x1, box.right);
          y1 = Math.min(y1, box.bottom);
          if (x1 - x0 <= 0 || y1 - y0 <= 0) {
            clipped = true;
            break;
          }
        }
      }
    }
    const rel = (element.getAttribute("rel") || "").toLowerCase().split(/\s+/);
    return {
      noBox,
      outsidePage,
      visibilityHidden,
      transparent,
      ariaHidden,
      clipped,
      nofollow: rel.includes("nofollow"),
      cosmetic: chain.some((at) => matchesAny(at, cosmetic)),
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
