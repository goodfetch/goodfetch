// Prints the title of the page its argument names, the Proving ground's
// when it has none: `npm start -- https://example.com/`.
import { Collie } from "@goodfetch/collie";
import { readTitle } from "./title.ts";

const url = process.argv[2] ?? "https://proving-ground.goodfetch.dev/";
await using collie = await Collie.connect();
console.log(await readTitle(collie, url));
