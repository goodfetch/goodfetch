import { chromium } from "playwright";

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();
await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
