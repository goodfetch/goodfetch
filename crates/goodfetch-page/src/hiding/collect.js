// Live hiding's read of a frame (ADR 0047), called in the guard's isolated
// world, which no page script reaches: the document's URL and every class and
// ID its elements hold, by which the engine picks the cosmetic rules that may
// match it, as a blocker's content script does.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function collect() {
  const classes = new Set();
  const ids = new Set();
  for (const element of document.querySelectorAll("[class], [id]")) {
    for (const name of element.classList) classes.add(name);
    if (element.id) ids.add(element.id);
  }
  return { url: location.href, classes: [...classes], ids: [...ids] };
}
