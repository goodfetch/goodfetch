// The canvas pages' script. On `/button` it draws, as pixels only, which
// button to press, the button and a decoy, and reports each click on the
// canvas: on the button, on the decoy, or elsewhere (a miss). On `/surface`
// it draws each stroke the pointer makes and reports where each began and
// ended, in grid cells. Every report carries the browser's `isTrusted`, and
// the root element's `data-posting` counts the reports in flight. It is the
// page's only script, so its names stay its own.
"use strict";

let posting = 0;

const post = (effect) => {
  const root = document.documentElement;
  posting += 1;
  root.dataset.posting = String(posting);
  fetch("/effects", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(effect),
  })
    .catch(() => {})
    .finally(() => {
      posting -= 1;
      root.dataset.posting = String(posting);
    });
};

const rectangle = (value) => {
  const [left, top, width, height] = value.split(" ").map(Number);
  return { left, top, width, height };
};

const label = (value) => String.fromCodePoint(...value.split(" ").map(Number));

const inside = (place, x, y) =>
  x >= place.left && x < place.left + place.width && y >= place.top && y < place.top + place.height;

const drawButtons = (canvas, data) => {
  const context = canvas.getContext("2d");
  const button = rectangle(data.button);
  const decoy = rectangle(data.decoy);
  const buttonLabel = label(data.buttonLabel);
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.font = "14px sans-serif";
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillStyle = "#111827";
  context.fillText(`Press ${buttonLabel}`, canvas.width / 2, 12);
  for (const [place, text, colour] of [
    [button, buttonLabel, "#1d4ed8"],
    [decoy, label(data.decoyLabel), "#4b5563"],
  ]) {
    context.fillStyle = colour;
    context.fillRect(place.left, place.top, place.width, place.height);
    context.fillStyle = "#ffffff";
    context.fillText(text, place.left + place.width / 2, place.top + place.height / 2);
  }
  let presses = 0;
  canvas.addEventListener("click", (event) => {
    const [x, y] = [event.offsetX, event.offsetY];
    let target = "canvas";
    if (inside(button, x, y)) {
      target = "button";
      presses += 1;
      document.getElementById("presses").textContent = String(presses);
    } else if (inside(decoy, x, y)) {
      target = "decoy";
    }
    post({ event: "click", target, trusted: event.isTrusted });
  });
};

const dot = (context, x, y) => {
  context.beginPath();
  context.arc(x, y, 4, 0, 2 * Math.PI);
  context.fill();
};

const drawSurface = (surface, data) => {
  const context = surface.getContext("2d");
  const cell = Number(data.cell);
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, surface.width, surface.height);
  context.strokeStyle = data.ink;
  context.fillStyle = data.ink;
  context.lineWidth = 8;
  context.lineCap = "round";
  context.lineJoin = "round";
  const at = (x, y) => `${Math.floor(x / cell)},${Math.floor(y / cell)}`;
  let start = null;
  let strokes = 0;
  surface.addEventListener("pointerdown", (event) => {
    start = { x: event.offsetX, y: event.offsetY, trusted: event.isTrusted };
    dot(context, start.x, start.y);
    context.beginPath();
    context.moveTo(start.x, start.y);
  });
  surface.addEventListener("pointermove", (event) => {
    if (start !== null) {
      context.lineTo(event.offsetX, event.offsetY);
      context.stroke();
    }
  });
  surface.addEventListener("pointerup", (event) => {
    if (start === null) {
      return;
    }
    const [x, y] = [event.offsetX, event.offsetY];
    context.lineTo(x, y);
    context.stroke();
    dot(context, x, y);
    strokes += 1;
    document.getElementById("strokes").textContent = String(strokes);
    post({
      event: "stroke",
      target: "surface",
      trusted: start.trusted && event.isTrusted,
      details: { from: at(start.x, start.y), to: at(x, y) },
    });
    start = null;
  });
};

document.documentElement.dataset.posting = "0";
const scene = document.currentScript.dataset;
const canvas = document.getElementById("canvas");
if (canvas !== null) {
  drawButtons(canvas, scene);
}
const surface = document.getElementById("surface");
if (surface !== null) {
  drawSurface(surface, scene);
}
