// The focused button leaves the page, so focus falls to the body.
document.getElementById("vanishing").addEventListener("focus", (event) => {
  event.target.remove();
});
