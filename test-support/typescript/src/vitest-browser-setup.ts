// The shared setup file of Vitest Browser Mode projects (ADR 0068): before
// each test, `hoverReset` moves the pointer off whatever the test before it
// hovered. The Playwright provider keeps the pointer between the tests of one
// file, and they run shuffled (ADR 0064). Browser state needs no reset here:
// `browserState` undoes itself when its test ends.
import { beforeEach } from "vitest";
import { hoverReset } from "./vitest-browser.ts";

beforeEach(hoverReset);
