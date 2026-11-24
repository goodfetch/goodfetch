// The font page's measuring script (ADR 0074): once the document's fonts
// are ready, a font is available when its text, set in it then in each
// generic family in turn, measures apart from the generic family's alone.
// It shows the fallback's size and each font's result as JSON in
// `#results` and sets `data-fonts="done"` on the root element.
"use strict";

/** A text's width and height as laid out. */
const size = (span) => {
  const box = span.getBoundingClientRect();
  return { width: box.width, height: box.height };
};

const measure = async () => {
  await document.fonts.ready;
  const fallback = [...document.querySelectorAll("#fallback span")].map(size);
  const fonts = {};
  for (const font of document.querySelectorAll(".font")) {
    const apart = [...font.querySelectorAll("span")].some((span, index) => {
      const { width, height } = size(span);
      return width !== fallback[index].width || height !== fallback[index].height;
    });
    fonts[font.dataset.family] = {
      listed: font.dataset.listed === "true",
      expected: font.dataset.expected,
      observed: apart ? "available" : "unavailable",
    };
  }
  document.getElementById("results").textContent = JSON.stringify({
    fallback: fallback[0],
    fonts,
  });
  document.documentElement.dataset.fonts = "done";
};

void measure();
