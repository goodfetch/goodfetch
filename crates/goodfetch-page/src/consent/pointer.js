// Whether the pointer reaches a click's target in its frame (ADR 0048), called
// in the guard's isolated world of a frame other than the page's own before
// the Consent prompt runner's click there. The browser sends a frame in
// another process its input by where the page last drew that frame, so a
// change of the page's own sheets, such as the lifting of its hiding for the
// click, reaches the input only once the page drew again: until then, a press
// may go to the page instead. The first call watches the pointer's moves for
// the element the runner handed the guard by `handle`; each later one waits
// at most `wait` milliseconds for a move the engine sent to reach it.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
async function pointer({ handle, wait }) {
  const world = globalThis;
  const KEY = Symbol.for("goodfetch.consent.pointer");
  const element = world[Symbol.for("goodfetch.guard.targets")]?.get(handle);
  if (!(element instanceof Element) || !element.isConnected) return "gone";
  const watch = world[KEY];
  if (watch?.element !== element) {
    watch?.stop();
    const state = { element, over: false };
    const listener = (event) => {
      if (event.composedPath().includes(element)) state.over = true;
    };
    window.addEventListener("pointermove", listener, true);
    // A watch whose click was dropped never outlives the click's window.
    const expiry = setTimeout(() => state.stop(), 10000);
    state.stop = () => {
      clearTimeout(expiry);
      window.removeEventListener("pointermove", listener, true);
      if (world[KEY] === state) delete world[KEY];
    };
    world[KEY] = state;
    return "watching";
  }
  const end = performance.now() + wait;
  while (!watch.over && performance.now() < end)
    await new Promise((resolve) => setTimeout(resolve, 10));
  if (!watch.over) return "elsewhere";
  watch.stop();
  return "over";
}
