// A page function's call and its result's walk (ADR 0085), evaluated in the
// page's main world or in the page functions' isolated world with
// `Runtime.evaluate`, never through `Runtime.enable`. The engine sends it as
// `(walk)(() => (source), argument, given)`: the thunk keeps the program's
// source outside this function's scope, so it sees only the page's globals
// and its argument. A source that evaluates to a function is called with the
// argument, and any other value is an expression's, which takes none. The
// result, awaited, is copied as JSON: `undefined` is dropped from objects and
// `null` in arrays, `-0` is `0`, and the first value outside JSON stops the
// walk, answered by its path from `$` and its kind. CDP sends this one
// function, so its helpers live inside it.
/* eslint-disable unicorn/consistent-function-scoping -- CDP sends the function alone */
// eslint-disable-next-line no-unused-vars -- the engine's expression calls it
async function walk(thunk, argument, given) {
  let value;
  try {
    value = thunk();
    if (typeof value === "function") {
      value = await value(argument);
    } else if (given) {
      return { expression: true };
    } else {
      value = await value;
    }
  } catch (error) {
    const thrown = error instanceof Error || (error && typeof error.name === "string");
    return {
      thrown: {
        name: thrown ? String(error.name) : null,
        message: thrown ? String(error.message) : String(error),
        stack: thrown && typeof error.stack === "string" ? error.stack : null,
      },
    };
  }
  const tag = (found) => Object.prototype.toString.call(found).slice(8, -1);
  const constructorOf = (found) => {
    try {
      const name = Object.getPrototypeOf(found)?.constructor?.name;
      return typeof name === "string" && name !== "" ? name : tag(found);
    } catch {
      return tag(found);
    }
  };
  const isPlain = (found) => {
    const prototype = Object.getPrototypeOf(found);
    return prototype === null || Object.getPrototypeOf(prototype) === null;
  };
  const kindOf = (found) => {
    if (found === globalThis || tag(found) === "Window") {
      return "window";
    }
    if (typeof found.nodeType === "number" && typeof found.nodeName === "string") {
      return "node";
    }
    const kinds = { Map: "map", Set: "set", Date: "date", RegExp: "regexp", Error: "error" };
    return kinds[tag(found)] ?? "object";
  };
  const IDENTIFIER = /^[A-Za-z_$][\w$]*$/u;
  const step = (path, key) =>
    typeof key === "number"
      ? `${path}[${key}]`
      : IDENTIFIER.test(key)
        ? `${path}.${key}`
        : `${path}[${JSON.stringify(key)}]`;
  // The first value outside JSON, after which the walk copies nothing more.
  let invalid = null;
  const mark = (path, kind, found) => {
    invalid = {
      path,
      kind,
      constructor: typeof found === "object" && found !== null ? constructorOf(found) : null,
    };
    return undefined;
  };
  const ancestors = new Set();
  const copy = (found, path) => {
    if (invalid !== null) {
      return undefined;
    }
    switch (typeof found) {
      case "string":
      case "boolean": {
        return found;
      }
      case "number": {
        if (Number.isNaN(found)) {
          return mark(path, "not_a_number", found);
        }
        if (!Number.isFinite(found)) {
          return mark(path, "infinity", found);
        }
        return found === 0 ? 0 : found;
      }
      case "bigint": {
        return mark(path, "bigint", found);
      }
      case "symbol": {
        return mark(path, "symbol", found);
      }
      case "function": {
        return mark(path, "function", found);
      }
      case "undefined": {
        return undefined;
      }
      default: {
        break;
      }
    }
    if (found === null) {
      return null;
    }
    if (ancestors.has(found)) {
      return mark(path, "cycle", found);
    }
    if (!Array.isArray(found) && !isPlain(found)) {
      return mark(path, kindOf(found), found);
    }
    ancestors.add(found);
    let copied;
    if (Array.isArray(found)) {
      copied = Array.from({ length: found.length }, (_, index) => {
        const item = copy(found[index], step(path, index));
        return item === undefined ? null : item;
      });
    } else {
      const entries = [];
      for (const key of Object.keys(found)) {
        const item = copy(found[key], step(path, key));
        if (item !== undefined) {
          entries.push([key, item]);
        }
      }
      // `fromEntries` defines each key, so `__proto__` stays a key.
      copied = Object.fromEntries(entries);
    }
    ancestors.delete(found);
    return copied;
  };
  try {
    const copied = copy(value, "$");
    if (invalid !== null) {
      return { invalid };
    }
    return copied === undefined ? {} : { value: copied };
  } catch (error) {
    const thrown = error instanceof Error;
    return {
      thrown: {
        name: thrown ? error.name : null,
        message: thrown ? error.message : String(error),
        stack: thrown && typeof error.stack === "string" ? error.stack : null,
      },
    };
  }
}
