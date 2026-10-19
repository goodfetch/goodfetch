// Escape hides the dialog without giving focus back to its trigger.
const panel = document.getElementById("panel");
document.getElementById("open").addEventListener("click", () => {
  panel.hidden = false;
  document.getElementById("close").focus();
});
panel.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    panel.hidden = true;
  }
});
