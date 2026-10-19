// Keeps a reference to the text node it shows, and later removes that node
// from the parent it put it in, as React does with the text it renders.
const details = document.getElementById("details");
const label = document.createTextNode("Details shown");
details.append(label);
document.getElementById("hide").addEventListener("click", () => {
  details.removeChild(label);
  document.getElementById("result").textContent = "hidden";
});
