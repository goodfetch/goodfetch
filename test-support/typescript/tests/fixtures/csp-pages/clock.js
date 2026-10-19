// Shows the page's time, and the time a one-minute timer fires.
document.getElementById("now").textContent = new Date().toISOString();
setTimeout(() => {
  document.getElementById("timer").textContent = `fired at ${new Date().toISOString()}`;
}, 60_000);
