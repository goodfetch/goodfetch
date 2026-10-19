// Keeps no reference to a text node: it sets its element's text instead.
const details = document.getElementById("details");
details.textContent = "Details shown";
document.getElementById("hide").addEventListener("click", () => {
  details.textContent = "";
  document.getElementById("result").textContent = "hidden";
});
