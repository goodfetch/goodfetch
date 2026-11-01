// A page function's call and its result's walk (ADR 0085), evaluated in the
// page's main world or in the page functions' isolated world with
// `Runtime.evaluate`, never through `Runtime.enable`. The engine sends it as
// `(walk)(() => (source), argument, given)`: the thunk keeps the program's
// source outside this function's scope, so it sees only the page's globals
// and its argument. A source that evaluates to a function is called with the
// argument, and any other value is an expression's, which takes none. The
// result, awaited, is copied as JSON: `undefined` is dropped from objects and
// `null` in arrays, `-0` is `0`, and the first value outside JSON stops the
// walk, answered by its path from `$` and its kind. A locator's page function
// also gets `targets` (`targets.js`), which answers its matches, handed to the
// function before the argument, or why there is none to hand (`unparsed`,
// `missing`), answered as it is. CDP sends this one function, so its helpers
// live inside it.
/* eslint-disable unicorn/consistent-function-scoping -- CDP sends the function alone */
// eslint-disable-next-line no-unused-vars -- the engine's expression calls it
async function walk(thunk, argument, given, targets) {
  // What a thrown value says: an error of any realm by its name, message
  // and stack, any other value as its text.
  const thrownOf = (error) => {
    const named = error instanceof Error || (error && typeof error.name === "string");
    return {
      thrown: {
        name: named ? String(error.name) : null,
        message: named ? String(error.message) : String(error),
        stack: named && typeof error.stack === "string" ? error.stack : null,
      },
    };
  };
  let target;
  if (targets !== undefined) {
    const found = targets();
    if (!("target" in found)) {
      return found;
    }
    target = found.target;
  }
  let value;
  try {
    value = thunk();
    if (typeof value === "function") {
      value = await (targets === undefined ? value(argument) : value(target, argument));
    } else if (given) {
      return { expression: true };
    } else {
      value = await value;
    }
  } catch (error) {
    return thrownOf(error);
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
  // The walk copies with syntax, never with a built-in the page may have
  // replaced, such as Prototype.js's `Array.from`; a value's path from `$` is
  // spelled only for the value that stops the walk.
  const IDENTIFIER = /^[A-Za-z_$][\w$]*$/u;
  const pathOf = (at) => {
    if (at === null) {
      return "$";
    }
    const parent = pathOf(at.parent);
    return typeof at.key === "number"
      ? `${parent}[${at.key}]`
      : IDENTIFIER.test(at.key)
        ? `${parent}.${at.key}`
        : `${parent}[${JSON.stringify(at.key)}]`;
  };
  // The first value outside JSON, after which the walk copies nothing more.
  let invalid = null;
  const mark = (at, kind, found) => {
    invalid = {
      path: pathOf(at),
      kind,
      constructor: typeof found === "object" && found !== null ? constructorOf(found) : null,
    };
    return undefined;
  };
  // The objects the walk is inside, the outermost first.
  const ancestors = [];
  let depth = 0;
  const inside = (found) => {
    for (let index = 0; index < depth; index += 1) {
      if (ancestors[index] === found) {
        return true;
      }
    }
    return false;
  };
  const copy = (found, at) => {
    if (invalid !== null) {
      return undefined;
    }
    switch (typeof found) {
      case "string":
      case "boolean": {
        return found;
      }
      case "number": {
        // `Number.isNaN` may be the page's: only `NaN` is not itself.
        if (found !== found) {
          return mark(at, "not_a_number", found);
        }
        if (found === Infinity || found === -Infinity) {
          return mark(at, "infinity", found);
        }
        return found === 0 ? 0 : found;
      }
      case "bigint": {
        return mark(at, "bigint", found);
      }
      case "symbol": {
        return mark(at, "symbol", found);
      }
      case "function": {
        return mark(at, "function", found);
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
    if (inside(found)) {
      return mark(at, "cycle", found);
    }
    const array = Array.isArray(found);
    if (!array && !isPlain(found)) {
      return mark(at, kindOf(found), found);
    }
    ancestors[depth] = found;
    depth += 1;
    let copied;
    if (array) {
      copied = [];
      const length = found.length;
      for (let index = 0; index < length; index += 1) {
        const item = copy(found[index], { parent: at, key: index });
        copied[index] = item === undefined ? null : item;
      }
    } else {
      // No prototype, so a `__proto__` key stays a key.
      copied = { __proto__: null };
      const keys = Object.keys(found);
      for (let index = 0; index < keys.length; index += 1) {
        const key = keys[index];
        const item = copy(found[key], { parent: at, key });
        if (item !== undefined) {
          copied[key] = item;
        }
      }
    }
    depth -= 1;
    return copied;
  };
  try {
    const copied = copy(value, null);
    if (invalid !== null) {
      return { invalid };
    }
    return copied === undefined ? {} : { value: copied };
  } catch (error) {
    return thrownOf(error);
  }
}
