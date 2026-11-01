// Which hiding rules hide a locator's target (ADRs 0047 and 0048), live
// hiding's or the Consent prompt runner's, called in the
// guard's isolated world when the guard finds the target not visible: each of
// `rules` that matches the target or an element holding it, through the
// shadow roots it sits in, and that element's computed `display` is `none`.
// None unless exactly one element matches `selector`, as the guard acts on.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function hiddenBy({ selector, rules }) {
  const found = [];
  const visit = (root) => {
    found.push(...root.querySelectorAll(selector));
    for (const element of root.querySelectorAll("*"))
      if (element.shadowRoot) visit(element.shadowRoot);
  };
  try {
    visit(document);
  } catch {
    return [];
  }
  if (found.length !== 1) return [];
  const hiding = [];
  for (const rule of rules) {
    let element = found[0];
    while (element) {
      let holder = null;
      try {
        holder = element.closest(rule);
      } catch {
        break;
      }
      if (holder !== null && getComputedStyle(holder).display === "none") {
        hiding.push(rule);
        break;
      }
      const root = element.getRootNode();
      element = root instanceof ShadowRoot ? root.host : null;
    }
  }
  return hiding;
}
