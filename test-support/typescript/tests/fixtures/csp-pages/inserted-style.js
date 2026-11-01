// Inserts an inline `<style>` element when the button is pressed, as a page's
// own code would: the production CSP refuses it, from this script.
document.getElementById("insert").addEventListener("click", () => {
  const style = document.createElement("style");
  style.textContent = "body {}";
  document.head.append(style);
});
