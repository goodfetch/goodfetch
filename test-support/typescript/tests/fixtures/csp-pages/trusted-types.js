// Tries a string-to-HTML sink, which Trusted Types refuses under the page's CSP.
const result = document.getElementById("result");
try {
  result.innerHTML = "<b>accepted</b>";
} catch (error) {
  result.textContent = `refused: ${error.name}`;
}
