// The guard's fill of an input whose value a person picks rather than types
// (a date, a time, a month, a week, a local date and time, a colour or a
// range), called in the guard's isolated world once the check made it ready:
// sets its value and fires `input` and `change`, as Playwright's fill does,
// and answers whether the input took the value.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function setValue(value) {
  const element = globalThis[Symbol.for("goodfetch.guard.fill")];
  delete globalThis[Symbol.for("goodfetch.guard.fill")];
  if (!element?.isConnected) return { set: false, gone: true };
  const trimmed = value.trim();
  element.focus();
  element.value = trimmed;
  if (element.value !== trimmed) return { set: false, gone: false };
  element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
  return { set: true, gone: false };
}
