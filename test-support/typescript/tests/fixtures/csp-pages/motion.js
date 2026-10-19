// Starts the spinner when the button is pressed.
const spinner = document.getElementById("spinner");
const announcement = document.getElementById("status");
document.getElementById("load").addEventListener("click", () => {
  spinner.classList.add("busy");
  announcement.textContent = "Loading runs";
});
