// Sends "ping" over a same-origin WebSocket and shows what comes back.
const socket = new WebSocket(`ws://${location.host}/echo`);
socket.addEventListener("open", () => socket.send("ping"));
socket.addEventListener("message", (event) => {
  document.getElementById("received").textContent = event.data;
});
