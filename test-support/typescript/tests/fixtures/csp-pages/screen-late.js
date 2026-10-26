// Shows the screen's second button two seconds after it opens, as a screen
// does once the engine answered what it reads.
setTimeout(() => {
  const list = document.getElementById("list");
  const late = document.createElement("button");
  late.type = "button";
  late.textContent = "Late";
  list.replaceWith(late);
}, 2000);
