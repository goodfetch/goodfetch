/// <reference types="node" />
// Golden scenario 03 on Stagehand 4.2.1. Its cache (`cache: true`) is kept on Browserbase's server
// and "has no effect on local browsers" (docs.stagehand.dev/v4/basics/observe), so the program keeps
// each resolved Action in its own file and replays it with `act(action)`, the documented "observe,
// then act" replay, which makes no model call (docs.stagehand.dev/v4/basics/act).
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { type Action, localBrowser, Stagehand } from "@browserbasehq/stagehand";

const FILE = `${import.meta.dirname}/quotes.actions.json`;
const kept: Record<string, Action> = existsSync(FILE) ? JSON.parse(readFileSync(FILE, "utf8")) : {};
const STEPS = [
  'open the tag "love" from the tag list',
  "go to its second page",
  "open the about page of the first quote's author",
];

const browser = await localBrowser.launch();
const stagehand = await Stagehand.create({
  browser,
  model: { modelName: "anthropic/claude-sonnet-4-5", apiKey: process.env.ANTHROPIC_API_KEY },
  selfHeal: true, // a kept Action whose selector no longer resolves is inferred again, once
});
const [page] = await browser.context.pages();
await page.goto("https://quotes.toscrape.com/");

for (const instruction of STEPS) {
  let action = kept[instruction];
  if (action === undefined) {
    const { data } = await stagehand.observe(instruction); // the one model call of a new step
    action = data[0];
    if (action === undefined) throw new Error(`Nothing on the page fits "${instruction}"`);
  }
  const { data } = await stagehand.act(action); // a failure is a value, not an exception
  if (!data.success) throw new Error(`"${instruction}" failed: ${data.message}`);
  const performed = data.actions[0] ?? action;
  if (kept[instruction] !== undefined && performed.selector !== action.selector) {
    console.warn(`The site changed: "${instruction}" went back to the model.`);
  }
  kept[instruction] = performed;
}

const url = await page.url();
if (!url.startsWith("https://quotes.toscrape.com/author/"))
  throw new Error(`Not an author page: ${url}`);
writeFileSync(FILE, JSON.stringify(kept, null, 2)); // kept only once the check holds

// `extract` has no local replay, so the fields are read with selectors: the second run makes no model call.
const author = {
  name: await page.locator(".author-title").textContent(),
  birthDate: await page.locator(".author-born-date").textContent(),
  birthplace: await page.locator(".author-born-location").textContent(),
};
console.log(author);

await stagehand.close();
await browser.close();
