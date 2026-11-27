// The pointer and input recorder probe page (ADR 0035; spec #599, #1018):
// records every mouse move, click, wheel event and key press with its
// coordinates, timing and `isTrusted`, so a test can tell a human-like curved
// path, wheel scroll and key-by-key typing from direct synthetic input.
const recorded = { moves: [], clicks: [], wheels: [], keys: [] };
window.pointerRecorder = recorded;
const summary = document.querySelector("#summary");
const publish = () => {
  if (summary) summary.textContent = JSON.stringify(recorded);
};
window.addEventListener("mousemove", (event) => {
  recorded.moves.push({
    x: event.clientX,
    y: event.clientY,
    t: Math.round(event.timeStamp),
    trusted: event.isTrusted,
  });
  publish();
});
window.addEventListener(
  "wheel",
  (event) => {
    recorded.wheels.push({
      dx: event.deltaX,
      dy: event.deltaY,
      trusted: event.isTrusted,
    });
    publish();
  },
  { passive: true },
);
for (const id of ["target", "far"]) {
  document.querySelector(`#${id}`)?.addEventListener("click", (event) => {
    recorded.clicks.push({
      id,
      x: event.clientX,
      y: event.clientY,
      trusted: event.isTrusted,
    });
    publish();
  });
}
document.querySelector("#field")?.addEventListener("keydown", (event) => {
  recorded.keys.push({
    key: event.key,
    trusted: event.isTrusted,
  });
  publish();
});
publish();
