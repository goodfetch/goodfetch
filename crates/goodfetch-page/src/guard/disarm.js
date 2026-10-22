// The guard's disarm (ADR 0019), called in the guard's isolated world once a
// gesture's input was sent: removes the interception listener and answers
// whether the gesture landed on its target, was cancelled on another element,
// or reached nothing. No arm left means the gesture's document went away with
// it, so nothing was cancelled.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function disarm() {
  const arm = globalThis[Symbol.for("goodfetch.guard.arm")];
  if (arm === undefined) return { state: "landed" };
  arm.disarm();
  if (arm.state.intercepted !== null)
    return { state: "intercepted", observed: arm.state.intercepted };
  return arm.state.reached
    ? { state: "landed" }
    : { state: "intercepted", observed: "the gesture reached no element" };
}
