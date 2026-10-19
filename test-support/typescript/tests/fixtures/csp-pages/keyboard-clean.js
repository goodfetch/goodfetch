// A menu and a dialog that each give focus back to their trigger on Escape.
const actions = document.getElementById("actions");
const menu = document.getElementById("actions-menu");
actions.addEventListener("click", () => {
  menu.hidden = false;
  actions.setAttribute("aria-expanded", "true");
  menu.querySelector('[role="menuitem"]').focus();
});
menu.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    menu.hidden = true;
    actions.setAttribute("aria-expanded", "false");
    actions.focus();
  }
});

const dialog = document.getElementById("settings-dialog");
document.getElementById("settings").addEventListener("click", () => dialog.showModal());
document.getElementById("settings-close").addEventListener("click", () => dialog.close());
