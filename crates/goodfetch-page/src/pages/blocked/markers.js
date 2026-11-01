// What block detection reads of the main frame's document (ADR 0035),
// called in the guard's isolated world, which no page script reaches: which
// of `markers`, each lowercase, the document's serialization holds, read
// lowercased, so the markup itself never leaves the page.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function blockMarkers(argument) {
  const root = document.documentElement;
  const markup = root === null ? "" : root.outerHTML.toLowerCase();
  return argument.markers.filter((marker) => markup.includes(marker));
}
