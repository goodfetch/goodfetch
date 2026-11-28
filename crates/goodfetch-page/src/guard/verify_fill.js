// Re-checks the focused editable field after human-like typing (ADR 0035;
// spec #599): returns `true` if its value already equals `expected`, else
// selects all contents and returns `false` so the guard restores the exact
// requested value.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function verifyFill(expected) {
  let element = document.activeElement;
  while (element && element.shadowRoot && element.shadowRoot.activeElement) {
    element = element.shadowRoot.activeElement;
  }
  if (!element) return true;
  if (element.isContentEditable) {
    if (element.textContent === expected) return true;
    const range = document.createRange();
    range.selectNodeContents(element);
    const selection = getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    return false;
  }
  if (!("value" in element)) return true;
  if (element.value === expected) return true;
  if (typeof element.select === "function") element.select();
  return false;
}
