// Tab on the second button goes back to the first, outside any overlay.
const first = document.getElementById("first");
const second = document.getElementById("second");
second.addEventListener("keydown", (event) => {
  if (event.key === "Tab" && !event.shiftKey) {
    event.preventDefault();
    first.focus();
  }
});
first.addEventListener("keydown", (event) => {
  if (event.key === "Tab" && event.shiftKey) {
    event.preventDefault();
    second.focus();
  }
});
